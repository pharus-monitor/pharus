//! Theme packaging: manifest validation and safe zip extraction.
//!
//! Uploaded/installed themes are arbitrary front-end code, so extraction is
//! the security boundary: no path traversal, no symlinks, bounded sizes
//! (zip-bomb protection) and a strict slug `id`.

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Total uncompressed bytes across the whole archive.
const MAX_TOTAL_UNCOMPRESSED: u64 = 50 * 1024 * 1024;
/// Entry count cap (protects against pathological archives).
const MAX_FILES: usize = 500;
/// Per-entry uncompressed cap.
const MAX_FILE_SIZE: u64 = 25 * 1024 * 1024;
/// Uploaded zip itself must not exceed this (before extraction).
const MAX_ZIP_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct ThemeManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub min_server_version: Option<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
}

fn default_entry() -> String {
    "index.html".into()
}

/// A theme slug may only contain `[a-zA-Z0-9_-]`, never start with a dot and
/// must not look like a path (blocks traversal via the `id`).
pub fn is_valid_slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// True when `path` is a plain entry file inside the archive, with no
/// `..`, absolute or otherwise escaping components.
fn is_safe_rel_path(path: &Path) -> bool {
    !path.is_absolute()
        && path.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn version_at_least(min: Option<&str>) -> bool {
    let Some(min) = min else { return true };
    let current = env!("CARGO_PKG_VERSION");
    compare_versions(current, min)
}

fn compare_versions(current: &str, min: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(current), parse(min));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    true
}

/// Install a theme zip into `themes_root/<id>`.
///
/// Returns the parsed manifest on success. The target directory is fully
/// replaced on success; on any validation/extraction error nothing is left
/// behind (a half-written temp dir is removed).
pub fn install_zip(themes_root: &Path, zip_bytes: &[u8]) -> Result<ThemeManifest> {
    if zip_bytes.len() > MAX_ZIP_BYTES {
        bail!("主题包超过 {MAX_ZIP_BYTES} 字节上限");
    }

    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("不是有效的 zip 文件")?;
    if archive.len() > MAX_FILES {
        bail!("主题包文件数超过上限 {MAX_FILES}");
    }

    // Pass 1: bounds check every entry without inflating anything.
    let mut total: u64 = 0;
    let mut manifest_index: Option<usize> = None;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| anyhow!("读取 zip 条目失败: {e}"))?;
        if entry.encrypted() {
            bail!("主题包包含加密条目，已拒绝");
        }
        let mode = entry.unix_mode().unwrap_or(0);
        if mode & 0o170000 == 0o120000 {
            bail!("主题包包含符号链接，已拒绝");
        }
        let size = entry.size();
        total = total.saturating_add(size);
        if size > MAX_FILE_SIZE || total > MAX_TOTAL_UNCOMPRESSED {
            bail!("主题包解压后体积超限，已拒绝");
        }
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .ok_or_else(|| anyhow!("主题包存在越界路径条目，已拒绝"))?;
        if name == Path::new("manifest.json") {
            manifest_index = Some(i);
        }
    }

    let manifest_index = manifest_index.ok_or_else(|| anyhow!("主题包缺少 manifest.json"))?;

    // Read the manifest out of the archive and validate it.
    let manifest: ThemeManifest = {
        let mut file = archive
            .by_index(manifest_index)
            .map_err(|e| anyhow!("读取 manifest.json 失败: {e}"))?;
        // Bound the read by actual inflated bytes: a zip may lie about the
        // declared size in its central directory, so reading it unbounded
        // would let a small file balloon into RAM.
        let mut buf = Vec::new();
        (&mut file)
            .take(256 * 1024 + 1)
            .read_to_end(&mut buf)
            .map_err(|e| anyhow!("读取 manifest.json 失败: {e}"))?;
        if buf.len() > 256 * 1024 {
            bail!("manifest.json 过大");
        }
        serde_json::from_slice::<ThemeManifest>(&buf).context("manifest.json 格式无效")?
    };
    validate_manifest(&manifest)?;

    let dest = themes_root.join(&manifest.id);
    let tmp = themes_root.join(format!(".tmp-{}", manifest.id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).context("无法创建主题目录")?;

    let extract = (|| -> Result<()> {
        let mut total_written: u64 = 0;
        for i in 0..archive.len() {
            let mut file = archive
                .by_index(i)
                .map_err(|e| anyhow!("读取 zip 条目失败: {e}"))?;
            if file.is_dir() {
                continue;
            }
            let name = file
                .enclosed_name()
                .ok_or_else(|| anyhow!("主题包存在越界路径条目，已拒绝"))?;
            if !is_safe_rel_path(&name) {
                bail!("主题包路径条目非法");
            }
            let out = tmp.join(&name);
            if !out.starts_with(&tmp) {
                bail!("主题包路径条目越界，已拒绝");
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).context("无法创建主题子目录")?;
            }
            let mut out_file = std::fs::File::create(&out).context("无法写入主题文件")?;
            // Enforce the cap on the real inflated byte count, not the
            // declared size: an entry can claim 1 byte yet expand to 2 GiB.
            let written = std::io::copy(&mut (&mut file).take(MAX_FILE_SIZE + 1), &mut out_file)
                .context("解压主题文件失败")?;
            if written > MAX_FILE_SIZE || total_written.saturating_add(written) > MAX_TOTAL_UNCOMPRESSED {
                bail!("主题包解压后体积超限，已拒绝");
            }
            total_written += written;
        }
        Ok(())
    })();

    if let Err(e) = extract {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    // The entry file must actually exist after extraction.
    let entry_path = tmp.join(&manifest.entry);
    if !entry_path.is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        bail!("主题入口文件 {} 不存在", manifest.entry);
    }

    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&tmp, &dest).context("无法移动主题到目标目录")?;
    Ok(manifest)
}

