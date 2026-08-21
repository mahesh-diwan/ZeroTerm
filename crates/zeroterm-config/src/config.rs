//! Config parsing and management

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub key_path: String,
    pub auto_connect: bool,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: whoami::username(),
            key_path: String::new(),
            auto_connect: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub shell: ShellConfig,
    pub window: WindowConfig,
    #[serde(default)]
    pub cursor: CursorConfig,
    #[serde(default)]
    pub mouse: MouseConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub terminal: TerminalConfig,
    pub sync: SyncConfig,
    pub ssh: SshConfig,
    pub keybindings: KeybindingsConfig,
    #[serde(default)]
    pub features: FeaturesConfig,
}

/// Protocol / interaction features.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalConfig {
    /// Kitty keyboard protocol: advertise support and emit CSI-u sequences for
    /// modified functional keys to apps that push `CSI > 1 u` (nvim, readline
    /// 8.2+, fish, zsh). Legacy apps are unaffected — the protocol is opt-in.
    pub kitty_keyboard: bool,
    /// OSC 9 desktop notifications (emitted by `notify-send`-style shell
    /// integrations) surface as native notifications.
    pub notifications: bool,
    /// OSC 8 hyperlinks: hover shows the URL in the status bar and click opens
    /// it with the system handler (xdg-open).
    pub hyperlinks: bool,
    /// Visual bell (kitty `visual_bell_duration`): on BEL the terminal
    /// background flashes toward the selection color for this many ms, fading
    /// in and out. 0 disables. On by default so a bell is never silent.
    #[serde(default = "default_visual_bell_ms")]
    pub visual_bell_ms: u64,
    /// Hex color for the visual bell flash (e.g., "#ffffff" for white flash).
    /// Empty string means use the selection color (existing behavior).
    #[serde(default = "default_visual_bell_color")]
    pub visual_bell_color: String,
    /// Whether to also flash the tab bar on bell activity.
    #[serde(default = "default_visual_bell_tab_bar")]
    pub visual_bell_tab_bar: bool,
}

fn default_visual_bell_ms() -> u64 {
    150
}

fn default_visual_bell_color() -> String {
    String::new()
}

