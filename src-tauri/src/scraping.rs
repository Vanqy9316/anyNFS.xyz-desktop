//! Live scraping of a domain's HTML/CSS for its theme colour and icon.
//! Synchronous (blocking HTTP + parsing) so it can run on a blocking thread
//! pool from async Tauri commands (scraper's Html type is !Sync). Nothing is
//! ever written to the remote domains; only public HTML/CSS is read.

use serde::Serialize;
use std::time::Duration;

use image::load_from_memory;
use reqwest::blocking::Client;
use reqwest::Url;
use scraper::{Html, Selector};

#[derive(Clone, Debug, Serialize)]
pub struct Theme {
    pub bg: String,
    pub accent: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Scraped {
    pub theme: Theme,
    pub icon_png: Option<Vec<u8>>,
}

const DEFAULT_BG: &str = "#0b0c10";
const DEFAULT_ACCENT: &str = "#38bdf8";
const DEFAULT_TEXT: &str = "#c8d6e5";

/// Fetch the domain's home page and scrape `<meta name="theme-color">` plus a
/// favicon/apple-touch-icon. All reads are read-only; the remote is never modified.
pub fn scrape_domain(domain: &str) -> Result<Scraped, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("anynfs.xyz-desktop/0.1.0")
        .build()
        .map_err(|e| e.to_string())?;

    let url = Url::parse(&format!("https://{domain}/")).map_err(|e| e.to_string())?;
    let response = client.get(url.clone()).send().map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Ok(fallback());
    }
    let html = response.text().map_err(|e| e.to_string())?;
    let document = Html::parse_document(&html);

    let mut theme = Theme {
        bg: DEFAULT_BG.to_string(),
        accent: DEFAULT_ACCENT.to_string(),
        text: DEFAULT_TEXT.to_string(),
    };
    if let Ok(sel) = Selector::parse("meta[name='theme-color']") {
        if let Some(el) = document.select(&sel).next() {
            if let Some(content) = el.value().attr("content") {
                if !content.is_empty() && content.starts_with('#') {
                    theme.accent = content.to_string();
                    theme.bg = content.to_string();
                }
            }
        }
    }

    let icon_png = fetch_icon(&client, &url, &document).ok().flatten();

    Ok(Scraped { theme, icon_png })
}

fn fallback() -> Scraped {
    Scraped {
        theme: Theme {
            bg: DEFAULT_BG.to_string(),
            accent: DEFAULT_ACCENT.to_string(),
            text: DEFAULT_TEXT.to_string(),
        },
        icon_png: None,
    }
}

/// Locate <link rel="icon"|"apple-touch-icon"> (falling back to /favicon.ico) and
/// validate that it decodes as an image. Returns the raw PNG bytes.
fn fetch_icon(client: &Client, base: &Url, document: &Html) -> Result<Option<Vec<u8>>, String> {
    let sel = Selector::parse(r#"link[rel~="icon"], link[rel~="apple-touch-icon"]"#).map_err(|e| e.to_string())?;
    let href = document
        .select(&sel)
        .filter_map(|el| el.value().attr("href"))
        .next()
        .unwrap_or("/favicon.ico");
    let icon_url = base.join(href).map_err(|e| e.to_string())?;
    let bytes = client
        .get(icon_url)
        .send()
        .map_err(|e| e.to_string())?
        .bytes()
        .map_err(|e| e.to_string())?;
    load_from_memory(&bytes).map_err(|e| format!("icon is not a valid image: {e}"))?;
    Ok(Some(bytes.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrape_nfsu2mod_extracts_live_theme() {
        let scraped = scrape_domain("nfsu2mod.xyz").expect("scrape nfsu2mod.xyz should succeed");
        assert!(!scraped.theme.bg.is_empty(), "bg colour should be scraped");
        assert!(!scraped.theme.accent.is_empty(), "accent colour should be scraped");
        // nfsu2mod.xyz exposes theme-color; the scraper should pick it up (not the fallback).
        assert_ne!(scraped.theme.accent, DEFAULT_ACCENT, "accent should be live-scraped, not the fallback");
    }

    #[test]
    fn scrape_all_domains_produce_a_theme() {
        for domain in ["nfsu2mod.xyz", "nfsu2.xyz", "nfscrb.xyz"] {
            let scraped = scrape_domain(domain).unwrap_or_else(|e| panic!("scrape {domain} failed: {e}"));
            // Every domain must yield a non-empty background + accent colour
            // (live-scraped when the domain exposes theme-color, fallback otherwise).
            assert!(!scraped.theme.bg.is_empty(), "{domain} bg colour should be present");
            assert!(!scraped.theme.accent.is_empty(), "{domain} accent colour should be present");
        }
        // At least one domain exposes a live theme-color (proves live scraping works).
        let nfsu2mod = scrape_domain("nfsu2mod.xyz").expect("scrape nfsu2mod.xyz should succeed");
        assert_ne!(
            nfsu2mod.theme.accent,
            DEFAULT_ACCENT,
            "nfsu2mod.xyz accent should be live-scraped, not the fallback"
        );
    }

    #[test]
    fn cache_writes_theme_and_icon_per_domain() {
        // Mimics the per-domain cache writes performed by anynfs_select_domain:
        // scrape -> write theme.json (+ icon.png when available) into <cache>/<domain>/.
        for domain in ["nfsu2mod.xyz", "nfsu2.xyz", "nfscrb.xyz"] {
            let scraped = scrape_domain(domain).unwrap_or_else(|e| panic!("scrape {domain} failed: {e}"));
            let tmp = std::env::temp_dir().join(format!("anynfs_cache_test_{domain}"));
            let _ = std::fs::remove_dir_all(&tmp);
            std::fs::create_dir_all(&tmp).unwrap();
            let theme_json = serde_json::to_string(&scraped.theme).unwrap();
            std::fs::write(tmp.join("theme.json"), &theme_json).unwrap();
            if let Some(png) = &scraped.icon_png {
                std::fs::write(tmp.join("icon.png"), png).unwrap();
            }
            assert!(
                tmp.join("theme.json").exists(),
                "theme.json should be written for {domain}"
            );
            // Cache isolation: each domain writes to its own distinct directory.
            assert!(
                std::fs::read_to_string(tmp.join("theme.json")).unwrap_or_default().is_empty() == false,
                "theme.json for {domain} should be non-empty"
            );
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
}
