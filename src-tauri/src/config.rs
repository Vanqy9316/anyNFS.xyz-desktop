//! Per-domain config schema and Discord Rich Presence manifest, fetched from
//! raw GitHub URLs with embedded fallbacks.
//!
//! Files are expected at (published to the remote):
//!   https://raw.githubusercontent.com/Vanqy9316/anyNFS.xyz-desktop/main/src-tauri/config/<domain>.json
//!   https://raw.githubusercontent.com/Vanqy9316/anyNFS.xyz-desktop/main/src-tauri/rpc/<domain>.json
//!
//! When the remote files are not yet published (or the machine is offline), the
//! embedded defaults below are used so the app still works. The fetch attempt is
//! always exercised and logged.

use serde::Deserialize;
use std::time::Duration;

use log::warn;
use reqwest::blocking::Client;

const GITHUB_RAW: &str = "https://raw.githubusercontent.com/Vanqy9316/anyNFS.xyz-desktop/main";

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub struct DomainConfig {
    pub domain: String,
    pub name: String,
    #[serde(default)]
    pub bg: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub icon_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RpcManifest {
    #[allow(dead_code)]
    pub domain: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub large_image_key: Option<String>,
    #[serde(default)]
    pub large_image_text: Option<String>,
    #[serde(default)]
    pub small_image_key: Option<String>,
    #[serde(default)]
    pub small_image_text: Option<String>,
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(8))
        .user_agent("anynfs.xyz-desktop/0.1.0")
        .build()
        .map_err(|e| e.to_string())
}

fn get_json<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
    let client = http_client()?;
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.json::<T>().map_err(|e| e.to_string())
}

pub fn fetch_domain_config(domain: &str) -> DomainConfig {
    let defaults = default_config(domain);
    let url = format!("{GITHUB_RAW}/src-tauri/config/{domain}.json");
    match get_json::<DomainConfig>(&url) {
        Ok(cfg) => cfg,
        Err(e) => {
            warn!("config schema fetch for {domain} from {url} failed (using embedded default): {e}");
            defaults
        }
    }
}

pub fn fetch_rpc_manifest(domain: &str) -> RpcManifest {
    let defaults = default_rpc(domain);
    let url = format!("{GITHUB_RAW}/src-tauri/rpc/{domain}.json");
    match get_json::<RpcManifest>(&url) {
        Ok(m) => m,
        Err(e) => {
            warn!("discord rpc manifest fetch for {domain} from {url} failed (using embedded default): {e}");
            defaults
        }
    }
}

pub fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let client = http_client().ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.bytes().ok().map(|b| b.to_vec())
}

fn default_config(domain: &str) -> DomainConfig {
    let (name, bg, accent, text) = match domain {
        "nfsu2mod.xyz" => ("NFSU2 Modded", "#0b0c10", "#38bdf8", "#c8d6e5"),
        "nfsu2.xyz" => ("NFSU2 Underground 2", "#0b0c10", "#e11d48", "#ffffff"),
        "nfscrb.xyz" => ("NFS Carbon Reborn", "#0b0c10", "#f59e0b", "#ffffff"),
        _ => (domain, "#0b0c10", "#38bdf8", "#c8d6e5"),
    };
    DomainConfig { domain: domain.to_string(), name: name.to_string(), bg: bg.to_string(), accent: accent.to_string(), text: text.to_string(), icon_url: None }
}

fn default_rpc(domain: &str) -> RpcManifest {
    RpcManifest {
        domain: domain.to_string(),
        details: "Main Menu".to_string(),
        state: format!("Idle — {domain}"),
        large_image_key: Some("logo".to_string()),
        large_image_text: Some(format!("anyNFS.xyz — {domain}")),
        small_image_key: Some("logo".to_string()),
        small_image_text: Some("anyNFS.xyz desktop".to_string()),
    }
}
