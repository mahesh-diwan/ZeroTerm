use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};

use crate::store::{merge, DocEntry};

/// In-memory document server. Stored payloads are opaque to it: the client
/// encrypts each `DocEntry.data` before it ever hits the wire, so the server
/// only ever sees ciphertext (E2E).
pub struct SyncServer {
    store: Mutex<HashMap<String, Vec<DocEntry>>>,
}

impl SyncServer {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, name: &str) -> Vec<DocEntry> {
        self.store
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .unwrap_or_default()
    }

    /// Replace-or-merge: LWW-merge the incoming entries against what is
    /// already stored and return the merged result.
    pub fn post(&self, name: &str, incoming: Vec<DocEntry>) -> Vec<DocEntry> {
        let mut store = self.store.lock().unwrap();
        let existing = store.get(name).cloned().unwrap_or_default();
        let merged = merge(&existing, &incoming);
        if merged.is_empty() {
            store.remove(name);
        } else {
            store.insert(name.to_string(), merged.clone());
        }
        merged
    }
}

impl Default for SyncServer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn router(server: Arc<SyncServer>) -> Router {
    Router::new()
        .route("/docs/:name", get(handle_get).post(handle_post))
        .with_state(server)
}

async fn handle_get(
    State(server): State<Arc<SyncServer>>,
    Path(name): Path<String>,
) -> Json<Vec<DocEntry>> {
    Json(server.get(&name))
}

async fn handle_post(
    State(server): State<Arc<SyncServer>>,
    Path(name): Path<String>,
    Json(incoming): Json<Vec<DocEntry>>,
) -> Json<Vec<DocEntry>> {
    Json(server.post(&name, incoming))
}

/// Bind an HTTP listener at `addr` and serve the sync API until the process
/// exits. Run with `cargo run -p zeroterm-sync` once wired, or embed the
/// `router()` in an existing axum app.
pub async fn run(addr: &str) -> Result<(), anyhow::Error> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let server = Arc::new(SyncServer::new());
    axum::serve(listener, router(server)).await?;
    Ok(())
}
