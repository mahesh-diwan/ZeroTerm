use zeroterm_core::screen::{CommandBlock, Screen};

#[cfg(feature = "plugins")]
use std::collections::HashMap;
#[cfg(feature = "plugins")]
use std::path::Path;
#[cfg(feature = "plugins")]
use tracing::{error, info, warn};
#[cfg(feature = "plugins")]
use zeroterm_plugin::{Plugin, PluginConfig, PluginHost};

/// Extract the plain text of a command block (its output rows) for copy.
pub fn block_output_text(screen: &Screen, block: &CommandBlock) -> String {
    let buffer = screen.buffer();
    let last = buffer.len().saturating_sub(1);
    let end = block
        .end_line
        .map_or(last, |e| e.saturating_sub(1))
        .min(last);
    let start = block.start_line.min(end);
    let mut text = String::new();
    for row in start..=end {
        if let Some(line) = buffer.get(row) {
            for cell in line {
                text.push(cell.ch);
            }
        }
        text.push('\n');
    }
    text.trim_end().to_string()
}

/// Load every `*.wasm` file from `plugins_dir` into a sandboxed [`Plugin`],
/// keyed by file stem. Load failures are logged, never fatal.
#[cfg(feature = "plugins")]
pub fn load_plugins(plugins_dir: &Path) -> HashMap<String, Plugin> {
    let mut plugins = HashMap::new();
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return plugins;
    };
    let host = match PluginHost::new() {
        Ok(host) => host,
        Err(e) => {
            warn!("Plugin host init failed: {}", e);
            return plugins;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let config = PluginConfig {
            name: name.clone(),
            ..PluginConfig::default()
        };
        match host.load(&path, config) {
            Ok(plugin) => {
                info!("Loaded plugin `{}`", name);
                plugins.insert(name, plugin);
            }
            Err(e) => error!("Failed to load plugin `{}`: {}", name, e),
        }
    }
    plugins
}
