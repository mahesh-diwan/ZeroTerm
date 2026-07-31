use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use zeroterm_config::Config;

use crate::PaneState;

#[derive(Serialize, Deserialize)]
pub struct SessionRecord {
    pub title: String,
    pub cmd: String,
    pub cwd: String,
}

pub fn session_file_path() -> std::path::PathBuf {
    Config::default_config_path().with_file_name("session.json")
}

pub fn save_session(path: &Path, panes: &HashMap<usize, PaneState>) -> Result<()> {
    let cwd = std::env::current_dir()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    let records: Vec<SessionRecord> = panes
        .values()
        .map(|p| SessionRecord {
            title: p.title.clone(),
            cmd: p.pane_cmd.clone(),
            cwd: cwd.clone(),
        })
        .collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&records)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_session(path: &Path) -> Option<Vec<SessionRecord>> {
    let contents = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&contents) {
        Ok(records) => Some(records),
        Err(e) => {
            warn!("Failed to parse session file: {}", e);
            None
        }
    }
}
