//! Online-update manifest: which versions can be installed or rolled back to,
//! and where each platform's server / agent asset lives. The copy embedded at
//! build time is the baseline; a configured URL can serve a fresher list that
//! is merged in at runtime and cached for a few minutes.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
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
    /// Server (panel) assets per platform. Only Linux is packaged by CI.
    #[serde(default)]
    pub server: HashMap<String, UpdateAsset>,
    /// Agent assets per platform.
    #[serde(default)]
    pub agent: HashMap<String, UpdateAsset>,
    /// Release notes keyed by language code (en, zh-CN, ja, ru).
    #[serde(default)]
    pub notes: HashMap<String, String>,
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

/// Find the asset for `version` + `kind` ("server"|"agent") + platform.
pub fn find_asset<'a>(
    manifest: &'a UpdateManifest,
    version: &str,
    kind: &str,
    platform: &str,
) -> Option<&'a UpdateAsset> {
    manifest.versions.iter().find(|v| v.version == version).and_then(|v| {
        if kind == "server" {
            v.server.get(platform)
        } else {
            v.agent.get(platform)
        }
    })
}

/// Whether a platform's asset is a raw executable or a tar.gz to unpack.
pub fn asset_kind(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        "exe"
    } else {
        "tar_gz"
    }
}

/// This server's platform string (e.g. "linux-x86_64").
pub fn server_platform() -> String {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        "x86" => "i686",
        other => other,
    };
    format!("{}-{arch}", std::env::consts::OS)
}

async fn download(url: &str) -> Result<Vec<u8>, anyhow::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn unpack(tar_gz: &std::path::Path, dest: &std::path::Path) -> Result<(), anyhow::Error> {
    let file = std::fs::File::open(tar_gz)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(dest)?;
    Ok(())
}

fn broadcast_server_status(state: &SharedState, phase: &str, done: bool, error: Option<String>) {
    state.broadcast(pharus_common::BrowserMsg::UpdateStatus {
        agent_id: None,
        kind: "server".into(),
        phase: phase.to_string(),
        done,
        error,
    });
}

/// Download, verify and stage a server bundle, then swap binary + themes and
/// relaunch the server. Called from the admin endpoint; the process exits to
/// let the replacement take over.
pub async fn apply_server_update(state: &SharedState, version: String) -> Result<(), String> {
    let manifest = load_manifest(state).await;
    let platform = server_platform();
    let asset = find_asset(&manifest, &version, "server", &platform)
        .ok_or_else(|| format!("版本 {version} 没有服务器 {platform} 安装包"))?;

    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let work = current
        .parent()
        .map(|p| p.join(".pharus-update"))
        .unwrap_or_else(|| PathBuf::from(".pharus-update"));
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let stage = work.join(format!("server-{version}"));
    if stage.is_dir() {
        std::fs::remove_dir_all(&stage).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;

    let bundle = stage.join("bundle.tar.gz");
    broadcast_server_status(state, "downloading", false, None);
    let bytes = download(&asset.url).await.map_err(|e| e.to_string())?;
    std::fs::write(&bundle, &bytes).map_err(|e| e.to_string())?;
    broadcast_server_status(state, "verifying", false, None);
    if sha256_hex(&bytes) != asset.sha256 {
        return Err("sha256 校验失败".into());
    }
    unpack(&bundle, &stage).map_err(|e| e.to_string())?;

    let staged_pharus = stage.join("pharus");
    let staged_themes = stage.join("themes");
    if !staged_pharus.is_file() || !staged_themes.is_dir() {
        return Err("安装包缺少 pharus 或 themes".into());
    }

    #[cfg(unix)]
    {
        broadcast_server_status(state, "applying", false, None);
        apply_server_replace(&staged_pharus, &staged_themes, &current, &state.themes_root)
            .map_err(|e| e.to_string())?;
        // Let the writer flush whatever it can, then the process exits and the
        // freshly spawned replacement keeps serving.
        broadcast_server_status(state, "restarting", true, None);
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        std::process::exit(0);
    }
    #[cfg(not(unix))]
    {
        Err("当前平台暂不支持服务器在线更新（无 Windows 服务器安装包）".into())
    }
}

/// Swap binary + themes for the running server and relaunch it with the same
/// args and working directory. Only reached on Unix where rename-over-running
/// is safe.
#[cfg(unix)]
fn apply_server_replace(
    staged_pharus: &std::path::Path,
    staged_themes: &std::path::Path,
    current: &std::path::Path,
    themes_root: &std::path::Path,
) -> Result<(), anyhow::Error> {
    use std::os::unix::fs::PermissionsExt;

    let work = current
        .parent()
        .map(|p| p.join(".pharus-update"))
        .unwrap_or_else(|| PathBuf::from(".pharus-update"));

    // Swap the themes directory (the running server keeps serving; the swap
    // happens right before the exit).
    let backup = work.join("themes-old");
    if themes_root.exists() {
        if backup.exists() {
            std::fs::remove_dir_all(&backup)?;
        }
        std::fs::rename(themes_root, &backup)?;
    }
    std::fs::rename(staged_themes, themes_root)?;

    // Swap the binary.
    std::fs::set_permissions(staged_pharus, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(staged_pharus, current)?;

    // Relaunch with the original args + cwd so the new server binds identically.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = std::env::current_dir()?;
    std::process::Command::new(current)
        .current_dir(&cwd)
        .args(&args)
        .spawn()?;
    Ok(())
}
