//! Online-update manifest: which agent versions can be installed or rolled
//! back to, and where each platform's asset lives. The copy embedded at build
//! time is the baseline; a configured URL can serve a fresher list that is
//! merged in at runtime and cached for a few minutes.

use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

use crate::state::SharedState;

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateAsset {
    pub url: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateVersion {
    pub version: String,
    pub assets: HashMap<String, UpdateAsset>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpdateManifest {
    pub versions: Vec<UpdateVersion>,
}

/// Default refresh source; operators can point this at their own mirror via
/// the `update_manifest_url` site setting.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/pharus-monitor/pharus/main/updates.json";

const EMBEDDED: &str = include_str!("../../updates.json");
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

pub fn embedded() -> UpdateManifest {
    serde_json::from_str(EMBEDDED).expect("embedded updates.json is valid")
}

/// Return the manifest: embedded baseline, refreshed from
/// `update_manifest_url` when reachable and not cached too recently.
pub async fn load_manifest(state: &SharedState) -> UpdateManifest {
    let url = {
        let conn = state.db.lock().unwrap();
        crate::db::get_setting(&conn, "update_manifest_url")
            .ok()
            .flatten()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MANIFEST_URL.to_string())
    };
    if let Some(cached) = state.update_cache.lock().unwrap().as_ref() {
        if cached.1.elapsed() < CACHE_TTL {
            return cached.0.clone();
        }
    }
    match fetch(&url).await {
        Ok(manifest) => {
            *state.update_cache.lock().unwrap() = Some((manifest.clone(), Instant::now()));
            manifest
        }
        Err(_) => embedded(),
    }
}

async fn fetch(url: &str) -> Result<UpdateManifest, anyhow::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let text = client.get(url).send().await?.error_for_status()?.text().await?;
    Ok(serde_json::from_str(&text)?)
}

/// Find the asset for `version` + `platform` (e.g. "linux-x86_64").
pub fn find_asset<'a>(
    manifest: &'a UpdateManifest,
    version: &str,
    platform: &str,
) -> Option<&'a UpdateAsset> {
    manifest
        .versions
        .iter()
        .find(|v| v.version == version)
        .and_then(|v| v.assets.get(platform))
}

/// Whether a platform's asset is a raw executable or a tar.gz to unpack.
pub fn asset_kind(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "exe"
    } else {
        "tar_gz"
    }
}
