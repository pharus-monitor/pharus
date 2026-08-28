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
    pharus_common::platform_string()
}

async fn download(url: &str) -> Result<Vec<u8>, anyhow::Error> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let resp = client.get(url).send().await?.error_for_status()?;
    Ok(resp.bytes().await?.to_vec())
}

fn sha256_hex(data: &[u8]) -> String {
    pharus_common::sha256_hex(data)
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
        // Stage the swap *next to the destination*, never from a single fixed
        // staging dir, because themes and binary often live on different
        // filesystems and rename across devices fails with EXDEV.
        let themes_root = &state.themes_root;
        let bin_tmp = current
            .parent()
            .map(|p| p.join(format!(".pharus-bin-{version}")))
            .unwrap_or_else(|| PathBuf::from(format!(".pharus-bin-{version}")));
        let themes_tmp = themes_root
            .parent()
            .map(|p| p.join(format!(".pharus-themes-{version}")))
            .unwrap_or_else(|| PathBuf::from(format!(".pharus-themes-{version}")));

        broadcast_server_status(state, "applying", false, None);
        apply_server_replace_cross_device(
            &staged_pharus,
            &staged_themes,
            &current,
            &bin_tmp,
            themes_root,
            &themes_tmp,
        )
        .map_err(|e| e.to_string())?;
        // Exit and let systemd (Restart=always) pick up the swapped binary.
        // Spawning our own replacement would fight the service manager and the
        // child would inherit this process's read-only mount namespace anyway.
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
/// Three-way rename that survives different destination filesystems.
/// Stages each swap through a sibling of the destination (same device) and
/// keeps the previous version as a `.old` so the next update can recover.
#[cfg(unix)]
fn apply_server_replace_cross_device(
    src_pharus: &std::path::Path,
    src_themes: &std::path::Path,
    current: &std::path::Path,
    bin_tmp: &std::path::Path,
    themes_root: &std::path::Path,
    themes_tmp: &std::path::Path,
) -> Result<(), anyhow::Error> {
    use std::os::unix::fs::PermissionsExt;

    // Copy a directory tree across filesystems (rename would fail with
    // EXDEV). Used for the themes half of the swap; binary stays on a
    // single filesystem so its swap can use plain rename.
    fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> Result<(), anyhow::Error> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            let ft = entry.file_type()?;
            if ft.is_dir() {
                copy_tree(&from, &to)?;
            } else if ft.is_symlink() {
                let target = std::fs::read_link(&from)?;
                std::os::unix::fs::symlink(&target, &to)?;
            } else {
                std::fs::copy(&from, &to)?;
                let _ = std::fs::set_permissions(
                    &to,
                    entry
                        .metadata()
                        .ok()
                        .map(|m| m.permissions())
                        .unwrap_or_else(|| std::fs::Permissions::from_mode(0o644)),
                );
            }
        }
        Ok(())
    }

    // Themes: copy into a sibling of themes_root, swap in place, retire
    // the old one to .old. Read+write crosses filesystems where rename
    // cannot.
    if themes_tmp.exists() {
        if themes_tmp.is_dir() {
            std::fs::remove_dir_all(themes_tmp)?;
        } else {
            std::fs::remove_file(themes_tmp)?;
        }
    }
    copy_tree(src_themes, themes_tmp)?;
    if themes_root.exists() {
        let old = themes_root.with_extension(".old");
        if old.exists() {
            if old.is_dir() {
                std::fs::remove_dir_all(&old)?;
            } else {
                std::fs::remove_file(&old)?;
            }
        }
        std::fs::rename(themes_root, &old)?;
    }
    std::fs::rename(themes_tmp, themes_root)?;

    // Binary: both staged_pharus and current live on the same filesystem
    // (the running binary's parent), so rename is fine.
    if bin_tmp.exists() {
        if bin_tmp.is_dir() {
            std::fs::remove_dir_all(bin_tmp)?;
        } else {
            std::fs::remove_file(bin_tmp)?;
        }
    }
    // Stage lives under the binary's parent directory.
    let bin_tmp_parent = bin_tmp.parent().ok_or_else(|| {
        anyhow::anyhow!("staging path {} has no parent", bin_tmp.display())
    })?;
    std::fs::create_dir_all(bin_tmp_parent)?;
    std::fs::rename(src_pharus, bin_tmp)?;
    if current.exists() {
        let old = current.with_extension(".old");
        if old.exists() {
            std::fs::remove_file(&old)?;
        }
        std::fs::rename(current, &old)?;
    }
    std::fs::set_permissions(bin_tmp, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(bin_tmp, current)?;
    Ok(())
}
