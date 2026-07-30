use std::sync::Arc;

use tokio::sync::Mutex;
use tracing::info;

use crate::crypto::CryptoKey;

pub struct SyncDaemon {
    pub server_url: String,
    key: CryptoKey,
    client: reqwest::Client,
    dirty: Arc<Mutex<bool>>,
}

impl SyncDaemon {
    pub fn new(server_url: String) -> Self {
        Self {
            server_url,
            key: CryptoKey::generate(),
            client: reqwest::Client::new(),
            dirty: Arc::new(Mutex::new(false)),
        }
    }

    pub fn mark_dirty(&self) {
        let dirty = self.dirty.clone();
        tokio::spawn(async move {
            *dirty.lock().await = true;
        });
    }

    pub async fn sync(&self) -> Result<(), anyhow::Error> {
        if self.server_url.is_empty() {
            return Ok(());
        }

        let resp = self
            .client
            .get(format!("{}/api/sync/latest", self.server_url))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        if resp.status().is_success() {
            let encrypted = resp.bytes().await?;
            if !encrypted.is_empty() {
                let decrypted = self.key.decrypt(&encrypted)?;
                info!("Sync: pulled {} bytes", decrypted.len());
            }
        }

        let settings = self.collect_settings().await?;
        let encrypted = self.key.encrypt(&settings)?;
        self.client
            .post(format!("{}/api/sync", self.server_url))
            .body(encrypted)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?;

        info!("Sync: pushed settings");
        Ok(())
    }

    async fn collect_settings(&self) -> Result<Vec<u8>, anyhow::Error> {
        let config = zeroterm_config::Config::load(None)?;
        Ok(serde_json::to_vec(&config)?)
    }
}