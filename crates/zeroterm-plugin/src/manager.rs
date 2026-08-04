//! PluginManager - install / remove / list / launch lifecycle over a plugin
//! directory, plus a GitHub marketplace fetch.
//!
//! An installed plugin is a subdirectory of the manager's root containing a
//! `plugin.toml` manifest and a `<name>.wasm` module. [`PluginManager::list`]
//! scans that directory tree; [`PluginManager::install_local`] and
//! [`PluginManager::install_from_repo`] create new ones from a local file or a
//! published GitHub release; [`PluginManager::remove`] deletes one.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Plugin, PluginConfig, PluginError, PluginHost, PluginResult};

/// Parsed `plugin.toml` manifest shipped inside an installed plugin directory.
#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    repository: Option<String>,
}

/// Describes an installed plugin, as returned by [`PluginManager::list`].
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub description: String,
    pub wasm_path: PathBuf,
}

impl PluginMeta {
    /// Reads metadata from a plugin directory that contains `plugin.toml` and
    /// `<name>.wasm`. Returns `None` if either is missing or unparsable.
    pub fn from_dir(dir: &Path) -> Option<PluginMeta> {
        let manifest = fs::read_to_string(dir.join("plugin.toml")).ok()?;
        let manifest: Manifest = toml::from_str(&manifest).ok()?;
        let wasm_path = dir.join(format!("{}.wasm", manifest.name));
        if !wasm_path.is_file() {
            return None;
        }
        Some(PluginMeta {
            name: manifest.name,
            version: manifest.version,
            description: manifest.description,
            wasm_path,
        })
    }
}

/// Manages the plugins under a single plugin directory.
pub struct PluginManager {
    root: PathBuf,
}

impl PluginManager {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The default plugin directory: `<config_dir>/zeroterm/plugins`.
    pub fn default_root() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("zeroterm").join("plugins"))
    }

    /// Scans `root` for installed plugins. A missing or empty root yields an
    /// empty list; entries without a valid manifest are skipped.
    pub fn list(&self) -> Vec<PluginMeta> {
        let mut plugins = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    if let Some(meta) = PluginMeta::from_dir(&entry.path()) {
                        plugins.push(meta);
                    }
                }
            }
        }
        plugins.sort_by(|a, b| a.name.cmp(&b.name));
        plugins
    }

    /// Installs the `.wasm` at `path` into root as a new plugin, scaffolding a
    /// `plugin.toml` manifest (version defaults to `0.1.0`).
    pub fn install_local(&self, path: &Path) -> anyhow::Result<PluginMeta> {
        let bytes = fs::read(path).map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("no usable file name for {}", path.display()))?
            .to_string();
        self.install(&name, "0.1.0", None, &bytes)
    }

    /// Fetches the latest release of `github_repo`, downloads its `.wasm`
    /// asset, and installs it with the release tag as the version.
    pub fn install_from_repo(&self, github_repo: &str) -> anyhow::Result<PluginMeta> {
        let (download_url, tag) = github_latest_release(github_repo)?;
        let bytes = reqwest::blocking::Client::new()
            .get(&download_url)
            .header("User-Agent", "zeroterm")
            .send()?
            .error_for_status()?
            .bytes()?
            .to_vec();
        let name = download_url
            .rsplit('/')
            .next()
            .and_then(|n| n.strip_suffix(".wasm"))
            .filter(|s| !s.is_empty())
            .unwrap_or("plugin")
            .to_string();
        self.install(&name, &tag, Some(github_repo), &bytes)
    }

    /// Removes `root/<name>` (plugin dir and manifest) if present.
    pub fn remove(&self, name: &str) -> anyhow::Result<()> {
        let dir = self.root.join(name);
        if !dir.is_dir() {
            return Ok(());
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// Loads the installed `root/<name>` wasm into a sandboxed [`Plugin`] via
    /// a fresh [`PluginHost`].
    pub fn launch(&self, name: &str) -> PluginResult<Plugin> {
        let meta = PluginMeta::from_dir(&self.root.join(name))
            .ok_or_else(|| PluginError::Load(name.to_string(), "not installed".to_string()))?;
        let host = PluginHost::new()?;
        let config = PluginConfig {
            name: name.to_string(),
            ..PluginConfig::default()
        };
        host.load(meta.wasm_path, config)
    }

    /// Writes `<root>/<name>/<name>.wasm` + `plugin.toml` from raw bytes.
    fn install(
        &self,
        name: &str,
        version: &str,
        repository: Option<&str>,
        bytes: &[u8],
    ) -> anyhow::Result<PluginMeta> {
        let dir = self.root.join(name);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("{name}.wasm")), bytes)?;
        let manifest = Manifest {
            name: name.to_string(),
            version: version.to_string(),
            description: String::new(),
            repository: repository.map(str::to_string),
        };
        fs::write(dir.join("plugin.toml"), toml::to_string(&manifest)?)?;
        Ok(PluginMeta::from_dir(&dir).expect("just installed"))
    }
}

