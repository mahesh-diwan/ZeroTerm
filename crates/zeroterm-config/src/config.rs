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
    pub ai: AiConfig,
    pub sync: SyncConfig,
    pub ssh: SshConfig,
    pub keybindings: KeybindingsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeybindingsConfig {
    pub vim_mode: bool,
    pub shift_arrows_select: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiConfig {
    pub endpoint: String,
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

        if let Ok(overrides) = crate::lua::LuaEngine::evaluate(".zeroterm.lua") {
            config.apply_overrides(overrides);
        }

        Ok(config)
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
                "ai_endpoint" | "ai.endpoint" => self.ai.endpoint = value,
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
        self.ai = new_config.ai;
        self.sync = new_config.sync;
        self.ssh = new_config.ssh;
        self.keybindings = new_config.keybindings;
        Ok(())
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
}

impl Default for ColorConfig {
    fn default() -> Self {
        Self {
            foreground: "#e0e0e0".to_string(),
            background: "#1e1e1e".to_string(),
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
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
            opacity: 1.0,
        }
    }
}
