//! Pane - a single terminal pane within a tab

pub struct Pane {
    pub id: usize,
}

impl Pane {
    pub fn new(id: usize) -> Self {
        Self { id }
    }
}