fn default_visual_bell_tab_bar() -> bool {
    true
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            kitty_keyboard: true,
            notifications: true,
            hyperlinks: true,
            visual_bell_ms: default_visual_bell_ms(),
            visual_bell_color: default_visual_bell_color(),
            visual_bell_tab_bar: default_visual_bell_tab_bar(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingsConfig {
    pub vim_mode: bool,
    pub shift_arrows_select: bool,
    /// Accepted for backwards compatibility but currently a no-op: sending
    /// CSI CUP to the shell at a bare prompt made readline render escape
    /// garbage on the command line, so click-to-position was removed. Apps
    /// that enable mouse tracking still get click positions via SGR sequences.
    pub click_to_position: bool,
}

impl Default for KeybindingsConfig {
    fn default() -> Self {
        Self {
            vim_mode: false,
            shift_arrows_select: true,
            click_to_position: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncConfig {
    pub server_url: String,
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let config_path = if let Some(p) = path {
            p.to_path_buf()
        } else {
            Self::default_config_path()
        };

        let mut config = if config_path.exists() {
            let contents = fs::read_to_string(&config_path)?;
            toml::from_str(&contents)?
        } else {
            Config::default()
        };

        if let Ok(overrides) = crate::lua::evaluate(".zeroterm.lua") {
            config.apply_overrides(overrides);
        }

        Ok(config)
    }

    pub fn load_async(path: Option<PathBuf>) -> (Self, std::sync::mpsc::Receiver<Self>) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Self::load(path.as_deref()).unwrap_or_default());
        });
        (Self::default(), rx)
    }

    pub fn apply_overrides(&mut self, overrides: HashMap<String, String>) {
        for (key, value) in overrides {
            match key.as_str() {
                "font_family" | "font.family" => self.font.family = Some(value),
                "font_size" | "font.size" => {
                    if let Ok(v) = value.parse::<f32>() {
                        self.font.size = v;
                    }
                }
                "line_height" | "font.line_height" => {
                    if let Ok(v) = value.parse::<f32>() {
                        self.font.line_height = v;
                    }
                }
                "font_path" | "font.path" => self.font.path = Some(value.to_string()),
                "foreground" | "colors.foreground" => self.colors.foreground = value,
                "background" | "colors.background" => self.colors.background = value,
                "shell" | "shell.program" => self.shell.program = value,
                "window_width" | "window.width" => {
                    if let Ok(v) = value.parse::<u32>() {
                        self.window.width = v;
                    }
                }
                "window_height" | "window.height" => {
                    if let Ok(v) = value.parse::<u32>() {
                        self.window.height = v;
                    }
                }
                "opacity" | "window.opacity" => {
                    if let Ok(v) = value.parse::<f64>() {
                        self.window.opacity = v;
                    }
                }
                "ssh_host" | "ssh.host" => self.ssh.host = value,
                "ssh_user" | "ssh.user" => self.ssh.user = value,
                "ssh_port" | "ssh.port" => {
                    if let Ok(v) = value.parse::<u16>() {
                        self.ssh.port = v;
                    }
                }
                "ssh_key_path" | "ssh.key_path" => self.ssh.key_path = value,
                "ssh_auto_connect" | "ssh.auto_connect" => {
                    if let Ok(v) = value.parse::<bool>() {
                        self.ssh.auto_connect = v;
                    }
                }
                "vim_mode" | "keybindings.vim_mode" => {
                    self.keybindings.vim_mode = value.parse().unwrap_or(false);
                }
                "shift_arrows_select" | "keybindings.shift_arrows_select" => {
                    self.keybindings.shift_arrows_select = value.parse().unwrap_or(true);
                }
                "click_to_position" | "keybindings.click_to_position" => {
                    self.keybindings.click_to_position = value.parse().unwrap_or(true);
                }
                "focus_follows_mouse" | "mouse.focus_follows_mouse" => {
                    self.mouse.focus_follows_mouse = value.parse().unwrap_or(false);
                }
                "restore_session" | "session.restore" => {
                    self.session.restore = value.parse().unwrap_or(false);
                }
                "kitty_keyboard" | "terminal.kitty_keyboard" => {
                    self.terminal.kitty_keyboard = value.parse().unwrap_or(true);
                }
                "notifications" | "terminal.notifications" => {
                    self.terminal.notifications = value.parse().unwrap_or(true);
                }
                "hyperlinks" | "terminal.hyperlinks" => {
                    self.terminal.hyperlinks = value.parse().unwrap_or(true);
                }
                "visual_bell_ms" | "terminal.visual_bell_ms" => {
                    self.terminal.visual_bell_ms = value.parse().unwrap_or(150);
                }
                "visual_bell_color" | "terminal.visual_bell_color" => {
                    self.terminal.visual_bell_color = value;
                }
                "visual_bell_tab_bar" | "terminal.visual_bell_tab_bar" => {
                    self.terminal.visual_bell_tab_bar = value.parse().unwrap_or(true);
                }
                _ => {}
            }
        }
    }

    pub fn default_config_path() -> PathBuf {
        if let Some(config_dir) = dirs::config_dir() {
            config_dir.join("zeroterm").join("config.toml")
        } else {
            PathBuf::from("config.toml")
        }
    }

    pub fn save(&self, path: Option<&Path>) -> Result<(), anyhow::Error> {
        let config_path = if let Some(p) = path {
            p.to_path_buf()
        } else {
            Self::default_config_path()
        };

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self)?;
        fs::write(config_path, contents)?;
        Ok(())
    }

    pub fn reload(&mut self, path: Option<&Path>) -> Result<(), anyhow::Error> {
        let new_config = Self::load(path)?;
        self.font = new_config.font;
        self.colors = new_config.colors;
        self.shell = new_config.shell;
        self.window = new_config.window;
        self.cursor = new_config.cursor;
        self.mouse = new_config.mouse;
        self.session = new_config.session;
        self.terminal = new_config.terminal;
        self.sync = new_config.sync;
        self.ssh = new_config.ssh;
        self.keybindings = new_config.keybindings;
        self.features = new_config.features;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn load_async_returns_defaults_then_hydrates_from_file() {
        let path = std::env::temp_dir().join(format!("zeroterm-test-{}.toml", std::process::id()));
        let mut expected = Config::default();
        expected.colors.foreground = "#112233".to_string();
        fs::write(&path, toml::to_string_pretty(&expected).unwrap()).unwrap();

        let (defaults, rx) = Config::load_async(Some(path.clone()));
        assert_eq!(
            defaults.colors.foreground,
            Config::default().colors.foreground
        );

        let hydrated = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(hydrated.colors.foreground, "#112233");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn window_config_defaults_are_correct() {
        let win = WindowConfig::default();
        assert_eq!(win.opacity, 1.0);
        assert!(!win.blur);
        assert_eq!(win.blur_radius, 8.0);
    }

    #[test]
    fn window_config_deserializes_with_new_keys() {
        let win: WindowConfig = toml::from_str(
            "width = 1200\nheight = 800\nopacity = 0.7\nblur = true\nblur_radius = 16.0",
        )
        .unwrap();
        assert_eq!(win.opacity, 0.7);
        assert!(win.blur);
        assert_eq!(win.blur_radius, 16.0);
    }

    #[test]
    fn window_config_without_new_keys_uses_defaults() {
        let win: WindowConfig =
            toml::from_str("width = 1200\nheight = 800\nopacity = 1.0").unwrap();
        assert!(!win.blur);
        assert_eq!(win.blur_radius, 8.0);
    }

    #[test]
    fn window_config_toml_round_trips() {
        let config = WindowConfig {
            width: 1200,
            height: 800,
            opacity: 0.7,
            blur: true,
            blur_radius: 16.0,
        };
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: WindowConfig = toml::from_str(&text).unwrap();
        assert_eq!(parsed.opacity, 0.7);
        assert!(parsed.blur);
        assert_eq!(parsed.blur_radius, 16.0);
    }

    #[test]
    fn cursor_config_defaults_are_correct() {
        let cursor = CursorConfig::default();
        assert!(cursor.blink);
        assert_eq!(cursor.blink_interval_ms, 530);
    }

    #[test]
    fn cursor_config_deserializes_with_new_keys() {
        let mut config = Config::default();
        config.cursor.blink = false;
        config.cursor.blink_interval_ms = 900;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(!parsed.cursor.blink);
        assert_eq!(parsed.cursor.blink_interval_ms, 900);
    }

    #[test]
    fn cursor_config_without_keys_uses_defaults() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("cursor");
        let parsed: Config = table.try_into().unwrap();
        assert!(parsed.cursor.blink);
        assert_eq!(parsed.cursor.blink_interval_ms, 530);
    }

    #[test]
    fn mouse_config_defaults_focus_follows_on() {
        assert!(Config::default().mouse.focus_follows_mouse);
        assert!(MouseConfig::default().focus_follows_mouse);
    }

    #[test]
    fn mouse_config_deserializes_focus_follows() {
        let mut config = Config::default();
        config.mouse.focus_follows_mouse = true;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(parsed.mouse.focus_follows_mouse);
    }

    #[test]
    fn mouse_config_without_keys_uses_defaults() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("mouse");
        let parsed: Config = table.try_into().unwrap();
        assert!(parsed.mouse.focus_follows_mouse);
    }

    #[test]
    fn terminal_config_defaults_all_on() {
        let t = TerminalConfig::default();
        assert!(t.kitty_keyboard);
        assert!(t.notifications);
        assert!(t.hyperlinks);
        assert_eq!(t.visual_bell_ms, 150);
    }

    #[test]
    fn terminal_config_absent_in_toml_uses_defaults() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("terminal");
        let parsed: Config = table.try_into().unwrap();
        assert!(parsed.terminal.kitty_keyboard);
        assert!(parsed.terminal.hyperlinks);
    }

    #[test]
    fn terminal_config_deserializes_roundtrip() {
        let mut config = Config::default();
        config.terminal.kitty_keyboard = false;
        config.terminal.notifications = false;
        config.terminal.visual_bell_ms = 400;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(!parsed.terminal.kitty_keyboard);
        assert!(!parsed.terminal.notifications);
        assert!(parsed.terminal.hyperlinks);
        assert_eq!(parsed.terminal.visual_bell_ms, 400);
    }

    #[test]
    fn terminal_config_without_new_keys_uses_defaults() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("terminal");
        let parsed: Config = table.try_into().unwrap();
        assert_eq!(parsed.terminal.visual_bell_ms, 150);
    }

    #[test]
    fn terminal_visual_bell_override_applies() {
        let mut config = Config::default();
        config.apply_overrides(HashMap::from([(
            "terminal.visual_bell_ms".to_string(),
            "0".into(),
        )]));
        assert_eq!(config.terminal.visual_bell_ms, 0);
    }

    #[test]
    fn terminal_overrides_apply_from_lua_style_keys() {
        let mut config = Config::default();
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("kitty_keyboard".to_string(), "false".to_string());
        overrides.insert("terminal.hyperlinks".to_string(), "false".to_string());
        config.apply_overrides(overrides);
        assert!(!config.terminal.kitty_keyboard);
        assert!(!config.terminal.hyperlinks);
    }

    #[test]
    fn session_restore_defaults_to_off() {
        // Fresh start is the default: restore must be off unless explicitly set.
        assert!(!Config::default().session.restore);
        assert!(!SessionConfig::default().restore);
    }

    #[test]
    fn session_config_deserializes_restore() {
        let mut config = Config::default();
        config.session.restore = true;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(parsed.session.restore);
    }

    #[test]
    fn session_config_without_keys_uses_defaults() {
        // A config with no [session] section must not reject the file: the
        // whole section is serde-optional like [cursor]/[mouse].
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("session");
        let parsed: Config = table.try_into().unwrap();
        assert!(!parsed.session.restore);
    }

    #[test]
    fn lua_override_toggles_session_restore() {
        let mut config = Config::default();
        config.apply_overrides(HashMap::from([(
            "session.restore".to_string(),
            "true".into(),
        )]));
        assert!(config.session.restore);
        config.apply_overrides(HashMap::from([(
            "restore_session".to_string(),
            "false".into(),
        )]));
        assert!(!config.session.restore);
    }

    #[test]
    fn features_config_defaults_all_true() {
        let f = FeaturesConfig::default();
        assert!(f.enable_bell);
        assert!(f.enable_scrollbar);
        assert!(f.enable_tab_bar);
        assert!(f.enable_status_bar);
        assert!(f.enable_search);
        assert!(f.enable_notifications);
    }

    #[test]
    fn features_config_absent_in_toml_uses_defaults() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        table.remove("features");
        let parsed: Config = table.try_into().unwrap();
        assert!(parsed.features.enable_bell);
        assert!(parsed.features.enable_scrollbar);
        assert!(parsed.features.enable_tab_bar);
        assert!(parsed.features.enable_status_bar);
        assert!(parsed.features.enable_search);
        assert!(parsed.features.enable_notifications);
    }

    #[test]
    fn features_config_deserializes_roundtrip() {
        let mut config = Config::default();
        config.features.enable_bell = false;
        config.features.enable_search = false;
        config.features.enable_notifications = false;
        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert!(!parsed.features.enable_bell);
        assert!(parsed.features.enable_scrollbar);
        assert!(parsed.features.enable_tab_bar);
        assert!(parsed.features.enable_status_bar);
        assert!(!parsed.features.enable_search);
        assert!(!parsed.features.enable_notifications);
    }

    #[test]
    fn features_config_partial_toml_uses_defaults_for_missing() {
        let mut table =
            toml::from_str::<toml::Table>(&toml::to_string(&Config::default()).unwrap()).unwrap();
        let features = table.get_mut("features").unwrap().as_table_mut().unwrap();
        features.insert("enable_bell".into(), toml::Value::Boolean(false));
        let parsed: Config = table.try_into().unwrap();
        assert!(!parsed.features.enable_bell);
        assert!(parsed.features.enable_scrollbar);
        assert!(parsed.features.enable_tab_bar);
        assert!(parsed.features.enable_status_bar);
        assert!(parsed.features.enable_search);
        assert!(parsed.features.enable_notifications);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    pub family: Option<String>,
    pub size: f32,
    pub line_height: f32,
    pub path: Option<String>,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 14.0,
            line_height: 1.2,
            path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorConfig {
    pub foreground: String,
    pub background: String,
    pub theme: String,
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            foreground: "#e0e0e0".to_string(),
            background: "#1e1e1e".to_string(),
            theme: "tokyo-night".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            program: if cfg!(windows) { "cmd.exe" } else { "bash" }.to_string(),
            args: if cfg!(windows) {
                vec![]
            } else {
                vec!["-l".to_string()]
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub opacity: f64,
    #[serde(default = "default_blur")]
    pub blur: bool,
    #[serde(default = "default_blur_radius")]
    pub blur_radius: f64,
}

fn default_blur() -> bool {
    false
}

fn default_blur_radius() -> f64 {
    8.0
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
            opacity: 1.0,
            blur: false,
            blur_radius: 8.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorConfig {
    #[serde(default = "default_cursor_blink")]
    pub blink: bool,
    #[serde(default = "default_cursor_blink_interval")]
    pub blink_interval_ms: u64,
}

fn default_cursor_blink() -> bool {
    true
}

fn default_cursor_blink_interval() -> u64 {
    530
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            blink: true,
            blink_interval_ms: 530,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseConfig {
    /// Hover over a pane in a split makes it the active pane. On by default.
    #[serde(default = "default_focus_follows_mouse")]
    pub focus_follows_mouse: bool,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            focus_follows_mouse: default_focus_follows_mouse(),
        }
    }
}

fn default_focus_follows_mouse() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// Restore the previous session's tabs/panes on launch. Off by default:
    /// ZeroTerm starts with a single fresh tab every time. When on, the layout
    /// is saved on close (layout.json next to config.toml) and re-spawned at
    /// init; when off, any stale layout file is discarded.
    #[serde(default = "default_session_restore")]
    pub restore: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            restore: default_session_restore(),
        }
    }
}

