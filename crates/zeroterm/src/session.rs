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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn pane(id: usize) -> (usize, PaneState) {
        let (_, rx) = mpsc::channel::<Vec<u8>>();
        (
            id,
            PaneState {
                parser: zeroterm_core::Parser::new(10, 5),
                pty_rx: rx,
                pty_tx: mpsc::channel::<crate::PtyCommand>().0,
                title: format!("pane {}", id),
                pane_cmd: "/bin/sh".into(),
                pty_dead: false,
                last_resize: None,
                bell_rung: false,
            },
        )
    }

    fn temp_session(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zt-session-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_round_trip_preserves_records_and_layout() {
        let mut panes = HashMap::new();
        for id in [1, 2, 3] {
            let (k, p) = pane(id);
            panes.insert(k, p);
        }
        let layout = SplitNode::from_ids(&[1, 2, 3]);
        let dir = temp_session("roundtrip");
        let path = dir.join("session.json");
        save_session(&path, &panes, Some(&layout)).unwrap();
        let (records, loaded) = load_session(&path).expect("session must load");
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|r| r.cmd == "/bin/sh"));
        let mut titles: Vec<&str> = records.iter().map(|r| r.title.as_str()).collect();
        titles.sort_unstable();
        assert_eq!(titles, vec!["pane 1", "pane 2", "pane 3"]);
        let loaded = loaded.expect("layout must round-trip");
        assert_eq!(
            serde_json::to_string(&layout).unwrap(),
            serde_json::to_string(&loaded).unwrap()
        );
        assert_eq!(loaded.leaves(), layout.leaves());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_missing_corrupt_and_forward_compat_files() {
        let dir = temp_session("load");
        assert!(
            load_session(&dir.join("missing.json")).is_none(),
            "missing file must yield None, not panic"
        );
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, b"{not valid json").unwrap();
        assert!(load_session(&corrupt).is_none());

        // Unknown top-level/record fields are ignored (forward compatibility).
        let future = dir.join("future.json");
        std::fs::write(
            &future,
            br#"{"records":[{"title":"a","cmd":"b","cwd":"c","future_field":1}],"layout":null,"future":true}"#,
        )
        .unwrap();
        let (records, layout) = load_session(&future).expect("forward-compat file must load");
        assert_eq!(records.len(), 1);
        assert!(layout.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn load_legacy_plain_array_format() {
        let dir = temp_session("legacy");
        let path = dir.join("session.json");
        std::fs::write(&path, br#"[{"title":"a","cmd":"b","cwd":"c"}]"#).unwrap();
        let (records, layout) = load_session(&path).expect("legacy array must load");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].title, "a");
        assert!(layout.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn split_node_serde_round_trip_preserves_ratio_and_ignores_unknown() {
        let mut layout = SplitNode::from_ids(&[1, 2]);
        layout.resize_leaf(2, 0.5, 0.13333334);
        let json = serde_json::to_string(&layout).unwrap();
        let back: SplitNode = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            json,
            "f32 ratio must survive a serde_json round trip exactly"
        );
        // Externally-tagged enum: {"Leaf": 5} and {"Split": {...}} round-trip.
        let leaf: SplitNode = serde_json::from_str(r#"{"Leaf":5}"#).unwrap();
        assert_eq!(leaf.leaves(), vec![5]);
        let split: SplitNode = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&split).unwrap(), json);
    }
}
