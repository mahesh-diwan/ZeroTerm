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
        Self {
            tabs: vec![],
            next_id: 0,
        }
    }

    pub fn add_tab(&mut self, tab: tab::Tab) -> usize {
        let id = tab.id;
        self.tabs.push(tab);
        self.next_id = self.next_id.max(id + 1);
        id
    }

    pub fn remove_tab(&mut self, id: usize) {
        self.tabs.retain(|t| t.id != id);
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }
}

impl Default for TabManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
