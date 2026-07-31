use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use zeroterm_config::Config;
use zeroterm_mux::split::SplitNode;

use crate::PaneState;

#[derive(Serialize, Deserialize)]
pub struct SessionRecord {
    pub title: String,
    pub cmd: String,
    pub cwd: String,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    records: Vec<SessionRecord>,
    layout: Option<SplitNode>,
}

pub fn session_file_path() -> std::path::PathBuf {
    Config::default_config_path().with_file_name("session.json")
}

pub fn save_session(
    path: &Path,
    panes: &HashMap<usize, PaneState>,
    layout: Option<&SplitNode>,
) -> Result<()> {
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
    let json = serde_json::to_string_pretty(&SessionFile {
        records,
        layout: layout.cloned(),
    })?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn load_session(path: &Path) -> Option<(Vec<SessionRecord>, Option<SplitNode>)> {
    let contents = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse session file: {}", e);
            return None;
        }
    };
    if value.is_array() {
        // legacy file: plain records array, no layout
        match serde_json::from_value::<Vec<SessionRecord>>(value) {
            Ok(records) => Some((records, None)),
            Err(e) => {
                warn!("Failed to parse legacy session file: {}", e);
                None
            }
        }
    } else {
        match serde_json::from_value::<SessionFile>(value) {
            Ok(file) => Some((file.records, file.layout)),
            Err(e) => {
                warn!("Failed to parse session file: {}", e);
                None
            }
        }
    }
}
