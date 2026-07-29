//! ZeroTerm Mux - Tabs, splits, session management

pub mod pane;
pub mod session;
pub mod tab;

#[allow(dead_code)]
pub struct TabManager {
    tabs: Vec<tab::Tab>,
    next_id: usize,
}

impl TabManager {
    pub fn new() -> Self {
        Self { tabs: vec![], next_id: 0 }
    }
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;