fn validate_manifest(m: &ThemeManifest) -> Result<()> {
    if !is_valid_slug(&m.id) {
        bail!("主题 id 只能是字母数字与 -_，且不能以 . 开头");
    }
    if m.id == "default" {
        bail!("内置 default 主题不能被覆盖");
    }
    if m.name.trim().is_empty() {
        bail!("manifest.name 不能为空");
    }
    let entry = Path::new(&m.entry);
    if !is_safe_rel_path(entry) || entry.file_name().is_none() {
        bail!("manifest.entry 非法");
    }
    if !version_at_least(m.min_server_version.as_deref()) {
        bail!("主题要求 server 版本 >= {}", m.min_server_version.as_deref().unwrap_or("?"));
    }
    if let Some(preview) = &m.preview {
        let p = Path::new(preview);
        if !is_safe_rel_path(p) {
            bail!("manifest.preview 非法");
        }
    }
    Ok(())
}

/// Path to a theme's directory inside the themes root, or None if the slug is
/// unsafe (caller should not touch the filesystem for such ids).
pub fn theme_dir(themes_root: &Path, id: &str) -> Option<PathBuf> {
    if is_valid_slug(id) {
        Some(themes_root.join(id))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pharus-theme-test-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn valid_zip_extracts_and_returns_manifest() {
        let root = temp_root();
        let zip = make_zip(&[
            (
                "manifest.json",
                r#"{"id":"mytheme","name":"My Theme","version":"1.0.0","entry":"index.html"}"#,
            ),
            ("index.html", "<h1>hi</h1>"),
        ]);
        let manifest = install_zip(&root, &zip).unwrap();
        assert_eq!(manifest.id, "mytheme");
        assert!(root.join("mytheme/index.html").is_file());
        let _ = std::fs::remove_dir_all(root.join("mytheme"));
    }

    #[test]
    fn traversal_zip_is_rejected() {
        let root = temp_root();
        let zip = make_zip(&[
            (
                "manifest.json",
                r#"{"id":"evil","name":"Evil","version":"1.0.0","entry":"index.html"}"#,
            ),
            ("../evil.txt", "pwned"),
        ]);
        assert!(install_zip(&root, &zip).is_err());
        assert!(!root.join("evil").exists());
    }

    #[test]
    fn zip_bomb_is_rejected() {
        let root = temp_root();
        // Even a small declared manifest, the oversized sibling should trip
        // the uncompressed-size bound during pass 1 (before any write).
        let zip = make_zip(&[
            (
                "manifest.json",
                r#"{"id":"bomb","name":"Bomb","version":"1.0.0","entry":"index.html"}"#,
            ),
            ("big.bin", "x".repeat(30 * 1024 * 1024).as_str()),
        ]);
        assert!(install_zip(&root, &zip).is_err());
        assert!(!root.join("bomb").exists());
    }

    #[test]
    fn bad_slug_rejected() {
        assert!(!is_valid_slug(".."));
        assert!(!is_valid_slug("a/b"));
        assert!(!is_valid_slug(""));
        assert!(is_valid_slug("my-theme_2"));
    }
}