/// A release returned by the public GitHub API.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Fetches the latest release of `owner/repo` from GitHub and returns the
/// `browser_download_url` of its first `.wasm` asset plus the release tag.
///
/// // ponytail: anonymous against the public API (rate-limited ~60/hr, no auth),
/// // fine for an interactive marketplace install command.
pub fn github_latest_release(repo: &str) -> anyhow::Result<(String, String)> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(&url)
        .header("User-Agent", "zeroterm")
        .send()?
        .error_for_status()?;
    let release: GithubRelease = resp.json()?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".wasm"))
        .ok_or_else(|| anyhow::anyhow!("latest release has no .wasm asset"))?;
    Ok((asset.browser_download_url.clone(), release.tag_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "zeroterm-mgr-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    /// Writes a loadable wasip1 module (just an empty `_start`) to a temp file
    /// whose stem is exactly `stem`.
    fn write_wasm(stem: &str) -> PathBuf {
        let wasm = wat::parse_str(r#"(module (func (export "_start")))"#).unwrap();
        let dir = std::env::temp_dir().join(format!(
            "zeroterm-mgr-wasm-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{stem}.wasm"));
        fs::write(&path, wasm).unwrap();
        path
    }

    #[test]
    fn list_on_empty_root_returns_empty() {
        let mgr = PluginManager::new(temp_root());
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn install_local_copies_wasm_and_writes_manifest_then_list_finds_it() {
        let root = temp_root();
        let mgr = PluginManager::new(root.clone());
        let src = write_wasm("hello");
        let meta = mgr.install_local(&src).unwrap();
        let _ = fs::remove_dir_all(src.parent().unwrap());

        assert_eq!(meta.name, "hello");
        assert_eq!(meta.version, "0.1.0");
        let dir = root.join("hello");
        assert!(dir.join("hello.wasm").is_file());
        assert!(dir.join("plugin.toml").is_file());

        let list = mgr.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "hello");
        assert_eq!(list[0].version, "0.1.0");
        assert_eq!(list[0].wasm_path, dir.join("hello.wasm"));
    }

    #[test]
    fn remove_deletes_plugin_and_list_is_empty_again() {
        let root = temp_root();
        let mgr = PluginManager::new(root.clone());
        let src = write_wasm("bye");
        mgr.install_local(&src).unwrap();
        let _ = fs::remove_dir_all(src.parent().unwrap());
        assert_eq!(mgr.list().len(), 1);

        mgr.remove("bye").unwrap();
        assert!(!root.join("bye").exists());
        assert!(mgr.list().is_empty());

        // removing a non-existent plugin is a no-op, not an error
        mgr.remove("never-existed").unwrap();
    }

    #[test]
    fn launch_loads_copied_wasm_into_plugin() {
        let mgr = PluginManager::new(temp_root());
        let src = write_wasm("echo");
        mgr.install_local(&src).unwrap();
        let _ = fs::remove_dir_all(src.parent().unwrap());

        let mut plugin = mgr.launch("echo").unwrap();
        assert!(plugin.call(b"hello").is_ok());
    }

    #[test]
    fn launch_missing_plugin_is_a_load_error() {
        let mgr = PluginManager::new(temp_root());
        assert!(matches!(mgr.launch("nope"), Err(PluginError::Load(_, _))));
    }

    #[test]
    fn install_bogus_wasm_then_launch_errors_on_invalid_module() {
        let mgr = PluginManager::new(temp_root());
        // a file that is not a wasm module
        let bogus = std::env::temp_dir().join(format!(
            "zeroterm-mgr-bogus-{}-{}.wasm",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::write(&bogus, b"this is not wasm").unwrap();
        let meta = mgr.install_local(&bogus).unwrap();
        fs::remove_file(&bogus).ok();

        assert!(matches!(
            mgr.launch(&meta.name),
            Err(PluginError::Load(_, _))
        ));
    }
}
