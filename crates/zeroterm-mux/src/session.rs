//! Session - a terminal session: an id plus its serde-serializable layout.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::split::SplitNode;

/// Serializable descriptor for one pane: enough to respawn it on restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSpec {
    pub title: String,
    pub cmd: String,
    pub cwd: String,
}

/// The on-disk layout of ONE tab: its pane descriptors (in sorted-id order,
/// so tree leaf ids double as positions into `panes`), its own split tree,
/// and which pane (an index into `panes`) was active inside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabLayout {
    pub panes: Vec<PaneSpec>,
    pub split: Option<SplitNode>,
    pub active_pane: usize,
}

/// The on-disk session layout: one split tree per tab (classic tabs — each
/// tab owns its panes and renders full-window when selected) plus which tab
/// (an index into `tabs`) was active at quit time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLayout {
    pub active_tab: usize,
    pub tabs: Vec<TabLayout>,
}

impl SessionLayout {
    /// Read and parse the layout file. A missing or corrupt file yields None so
    /// the caller falls back to a single default tab. The legacy flat
    /// single-tree format (`{active_pane, panes, split}`) is upgraded in place
    /// to a one-tab layout, so old layout.json files survive.
    pub fn restore(path: &Path) -> Option<Self> {
        let contents = std::fs::read_to_string(path).ok()?;
        if let Ok(layout) = serde_json::from_str::<SessionLayout>(&contents) {
            return Some(layout);
        }
        // Legacy: a single global split tree of panes.
        #[derive(Deserialize)]
        struct Legacy {
            active_pane: usize,
            panes: Vec<PaneSpec>,
            split: Option<SplitNode>,
        }
        let legacy: Legacy = serde_json::from_str(&contents).ok()?;
        Some(SessionLayout {
            active_tab: 0,
            tabs: vec![TabLayout {
                panes: legacy.panes,
                split: legacy.split,
                active_pane: legacy.active_pane,
            }],
        })
    }

    /// Persist this layout as pretty JSON, creating the parent directory as
    /// needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    /// Map a saved split tree onto freshly assigned pane ids. Saved leaf ids
    /// are positions into `panes` (saved in sorted-id order), so
    /// `restored_ids[k]` is the new id for saved pane k. Out-of-range leaves
    /// fall back to the first restored pane so the tree stays well-formed.
    pub fn remap_split(node: &SplitNode, restored_ids: &[usize]) -> SplitNode {
        let fallback = restored_ids.first().copied().unwrap_or(0);
        match node {
            SplitNode::Leaf(k) => {
                SplitNode::Leaf(restored_ids.get(*k).copied().unwrap_or(fallback))
            }
            SplitNode::Split {
                dir,
                children,
                ratio,
            } => SplitNode::Split {
                dir: *dir,
                children: children
                    .iter()
                    .map(|c| Self::remap_split(c, restored_ids))
                    .collect(),
                ratio: *ratio,
            },
        }
    }
}

/// A terminal session: an id plus the layout needed to restore it.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: usize,
    pub layout: SessionLayout,
}

impl Session {
    pub fn new(id: usize, layout: SessionLayout) -> Self {
        Self { id, layout }
    }

    /// Persist this session's layout to `path`.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        self.layout.save(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zt-mux-session-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn pane(title: &str) -> PaneSpec {
        PaneSpec {
            title: title.into(),
            cmd: "/bin/sh".into(),
            cwd: "/tmp".into(),
        }
    }

    fn sample_layout() -> SessionLayout {
        // Two tabs, the first with a nested split tree (dragged ratio) so the
        // round trip is checked against structure, not just a flat list.
        let mut split = SplitNode::from_ids(&[0, 1, 2, 3]);
        split.resize_leaf(2, 0.5, 0.13333334);
        SessionLayout {
            active_tab: 1,
            tabs: vec![
                TabLayout {
                    panes: vec![pane("t0"), pane("t1"), pane("t2"), pane("t3")],
                    split: Some(split),
                    active_pane: 2,
                },
                TabLayout {
                    panes: vec![pane("t4")],
                    split: Some(SplitNode::Leaf(0)),
                    active_pane: 0,
                },
            ],
        }
    }

    #[test]
    fn save_restore_round_trip_preserves_layout() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("layout.json");
        let saved = sample_layout();
        saved.save(&path).unwrap();

        let restored = SessionLayout::restore(&path).expect("layout must load");
        // Exact serialized equality: tree geometry (dirs, ratios, order) and
        // pane descriptors must survive the round trip byte-for-byte.
        assert_eq!(
            serde_json::to_string(&restored).unwrap(),
            serde_json::to_string(&saved).unwrap()
        );
        assert_eq!(restored.active_tab, saved.active_tab);
        assert_eq!(restored.tabs.len(), saved.tabs.len());
        for (r, s) in restored.tabs.iter().zip(saved.tabs.iter()) {
            assert_eq!(r.active_pane, s.active_pane);
            assert_eq!(
                r.split.clone().unwrap().leaves(),
                s.split.clone().unwrap().leaves()
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn legacy_flat_layout_upgrades_to_one_tab() {
        let dir = temp_dir("legacy");
        let path = dir.join("layout.json");
        // The pre-classic-tabs format: a single global split tree.
        let legacy = serde_json::json!({
            "active_pane": 2,
            "panes": [
                {"title": "t0", "cmd": "/bin/sh", "cwd": "/tmp"},
                {"title": "t1", "cmd": "/bin/sh", "cwd": "/tmp"},
                {"title": "t2", "cmd": "/bin/zsh", "cwd": "/tmp"}
            ],
            "split": {"Leaf": 2}
        });
        std::fs::write(&path, serde_json::to_string(&legacy).unwrap()).unwrap();
        let restored = SessionLayout::restore(&path).expect("legacy layout loads");
        assert_eq!(restored.active_tab, 0);
        assert_eq!(restored.tabs.len(), 1);
        assert_eq!(restored.tabs[0].panes.len(), 3);
        assert_eq!(restored.tabs[0].active_pane, 2);
        assert_eq!(restored.tabs[0].split.clone().unwrap().leaves(), vec![2]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn restore_missing_and_corrupt_files_fall_back_to_none() {
        let dir = temp_dir("load");
        assert!(
            SessionLayout::restore(&dir.join("missing.json")).is_none(),
            "missing file must yield None, not panic"
        );
        let corrupt = dir.join("corrupt.json");
        std::fs::write(&corrupt, b"{not valid json").unwrap();
        assert!(SessionLayout::restore(&corrupt).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn remap_split_rebases_leaf_ids_onto_fresh_ids() {
        let split = SplitNode::from_ids(&[0, 1, 2, 3]);
        let remapped = SessionLayout::remap_split(&split, &[7, 9, 11, 13]);
        assert_eq!(remapped.leaves(), vec![7, 9, 11, 13]);
        // Out-of-range saved leaf ids (shouldn't happen) fall back to pane 0.
        let bad = SplitNode::Leaf(99);
        assert_eq!(SessionLayout::remap_split(&bad, &[7, 9]).leaves(), vec![7]);
        assert_eq!(
            SessionLayout::remap_split(&bad, &[]).leaves(),
            vec![0],
            "empty restore list keeps the tree well-formed"
        );
    }

    #[test]
    fn session_save_delegates_to_layout() {
        let dir = temp_dir("session");
        let path = dir.join("layout.json");
        let session = Session::new(0, sample_layout());
        session.save(&path).unwrap();
        assert!(SessionLayout::restore(&path).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
