use crate::crypto::CryptoKey;
use crate::store::{merge, DocEntry, DocStore};

pub type SyncError = anyhow::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum SyncResult {
    NoChange,
    Updated,
}

pub struct SyncClient {
    url: String,
    key: CryptoKey,
    http: reqwest::Client,
}

impl SyncClient {
    pub fn new(url: String, key: CryptoKey) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            key,
            http: reqwest::Client::new(),
        }
    }

    pub async fn push(&self, name: &str, entries: &[DocEntry]) -> Result<(), SyncError> {
        let mut wire = Vec::with_capacity(entries.len());
        for e in entries {
            wire.push(DocEntry {
                rev: e.rev,
                data: self.key.encrypt(&e.data)?,
                timestamp_ms: e.timestamp_ms,
            });
        }
        let url = format!("{}/docs/{name}", self.url);
        self.http
            .post(&url)
            .json(&wire)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn pull(&self, name: &str) -> Result<Vec<DocEntry>, SyncError> {
        let url = format!("{}/docs/{name}", self.url);
        let wire: Vec<DocEntry> = self
            .http
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        wire.into_iter()
            .map(|mut e| {
                e.data = self.key.decrypt(&e.data)?;
                Ok(e)
            })
            .collect()
    }

    /// Pull remote, LWW-merge with local, push merged back to the server, and
    /// write the merged result into the store. `Updated` when anything changed
    /// on either side.
    pub async fn sync(&self, store: &mut DocStore, name: &str) -> Result<SyncResult, SyncError> {
        let remote = self.pull(name).await?;
        let local = store.entries(name).to_vec();
        let merged = merge(&remote, &local);
        let updated = merged != remote;
        if updated {
            self.push(name, &merged).await?;
        }
        store.replace(name, merged);
        Ok(if updated {
            SyncResult::Updated
        } else {
            SyncResult::NoChange
        })
    }
}