fn default_session_restore() -> bool {
    false
}

/// Runtime feature flags — enable/disable optional UI behaviors via `[features]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturesConfig {
    /// Terminal bell (BEL). When off, BEL is silently ignored.
    #[serde(default = "default_true")]
    pub enable_bell: bool,
    /// Vertical scrollbar overlay on the right edge.
    #[serde(default = "default_true")]
    pub enable_scrollbar: bool,
    /// Top tab bar. Hidden automatically when only one tab exists regardless
    /// of this flag (kitty `tab_bar_min_tabs` behavior).
    #[serde(default = "default_true")]
    pub enable_tab_bar: bool,
    /// Bottom status bar (active pane title, exit chip, hover URI, scroll
    /// indicator).
    #[serde(default = "default_true")]
    pub enable_status_bar: bool,
    /// Ctrl+Shift+F search overlay.
    #[serde(default = "default_true")]
    pub enable_search: bool,
    /// Desktop notifications (OSC 9). Also gated by `terminal.notifications`;
    /// both must be true for a notification to fire.
    #[serde(default = "default_true")]
    pub enable_notifications: bool,
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            enable_bell: true,
            enable_scrollbar: true,
            enable_tab_bar: true,
            enable_status_bar: true,
            enable_search: true,
            enable_notifications: true,
        }
    }
}

fn default_true() -> bool {
    true
}
