//! anyNFS.xyz desktop — native Rust Tauri MVP.
//!
//! Design notes
//! -------------
//! * Vite is removed. Rust owns dev/build (`cargo tauri dev` / `cargo tauri build`); there is
//!   no HTML/JS frontend build step. The domain selector is an embedded HTML document served by
//!   Rust over a custom `anynfs://` protocol.
//! * Tauri windows *are* webviews, so each domain runs in its own isolated `WebviewWindow`
//!   (separate webview instance per domain). F11 toggles fullscreen on the active playback
//!   window.
//! * The native UI colour scheme and window icon are scraped live from the selected domain's
//!   HTML/CSS and cached under a per-domain directory (`<app-data>/cache/<domain>/`).
//! * Discord Rich Presence runs on the Rust side via Discord's IPC. Per-domain game state lives
//!   inside the playback page's JS, so reading it from outside the webview is not feasible; a
//!   tiny read-only script is injected into the playback page, modelled on the nfsu2mod-desktop
//!   `main.js`/`nfsu2mod.js` anti-cheat-safe pattern:
//!     - read-only observation of `window.__nfsu2` and the DOM (no mutation, no cheats)
//!     - NO cheat-signature globals are written to `window`
//!     - activity is reported to the host through the Tauri bridge (`window.__TAURI_INTERNALS__.invoke`)
//!   F11 is also exposed through that same bridge as a benign UI hotkey (fullscreen toggle).
//! * Config schema + Discord RPC manifest are fetched from raw GitHub URLs with an embedded
//!   fallback, so the app works even before those files are published to the remote.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use image::load_from_memory;
use reqwest::Url;
use serde::Deserialize;
use tauri::{generate_context, generate_handler, Manager, WebviewWindowBuilder, WebviewUrl};

mod config;
mod rpc;
mod scraping;

use config::{fetch_bytes, fetch_domain_config, fetch_rpc_manifest, RpcManifest};
use rpc::RpcClient;
use scraping::Theme;

// ---------------------------------------------------------------------------
// Embedded selector UI (Rust-owned; no Vite, no external frontend build)
// ---------------------------------------------------------------------------

