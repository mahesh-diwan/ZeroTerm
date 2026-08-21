//! Tab - a tab containing multiple panes

use crate::split::SplitNode;

/// A winlink connects a tab index to a shared Window (its pane set and split tree).
/// The same Window can appear in multiple tabs with different indices.
#[derive(Debug, Clone)]
pub struct Winlink {
    /// The window (pane set + split tree) this link points to.
    pub window_id: usize,
    /// The tab index (position in the status bar) for this link.
    pub tab_index: usize,
    /// Whether this is the active link for its window.
    pub active: bool,
}

/// A tab owns a split tree of panes and renders full-window when it is the
/// active tab (classic terminal tabs: switching tabs swaps the whole view).
pub struct Tab {
    pub id: usize,
    pub panes: Vec<usize>,  // pane IDs in this tab
    pub tree: SplitNode,    // this tab's split tree
    pub active_pane: usize, // focused pane within this tab
    /// Other tab IDs that share this tab's window (pane set + split tree).
    /// Empty for standalone tabs.
    pub shared_with: Vec<usize>,
}

impl Tab {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            panes: vec![],
            tree: SplitNode::Leaf(0),
            active_pane: 0,
            shared_with: vec![],
        }
    }

    /// A tab containing a single fresh pane (its tree is just that leaf).
    pub fn with_pane(id: usize, pane: usize) -> Self {
        Self {
            id,
            panes: vec![pane],
            tree: SplitNode::Leaf(pane),
            active_pane: pane,
            shared_with: vec![],
        }
    }
}
