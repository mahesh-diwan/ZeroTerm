//! Tab - a tab containing multiple panes

pub struct Tab {
    pub id: usize,
    pub panes: Vec<usize>, // pane IDs
}

impl Tab {
    pub fn new(id: usize) -> Self {
        Self { id, panes: vec![] }
    }
}
