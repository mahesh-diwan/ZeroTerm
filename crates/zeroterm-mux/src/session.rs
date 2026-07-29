//! Session - a terminal session with its own PTY

pub struct Session {
    pub id: usize,
}

impl Session {
    pub fn new(id: usize) -> Self {
        Self { id }
    }
}
