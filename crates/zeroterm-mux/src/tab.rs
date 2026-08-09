//! Tab - a tab containing multiple panes

use crate::split::SplitNode;

/// A tab owns a split tree of panes and renders full-window when it is the
/// active tab (classic terminal tabs: switching tabs swaps the whole view).
pub struct Tab {
    pub id: usize,
    pub panes: Vec<usize>,  // pane IDs in this tab
    pub tree: SplitNode,    // this tab's split tree
    pub active_pane: usize, // focused pane within this tab
}

impl Tab {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            panes: vec![],
            tree: SplitNode::Leaf(0),
            active_pane: 0,
        }
    }

    /// A tab containing a single fresh pane (its tree is just that leaf).
    pub fn with_pane(id: usize, pane: usize) -> Self {
        Self {
            id,
            panes: vec![pane],
            tree: SplitNode::Leaf(pane),
            active_pane: pane,
        }
    }
}
