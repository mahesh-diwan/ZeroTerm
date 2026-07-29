//! Sync daemon for self-hosted encrypted sync

pub struct SyncDaemon {
    pub server_url: String,
}

impl SyncDaemon {
    pub fn new(server_url: String) -> Self {
        Self { server_url }
    }

    pub async fn sync(&self) -> Result<(), anyhow::Error> {
        // TODO: Implement sync logic
        Ok(())
    }
}