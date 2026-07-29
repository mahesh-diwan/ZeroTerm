//! Config parsing and management

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub font: FontConfig,
    pub colors: ColorConfig,
    pub shell: ShellConfig,
    pub window: WindowConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            colors: ColorConfig::default(),
            shell: ShellConfig::default(),
            window: WindowConfig::default(),
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let config_path = if let Some(p) = path {
            p.to_path_buf()
        } else {
            Self::default_config_path()
        };

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    fn default_config_path() -> PathBuf {
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".to_string(),
            size: 14.0,
            line_height: 1.2,
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
            program: if cfg!(windows) { "cmd.exe" } else { "zsh" }.to_string(),
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
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 800,
        }
    }
}