const SELECTOR_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>anyNFS.xyz — Domain Selector</title>
<style>
  * { box-sizing: border-box; margin: 0; padding: 0; }
  html, body { height: 100%; }
  body {
    font-family: system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
    background: var(--anynfs-bg, #0b0c10);
    color: var(--anynfs-text, #c8d6e5);
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 14px; padding: 24px;
  }
  h1 { font-size: 26px; letter-spacing: 0.5px; }
  .swatch { width: 44px; height: 44px; border-radius: 50%; border: 3px solid var(--anynfs-accent, #38bdf8); }
  .current { font-weight: bold; color: var(--anynfs-accent, #38bdf8); font-size: 15px; margin: 8px 0 4px; }
  .grid { display: grid; grid-template-columns: repeat(3, 170px); gap: 14px; width: 100%; max-width: 560px; }
  .card { border: 1px solid var(--anynfs-accent, #38bdf8); border-radius: 12px; padding: 14px; background: var(--anynfs-bg, #0b0c10); text-align: center; }
  .domain { font-weight: bold; font-size: 14px; margin-bottom: 6px; color: var(--anynfs-text, #c8d6e5); }
  .desc { font-size: 11.5px; color: #94a3b8; margin-bottom: 10px; line-height: 1.4; }
  button {
    width: 100%; padding: 11px; border-radius: 8px; border: 1px solid var(--anynfs-accent, #38bdf8);
    background: var(--anynfs-bg, #0b0c10); color: var(--anynfs-text, #c8d6e5); font-size: 13px; cursor: pointer;
  }
  button:hover { filter: brightness(1.15); }
  .hint { margin-top: 16px; font-size: 12px; color: #94a3b8; text-align: center; line-height: 1.5; }
  .status { font-size: 12px; min-height: 16px; margin-top: 8px; color: #4ade80; }
</style>
</head>
<body>
  <div class="swatch" id="swatch"></div>
  <h1>anyNFS.xyz</h1>
  <div class="current" id="currentDomain">Select a domain</div>
  <div class="grid" id="grid"></div>
  <div class="status" id="status"></div>
  <div class="hint">F11 toggles fullscreen in the playback window • Theme colour + icon are scraped live from each domain and cached per-domain</div>
<script>
(async () => {
  const grid = document.getElementById('grid');
  const status = document.getElementById('status');
  const currentDomainEl = document.getElementById('currentDomain');
  const swatch = document.getElementById('swatch');
  const domains = ['nfsu2mod.xyz', 'nfsu2.xyz', 'nfscrb.xyz'];
  const labels = { 'nfsu2mod.xyz': 'NFSU2 Modded', 'nfsu2.xyz': 'NFSU2 Underground 2', 'nfscrb.xyz': 'NFS Carbon Reborn' };
  const descs = { 'nfsu2mod.xyz': 'Underground 2 Revived (modded)', 'nfsu2.xyz': 'Need for Speed: Underground 2', 'nfscrb.xyz': 'Need for Speed: Carbon Reborn' };

  async function applyTheme(theme) {
    document.documentElement.style.setProperty('--anynfs-bg', theme.bg);
    document.documentElement.style.setProperty('--anynfs-accent', theme.accent);
    document.documentElement.style.setProperty('--anynfs-text', theme.text);
    swatch.style.background = theme.accent;
  }

  async function loadTheme() {
    try {
      const theme = await window.__TAURI_INTERNALS__.invoke('anynfs_get_theme');
      await applyTheme(theme);
      const d = theme.currentDomain || '';
      currentDomainEl.textContent = d ? `Domain: ${d}` : 'Select a domain';
      if (d) status.textContent = `Theme + icon applied from live scrape of ${d}`;
    } catch (_) { status.textContent = 'Theme unavailable'; }
  }

  grid.innerHTML = domains.map(d => `
    <div class="card">
      <div class="domain">${labels[d]}</div>
      <div class="desc">${descs[d]}</div>
      <button data-domain="${d}">Open ${labels[d]}</button>
    </div>
  `).join('');

  grid.addEventListener('click', async (ev) => {
    const btn = ev.target.closest('button');
    if (!btn) return;
    const domain = btn.dataset.domain;
    status.textContent = `Loading ${domain} ...`;
    try {
      await window.__TAURI_INTERNALS__.invoke('anynfs_select_domain', { domain });
      status.textContent = `${domain} opened in an isolated webview.`;
    } catch (e) { status.textContent = `Error: ${e}`; }
  });

  loadTheme();
})();
</script>
</body>
</html>"#;

// ---------------------------------------------------------------------------
// Anti-cheat-safe in-page bridge scripts (read-only game-state observers).
// Modelled on nfsu2mod-desktop main.js / nfsu2mod.js. No cheat-signature
// globals are written to window; activity is reported via the Tauri bridge.
// ---------------------------------------------------------------------------

// Theme applied to the selector webview after a domain is selected.
const THEME_APPLY_TEMPLATE: &str = r#"
(async () => {
  try {
    const t = __THEME_JSON__;
    document.documentElement.style.setProperty('--anynfs-bg', t.bg);
    document.documentElement.style.setProperty('--anynfs-accent', t.accent);
    document.documentElement.style.setProperty('--anynfs-text', t.text);
    document.getElementById('swatch').style.background = t.accent;
    document.getElementById('currentDomain').textContent = 'Domain: __DOMAIN__';
  } catch (_) {}
})();
"#;

const NFSU2MOD_BRIDGE_TEMPLATE: &str = r#"
(() => {
  // anyNFS.xyz Discord Rich Presence bridge for nfsu2mod.xyz.  // Anti-cheat-safe, modelled on nfsu2mod-desktop's nfsu2mod.js:
  //  * read-only observation (window.__nfsu2 + DOM); no mutation, no cheats
  //  * NO cheat-signature globals written to window
  //  * activity reported to host via window.__TAURI_INTERNALS__.invoke (Tauri bridge)
  const DOMAIN = 'nfsu2mod.xyz';
  const CMD = 'anynfs_rpc_set_activity';

  function readStatus() {
    const isPlaying = Boolean(document.body && document.body.dataset.phase === 'playing');
    const text = (id) => { const el = document.getElementById(id); return el ? el.textContent.trim() : ''; };
    const onlineTotal = text('players-online');
    const onlineRoam = text('players-roam');
    const lobbyText = (() => { const el = document.getElementById('lobby'); return el && !el.hidden ? el.textContent.trim() : ''; })();
    let gameData = { saveName: '', fps: 0, statusMap: {}, peersCount: 0 };
    if (window.__nfsu2) {
      const nf = window.__nfsu2;
      gameData.saveName = typeof nf.save === 'string' ? nf.save : '';
      gameData.fps = nf.fps ? Math.round(nf.fps.game || 0) : 0;
      const session = nf.session;
      if (session && session.status && typeof session.status.read === 'function') {
        try { const m = session.status.read(); if (m) { for (const [k, v] of m) gameData.statusMap[k] = v; } } catch (_) {}
      }
      if (session && session.net && Array.isArray(session.net.peers)) gameData.peersCount = session.net.peers.length;
    }
    return { isPlaying, statusText: text('status'), onlineRoam, onlineTotal, lobbyText, gameData };
  }

  function compute(info) {
    const isPlaying = Boolean(info && info.isPlaying);
    const sm = info?.gameData?.statusMap || {};
    const drive = sm.drive ?? 0;
    const netrace = sm.netrace ?? 0;
    const engage = sm.engage ?? 0;
    const isInRace = netrace > 0 || engage > 0;
    if (isPlaying && isInRace) {
      return { details: 'Multiplayer Online Race', state: `${info.gameData.peersCount > 0 ? info.gameData.peersCount + 1 + ' Racers | ' : ''}Bayview Circuit`, startTimestamp: Date.now(), largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — NFSU2 Modded', smallImageText: 'NFSU2 Modded', instance: false };
    }
    if (isPlaying && (drive & 3) && !(drive & 8)) {
      return { details: 'Cruising Bayview (Free Roam)', state: 'Exploring Bayview City', startTimestamp: Date.now(), largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — NFSU2 Modded', smallImageText: 'NFSU2 Modded', instance: false };
    }
    if (isPlaying) {
      return { details: 'In Game', state: info.statusText || 'Bayview', startTimestamp: Date.now(), largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — NFSU2 Modded', smallImageText: 'NFSU2 Modded', instance: false };
    }
    return { details: 'Main Menu', state: 'Idle — NFSU2 Modded', largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — NFSU2 Modded', instance: false };
  }

  let lastKey = '';
  function tick() {
    try {
      const info = readStatus();
      const activity = compute(info);
      const key = activity.details + '|' + activity.state;
      if (key !== lastKey) {
        lastKey = key;
        if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
          window.__TAURI_INTERNALS__.invoke(CMD, { domain: DOMAIN, activity });
        }
      }
    } catch (err) { console.warn('[anynfs] tick', err); }
  }
  // Benign fullscreen hotkey (F11) — UI affordance only, no game-state access.
  window.addEventListener('keydown', (ev) => {
    if (ev.key === 'F11' && window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
      window.__TAURI_INTERNALS__.invoke('anynfs_toggle_fullscreen');
    }
  });
  if (document.documentElement.dataset.anynfsBridge === '1') return;
  document.documentElement.dataset.anynfsBridge = '1';
  tick();
  setInterval(tick, 4000);
})();
"#;

const GENERIC_BRIDGE_TEMPLATE: &str = r#"
(() => {
  const DOMAIN = '__DOMAIN__';
  const CMD = 'anynfs_rpc_set_activity';
  function readStatus() {
    const isPlaying = Boolean(document.body && document.body.dataset.phase === 'playing');
    const statusEl = document.getElementById('status') || document.getElementById('players-online');
    const statusText = statusEl ? statusEl.textContent.trim() : '';
    return { isPlaying, statusText, title: document.title || '' };
  }

  function compute(info) {
    const isPlaying = Boolean(info && info.isPlaying);
    if (!isPlaying) {
      return { details: 'Main Menu', state: 'Idle — __DOMAIN__', largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — __DOMAIN__', instance: false };
    }
    return { details: 'In Game', state: info.statusText || info.title || 'Playing', startTimestamp: Date.now(), largeImageKey: 'logo', largeImageText: 'anyNFS.xyz — __DOMAIN__', smallImageText: '__DOMAIN__', instance: false };
  }

  let lastKey = '';
  function tick() {
    try {
      const info = readStatus();
      const activity = compute(info);
      const key = activity.details + '|' + activity.state;
      if (key !== lastKey) {
        lastKey = key;
        if (window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
          window.__TAURI_INTERNALS__.invoke(CMD, { domain: DOMAIN, activity });
        }
      }
    } catch (_) {}
  }
  window.addEventListener('keydown', (ev) => {
    if (ev.key === 'F11' && window.__TAURI_INTERNALS__ && typeof window.__TAURI_INTERNALS__.invoke === 'function') {
      window.__TAURI_INTERNALS__.invoke('anynfs_toggle_fullscreen');
    }
  });
  if (document.documentElement.dataset.anynfsBridge === '1') return;
  document.documentElement.dataset.anynfsBridge = '1';
  tick();
  setInterval(tick, 4000);
})();
"#;

fn nfsu2mod_bridge_js() -> String {
    NFSU2MOD_BRIDGE_TEMPLATE.to_string()
}

fn generic_bridge_js(domain: &str) -> String {
    GENERIC_BRIDGE_TEMPLATE.replace("__DOMAIN__", domain)
}

fn bridge_js_for_domain(domain: &str) -> String {
    if domain == "nfsu2mod.xyz" {
        nfsu2mod_bridge_js()
    } else {
        generic_bridge_js(domain)
    }
}

/// Convert a domain into a Tauri-safe webview window label.
/// Tauri labels may only contain alphanumeric chars, `-`, `/`, `:` and `_`;
/// dots (as in `nfsu2mod.xyz`) are rejected, so we map them to `-`.
fn domain_to_label(domain: &str) -> String {
    domain
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '/' || c == ':' || c == '_' {
            c
        } else {
            '-'
        })
        .collect()
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, serde::Serialize)]
struct ThemeView {
    bg: String,
    accent: String,
    text: String,
    current_domain: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct RpcActivity {
    details: String,
    state: String,
    #[serde(default)]
    start_timestamp: Option<i64>,
    #[serde(default)]
    large_image_key: Option<String>,
    #[serde(default)]
    large_image_text: Option<String>,
    #[serde(default)]
    small_image_key: Option<String>,
    #[serde(default)]
    small_image_text: Option<String>,
    #[serde(default)]
    instance: bool,
}

struct AppState {
    cache_root: PathBuf,
    current: Arc<Mutex<Option<String>>>,
    themes: Arc<Mutex<HashMap<String, Theme>>>,
    icons: Arc<Mutex<HashMap<String, (Vec<u8>, u32, u32)>>>,
    windows: Arc<Mutex<HashMap<String, tauri::WebviewWindow>>>,
    selector_window: Arc<Mutex<Option<tauri::WebviewWindow>>>,
    active_domain: Arc<Mutex<Option<String>>>,
    rpc: Arc<Mutex<Option<RpcClient>>>,
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
async fn anynfs_get_theme(state: tauri::State<'_, AppState>) -> Result<ThemeView, String> {
    let current = state.current.lock().map_err(|e| e.to_string())?;
    let themes = state.themes.lock().map_err(|e| e.to_string())?;
    let theme = current.as_ref().and_then(|d| themes.get(d)).cloned();
    Ok(ThemeView {
        bg: theme.as_ref().map(|t| t.bg.as_str()).unwrap_or("#0b0c10").to_string(),
        accent: theme.as_ref().map(|t| t.accent.as_str()).unwrap_or("#38bdf8").to_string(),
        text: theme.as_ref().map(|t| t.text.as_str()).unwrap_or("#c8d6e5").to_string(),
        current_domain: current.clone(),
    })
}

#[tauri::command]
async fn anynfs_select_domain(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    domain: String,
) -> Result<(), String> {
    // 1. Fetch per-domain config schema + RPC manifest from raw GitHub URLs
    //    (best-effort, embedded fallback), and scrape the domain's live HTML/CSS
    //    for theme + icon. Runs on a blocking thread pool (scraper::Html is !Sync;
    //    the Tauri command future must be Send).
    let domain_for_scrape = domain.clone();
    let (scraped, config, rpc_manifest) =
        tokio::task::spawn_blocking(move || {
            (
                scraping::scrape_domain(&domain_for_scrape),
                fetch_domain_config(&domain_for_scrape),
                fetch_rpc_manifest(&domain_for_scrape),
            )
        })
        .await
        .map_err(|e| e.to_string())?;
    let scraped = scraped.map_err(|e| format!("scrape failed: {e}"))?;
    log::info!(
        "selected domain {domain} — config '{}' (live theme {:?})",
        config.name,
        scraped.theme
    );

    // 2. Decode icon PNG to RGBA for the window icon. Prefer the live-scraped
    //    icon, fall back to the config's icon_url.
    let icon_png =
        scraped.icon_png.or_else(|| config.icon_url.as_deref().and_then(fetch_bytes));
    let icon_rgba = icon_png.as_ref().map(|png| {
        let img = load_from_memory(png).expect("icon validated as image").into_rgba8();
        let (w, h) = img.dimensions();
        (img.into_raw(), w, h)
    });

    // 2b. Cache the scraped theme + icon in the isolated per-domain cache dir.
    let domain_cache_dir = state.cache_root.join(&domain);
    std::fs::create_dir_all(&domain_cache_dir).map_err(|e| e.to_string())?;
    let theme_json = serde_json::to_string(&scraped.theme).map_err(|e| e.to_string())?;
    std::fs::write(domain_cache_dir.join("theme.json"), theme_json).map_err(|e| e.to_string())?;
    if let Some(png) = &icon_png {
        std::fs::write(domain_cache_dir.join("icon.png"), png).map_err(|e| e.to_string())?;
    }

    // 3. Persist theme, icon and current domain.
    {
        let mut themes = state.themes.lock().map_err(|e| e.to_string())?;
        themes.insert(domain.clone(), scraped.theme.clone());
    }
    if let Some((ref rgba, w, h)) = icon_rgba {
        let mut icons = state.icons.lock().map_err(|e| e.to_string())?;
        icons.insert(domain.clone(), (rgba.clone(), w, h));
    }
    {
        let mut current = state.current.lock().map_err(|e| e.to_string())?;
        *current = Some(domain.clone());
    }

    // 4. Switch the native UI window icon (selector window).
    if let Some((rgba, w, h)) = &icon_rgba {
        if let Some(selector) = state.selector_window.lock().map_err(|e| e.to_string())?.as_ref() {
            let icon = tauri::image::Image::new(rgba, *w, *h);
            let _ = selector.set_icon(icon);
        }
    }

    // 5. Push the new theme to the already-open selector webview.
    if let Some(selector) = state.selector_window.lock().map_err(|e| e.to_string())?.as_ref() {
        let theme_json = serde_json::to_string(&scraped.theme).map_err(|e| e.to_string())?;
        let js = THEME_APPLY_TEMPLATE
            .replace("__THEME_JSON__", &theme_json)
            .replace("__DOMAIN__", &domain);
        let _ = selector.eval(&js);
    }

    // 6. Open (or reuse) the isolated playback webview for this domain.
    let domain_for_js = domain.clone();
    let label = domain_to_label(&domain);
    let url = format!("https://{domain}/");
    let parsed = Url::parse(&url).map_err(|e| e.to_string())?;
    {
        let mut windows = state.windows.lock().map_err(|e| e.to_string())?;
        if !windows.contains_key(&label) {
            let w = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed))
                .title(format!("anyNFS.xyz — {domain}"))
                .inner_size(1280.0, 800.0)
                .resizable(true)
                .on_page_load(move |window, _payload| {
                    // Inject the anti-cheat-safe read-only bridge (game-state observer + F11 hotkey)
                    // into the playback page once it has loaded.
                    let js = bridge_js_for_domain(&domain_for_js);
                    let _ = window.eval(&js);
                })
                .build()
                .map_err(|e| e.to_string())?;
            windows.insert(label.clone(), w);
        }
    }
    {
        let windows = state.windows.lock().map_err(|e| e.to_string())?;
        if let Some(w) = windows.get(&label) {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }

    // 7. Record active domain and kick off Discord Rich Presence with domain-specific state.
    {
        let mut ad = state.active_domain.lock().map_err(|e| e.to_string())?;
        *ad = Some(domain.clone());
    }
    let activity = rpc::initial_activity_value(&domain, &scraped.theme, &rpc_manifest);
    if let Ok(mut m) = state.rpc.lock().map_err(|e| e.to_string()) {
        if let Some(rpc) = m.as_mut() {
            let _ = rpc
                .set_activity(&activity)
                .map_err(|e| format!("rpc set_activity failed: {e}"));
        }
    }

    Ok(())
}

#[tauri::command]
async fn anynfs_rpc_set_activity(
    state: tauri::State<'_, AppState>,
    domain: String,
    activity: RpcActivity,
) -> Result<(), String> {
    let value = rpc::activity_value(&domain, &activity);
    if let Ok(mut m) = state.rpc.lock().map_err(|e| e.to_string()) {
        if let Some(rpc) = m.as_mut() {
            rpc.set_activity(&value).map_err(|e| format!("rpc set_activity failed: {e}"))?;
        }
    }
    Ok(())
}

#[tauri::command]
fn anynfs_toggle_fullscreen(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let domain = state.active_domain.lock().map_err(|e| e.to_string())?;
    let domain = domain.as_deref().unwrap_or("");
    if domain.is_empty() {
        return Ok(());
    }
    let label = domain_to_label(domain);
    let windows = state.windows.lock().map_err(|e| e.to_string())?;
    let w = windows
        .get(&label)
        .ok_or_else(|| format!("no webview for {domain} (label {label})"))?;
    let fullscreen = w.is_fullscreen().map_err(|e| e.to_string())?;
    w.set_fullscreen(!fullscreen).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// App entry
// ---------------------------------------------------------------------------

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let cache_root = app
                .path()
                .app_cache_dir()
                .map_err(|e| anyhow::anyhow!("app_cache_dir failed: {e}"))?;
            std::fs::create_dir_all(&cache_root).map_err(|e| e.to_string())?;
            // Per-domain cache directories (isolated, created up front).
            for d in ["nfsu2mod.xyz", "nfsu2.xyz", "nfscrb.xyz"] {
                let _ = std::fs::create_dir_all(cache_root.join(d));
            }

            let state = AppState {
                cache_root,
                current: Arc::new(Mutex::new(None)),
                themes: Arc::new(Mutex::new(HashMap::new())),
                icons: Arc::new(Mutex::new(HashMap::new())),
                windows: Arc::new(Mutex::new(HashMap::new())),
                selector_window: Arc::new(Mutex::new(None)),
                active_domain: Arc::new(Mutex::new(None)),
                rpc: Arc::new(Mutex::new(None)),
            };
            app.manage(state);

            // Best-effort Discord RPC init (no secrets; public client id).
            match RpcClient::connect(1568757859649323199) {
                Ok(rpc) => {
                    if rpc.is_connected() {
                        log::info!("discord rpc connected");
                    } else {
                        log::warn!("discord rpc not connected (Discord may not be running)");
                    }
                    if let Ok(mut m) = app.state::<AppState>().rpc.lock() {
                        *m = Some(rpc);
                    }
                }
                Err(e) => log::warn!("discord rpc connect failed: {e}"),
            }

            Ok(())
        })
        .invoke_handler(generate_handler![
            anynfs_get_theme,
            anynfs_select_domain,
            anynfs_rpc_set_activity,
            anynfs_toggle_fullscreen,
        ])
        // Custom protocol serving the Rust-owned embedded selector UI.
        .register_uri_scheme_protocol("anynfs", move |_ctx, req| {
            let path = req.uri().path();
            if path == "/selector" {
                tauri::http::Response::builder()
                    .header("Content-Type", "text/html; charset=utf-8")
                    .body(SELECTOR_HTML.as_bytes().to_vec())
                    .unwrap()
            } else {
                tauri::http::Response::builder()
                    .status(tauri::http::StatusCode::NOT_FOUND)
                    .body(b"not found".to_vec())
                    .unwrap()
            }
        });

    let context = generate_context!();
    let app = builder.build(context).expect("failed to build tauri application");

    app.run(|app, event| {
        if let tauri::RunEvent::Ready = event {
            let selector = WebviewWindowBuilder::new(app, "selector", WebviewUrl::CustomProtocol(
                "anynfs://app/selector".parse().expect("valid custom-protocol url"),
            ))
            .title("anyNFS.xyz — Domain Selector")
            .inner_size(800.0, 600.0)
            .resizable(true)
            .build()
            .expect("failed to create selector window");
            if let Ok(mut m) = app.state::<AppState>().selector_window.lock() {
                *m = Some(selector);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::domain_to_label;

    #[test]
    fn domain_to_label_is_tauri_safe() {
        let cases = [
            ("nfsu2mod.xyz", "nfsu2mod-xyz"),
            ("nfsu2.xyz", "nfsu2-xyz"),
            ("nfscrb.xyz", "nfscrb-xyz"),
            ("example.com", "example-com"),
        ];
        for (input, expected) in cases {
            let label = domain_to_label(input);
            assert_eq!(label, expected, "label for {input} should be {expected}");
            // Every char must be in Tauri's allowed label set.
            assert!(
                label
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '-' || c == '/' || c == ':' || c == '_'),
                "label '{label}' contains unsafe chars"
            );
        }
    }
}
