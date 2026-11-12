//! Discord Rich Presence over Discord's IPC protocol (no GameSDK, no secrets).
//!
//! The public RPC client id (1568757859649323199) is used. Discord may not be
//! running on the dev machine, in which case connect/set_activity fail
//! gracefully and are logged — the app still builds, launches and attempts a
//! presence update using domain-specific state.

use serde_json::{json, Value};

#[cfg(unix)]
mod imp {
    use super::{json, Value};
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::process;
    use std::time::Duration;

    fn socket_paths() -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            // Discord uses `discord-ipc-0`..`discord-ipc-9` (hyphens); some
            // forks (vesktop) may use `discord_ipc_0` (underscores). Try both.
            for i in 0..10usize {
                paths.push(std::path::PathBuf::from(dir.clone()).join(format!("discord-ipc-{i}")));
                paths.push(std::path::PathBuf::from(dir.clone()).join(format!("discord_ipc_{i}")));
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            for i in 0..10usize {
                paths.push(
                    std::path::Path::new(&home)
                        .join(".config/discord")
                        .join(format!("discord-ipc-{i}")),
                );
                paths.push(
                    std::path::Path::new(&home)
                        .join(".config/discord")
                        .join(format!("discord_ipc_{i}")),
                );
            }
        }
        paths
    }

    fn connect_stream() -> Result<UnixStream, String> {
        let mut last_err = String::new();
        for path in socket_paths() {
            match UnixStream::connect(&path) {
                Ok(s) => return Ok(s),
                Err(e) => last_err = e.to_string(),
            }
        }
        Err(format!("no discord ipc socket found: {last_err}"))
    }

    fn send_frame(stream: &mut UnixStream, opcode: u32, payload: &Value) -> Result<(), String> {
        let buf = serde_json::to_vec(payload).map_err(|e| e.to_string())?;
        let len = buf.len() as u32;
        let mut out = Vec::with_capacity(8 + buf.len());
        // vesktop (and Discord IPC on this socket) frames as [opcode:4 LE][length:4 LE][payload].
        out.extend_from_slice(&opcode.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&buf);
        stream.write_all(&out).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_frame(stream: &mut UnixStream) -> Result<Value, String> {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut hdr = [0u8; 8];
        stream.read_exact(&mut hdr).map_err(|e| e.to_string())?;
        // [opcode:4 LE][length:4 LE]
        let _opcode = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
        let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
        let mut payload = vec![0u8; len];
        stream.read_exact(&mut payload).map_err(|e| e.to_string())?;
        serde_json::from_slice(&payload).map_err(|e| e.to_string())
    }

    pub(super) struct RpcClientImpl {
        stream: UnixStream,
        client_id: String,
        connected: bool,
    }

    impl RpcClientImpl {
        pub(super) fn connect(client_id: u64) -> Result<Self, String> {
            let mut stream = connect_stream()?;
            let client_id = client_id.to_string();
            // Handshake.
            send_frame(&mut stream, 0, &json!({ "v": 1, "client_id": &client_id }))?;
            let resp = read_frame(&mut stream)?;
            let ready = resp.get("cmd").and_then(|v| v.as_str()) == Some("DISPATCH")
                && resp.get("evt").and_then(|v| v.as_str()) == Some("READY");
            Ok(Self {
                stream,
                client_id,
                connected: ready,
            })
        }

        pub(super) fn set_activity(&mut self, activity: &Value) -> Result<(), String> {
            if !self.connected {
                return Err("rpc not ready".to_string());
            }
            let payload = json!({
                "cmd": "SET_ACTIVITY",
                "args": { "activity": activity, "pid": process::id() },
                "nonce": format!("anynfs-{}", process::id()),
            });
            send_frame(&mut self.stream, 0, &payload)?;
            let _ = read_frame(&mut self.stream);
            Ok(())
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use super::Value;
    pub(super) struct RpcClientImpl;
    impl RpcClientImpl {
        pub(super) fn connect(_client_id: u64) -> Result<Self, String> {
            Err("rpc only implemented on unix".to_string())
        }
        pub(super) fn set_activity(&self, _activity: &Value) -> Result<(), String> {
            Err("not unix".to_string())
        }
    }
}

pub struct RpcClient {
    inner: Option<imp::RpcClientImpl>,
}

impl RpcClient {
    pub fn connect(client_id: u64) -> Result<Self, String> {
        match imp::RpcClientImpl::connect(client_id) {
            Ok(inner) => Ok(Self { inner: Some(inner) }),
            Err(e) => {
                // Best-effort: Discord is often not running on a dev machine.
                log::warn!("discord rpc handshake failed (Discord may not be running): {e}");
                Ok(Self { inner: None })
            }
        }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.is_some()
    }

    pub fn set_activity(&mut self, activity: &Value) -> Result<(), String> {
        match &mut self.inner {
            Some(inner) => inner.set_activity(activity),
            None => Err("rpc not connected".to_string()),
        }
    }
}

/// Build the Discord SET_ACTIVITY activity payload from a domain-specific activity.
pub fn activity_value(domain: &str, activity: &super::RpcActivity) -> Value {
    json!({
        "details": activity.details,
        "state": activity.state,
        "timestamps": activity
            .start_timestamp
            .map(|start| json!({ "start": start })),
        "assets": {
            "large_image": activity.large_image_key.as_deref().unwrap_or("logo"),
            "large_text": activity
                .large_image_text
                .as_deref()
                .unwrap_or(&format!("anyNFS.xyz — {domain}")),
            "small_image": activity.small_image_key.as_deref().unwrap_or("logo"),
            "small_text": activity
                .small_image_text
                .as_deref()
                .unwrap_or(domain),
        },
        "instance": activity.instance,
    })
}

/// Domain-specific initial presence used when a domain is selected (before the
/// in-page bridge has polled any game state). Blends the live theme with the
/// RPC manifest fetched from GitHub raw.
pub fn initial_activity_value(domain: &str, _theme: &super::Theme, manifest: &super::RpcManifest) -> Value {
    json!({
        "details": manifest.details,
        "state": manifest.state,
        "assets": {
            "large_image": manifest.large_image_key.as_deref().unwrap_or("logo"),
            "large_text": manifest.large_image_text.as_deref().unwrap_or(&format!("anyNFS.xyz — {domain}")),
            "small_image": manifest.small_image_key.as_deref().unwrap_or("logo"),
            "small_text": manifest.small_image_text.as_deref().unwrap_or(domain),
        },
        "instance": false,
    })
}

#[cfg(test)]
mod tests {
    use crate::rpc::activity_value;
    use crate::config::RpcManifest;
    use crate::rpc::initial_activity_value;
    use crate::scraping::Theme;
    use crate::RpcActivity;

    fn sample_activity() -> RpcActivity {
        RpcActivity {
            details: "In Game".to_string(),
            state: "Cruising Bayview".to_string(),
            start_timestamp: Some(1_000_000),
            large_image_key: Some("logo".to_string()),
            large_image_text: Some("anyNFS.xyz — test".to_string()),
            small_image_key: Some("logo".to_string()),
            small_image_text: Some("test".to_string()),
            instance: false,
        }
    }

    fn sample_manifest(domain: &str) -> RpcManifest {
        RpcManifest {
            domain: domain.to_string(),
            details: "Main Menu".to_string(),
            state: format!("Idle — {domain}"),
            large_image_key: Some("logo".to_string()),
            large_image_text: Some(format!("anyNFS.xyz — {domain}")),
            small_image_key: Some("logo".to_string()),
            small_image_text: Some(domain.to_string()),
        }
    }

    #[test]
    fn activity_value_is_domain_specific() {
        // activity with no text overrides -> domain defaults should appear.
        let act = RpcActivity {
            details: "In Game".to_string(),
            state: "Cruising Bayview".to_string(),
            start_timestamp: Some(1_000_000),
            large_image_key: Some("logo".to_string()),
            large_image_text: None,
            small_image_key: Some("logo".to_string()),
            small_image_text: None,
            instance: false,
        };
        let v_nfsu2mod = activity_value("nfsu2mod.xyz", &act);
        let v_nfscrb = activity_value("nfscrb.xyz", &act);
        assert_eq!(
            v_nfsu2mod["assets"]["large_text"].as_str().unwrap(),
            "anyNFS.xyz — nfsu2mod.xyz"
        );
        assert_eq!(
            v_nfscrb["assets"]["large_text"].as_str().unwrap(),
            "anyNFS.xyz — nfscrb.xyz"
        );
        assert_eq!(
            v_nfsu2mod["assets"]["small_text"].as_str().unwrap(),
            "nfsu2mod.xyz"
        );
        assert_eq!(
            v_nfscrb["assets"]["small_text"].as_str().unwrap(),
            "nfscrb.xyz"
        );
        assert_eq!(v_nfsu2mod["timestamps"]["start"].as_i64(), Some(1_000_000));
        assert_eq!(v_nfsu2mod["state"].as_str(), Some("Cruising Bayview"));
        assert_eq!(v_nfsu2mod["instance"].as_bool(), Some(false));
    }

    #[test]
    fn initial_activity_value_is_domain_specific() {
        let theme = Theme { bg: "#0b0c10".to_string(), accent: "#38bdf8".to_string(), text: "#c8d6e5".to_string() };
        let m1 = sample_manifest("nfsu2mod.xyz");
        let m2 = sample_manifest("nfscrb.xyz");
        let v1 = initial_activity_value("nfsu2mod.xyz", &theme, &m1);
        let v2 = initial_activity_value("nfscrb.xyz", &theme, &m2);
        assert_eq!(v1["state"].as_str(), Some("Idle — nfsu2mod.xyz"));
        assert_eq!(v2["state"].as_str(), Some("Idle — nfscrb.xyz"));
        assert_eq!(v1["details"].as_str(), Some("Main Menu"));
        assert_eq!(v2["details"].as_str(), Some("Main Menu"));
        assert_eq!(v1["assets"]["small_text"].as_str(), Some("nfsu2mod.xyz"));
        assert_eq!(v2["assets"]["small_text"].as_str(), Some("nfscrb.xyz"));
        assert_eq!(v1["instance"].as_bool(), Some(false));
        assert_eq!(v2["instance"].as_bool(), Some(false));
    }
}
