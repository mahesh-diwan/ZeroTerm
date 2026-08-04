//! Settings state model + serde import/export.
//!
//! The app renders this state as an in-terminal overlay (keyboard-driven, no
//! native GUI); this crate keeps the testable model and the TOML round-trip.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

pub const FONT_SIZE_MIN: f32 = 6.0;
pub const FONT_SIZE_MAX: f32 = 96.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingEntry {
    /// Stable dotted key, matching `Config::apply_overrides` ("font.size", ...).
    pub key: String,
    /// Current value rendered in the overlay.
    pub value: String,
}

#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    pub open: bool,
    pub focused: usize,
    pub items: Vec<SettingEntry>,
}

impl SettingsState {
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if self.open {
            self.focused = 0;
        }
    }

    pub fn focus_next(&mut self) {
        if !self.items.is_empty() {
            self.focused = (self.focused + 1) % self.items.len();
        }
    }

    pub fn focus_prev(&mut self) {
        if !self.items.is_empty() {
            self.focused = (self.focused + self.items.len() - 1) % self.items.len();
        }
    }

    /// Replace the item list, keeping the focused index (wrapped into range).
    pub fn refresh(&mut self, items: Vec<SettingEntry>) {
        self.items = items;
        self.focused = if self.items.is_empty() {
            0
        } else {
            self.focused.min(self.items.len() - 1)
        };
    }

    pub fn focused_item(&self) -> Option<&SettingEntry> {
        self.items.get(self.focused)
    }

    pub fn clamp_font_size(size: f32) -> f32 {
        size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX)
    }

    /// Export `config` as pretty TOML to a sibling of the config path
    /// (`config.export.toml`), returning the written path.
    pub fn export_config(config: &Config, path: Option<&Path>) -> Result<PathBuf, anyhow::Error> {
        let out = path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::export_path);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&out, toml::to_string_pretty(config)?)?;
        Ok(out)
    }

    /// Read a TOML file back into a `Config` (for import).
    pub fn import_config(path: &Path) -> Result<Config, anyhow::Error> {
        let contents = fs::read_to_string(path)?;
        Ok(toml::from_str(&contents)?)
    }

    /// Path import/export both default to: `<config dir>/config.export.toml`.
    pub fn export_path() -> PathBuf {
        Config::default_config_path().with_extension("export.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_items() -> Vec<SettingEntry> {
        vec![
            SettingEntry {
                key: "font.size".into(),
                value: "14.0".into(),
            },
            SettingEntry {
                key: "colors.theme".into(),
                value: "tokyo-night".into(),
            },
            SettingEntry {
                key: "window.opacity".into(),
                value: "1.0".into(),
            },
        ]
    }

    #[test]
    fn settings_list_stable_across_refresh() {
        let mut s = SettingsState::default();
        s.refresh(sample_items());
        let keys: Vec<&str> = s.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["font.size", "colors.theme", "window.opacity"]);
        s.refresh(sample_items());
        let keys: Vec<&str> = s.items.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["font.size", "colors.theme", "window.opacity"]);
        assert_eq!(s.focused, 0);
    }

    #[test]
    fn focus_wraps_and_clamps_on_shrink() {
        let mut s = SettingsState::default();
        s.refresh(sample_items());
        s.focus_next();
        assert_eq!(s.focused, 1);
        s.focus_prev();
        assert_eq!(s.focused, 0);
        s.focus_prev();
        assert_eq!(s.focused, 2);
        s.focused = 5;
        s.refresh(vec![sample_items()[0].clone()]);
        assert_eq!(s.focused, 0);
        s.refresh(Vec::new());
        assert_eq!(s.focused, 0);
    }

    #[test]
    fn font_size_clamped_to_bounds() {
        assert_eq!(SettingsState::clamp_font_size(2.0), FONT_SIZE_MIN);
        assert_eq!(SettingsState::clamp_font_size(999.0), FONT_SIZE_MAX);
        assert_eq!(SettingsState::clamp_font_size(14.0), 14.0);
    }

    #[test]
    fn toggle_opens_with_focus_reset() {
        let mut s = SettingsState::default();
        s.refresh(sample_items());
        s.focus_next();
        s.toggle();
        assert!(s.open);
        assert_eq!(s.focused, 0);
        s.toggle();
        assert!(!s.open);
    }

    #[test]
    fn import_export_round_trips_through_toml() {
        let path =
            std::env::temp_dir().join(format!("zeroterm-gui-{}.export.toml", std::process::id()));
        let mut config = Config::default();
        config.font.size = 22.0;
        config.colors.theme = "gruvbox".into();
        config.keybindings.vim_mode = true;
        let written = SettingsState::export_config(&config, Some(&path)).unwrap();
        assert_eq!(written, path);
        let imported = SettingsState::import_config(&path).unwrap();
        assert_eq!(imported.font.size, 22.0);
        assert_eq!(imported.colors.theme, "gruvbox");
        assert!(imported.keybindings.vim_mode);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn export_uses_default_sibling_path() {
        assert_eq!(
            SettingsState::export_path().file_name().unwrap(),
            "config.export.toml"
        );
    }
}
