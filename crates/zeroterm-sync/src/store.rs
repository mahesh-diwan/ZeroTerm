use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocEntry {
    pub rev: u64,
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

pub struct DocStore {
    docs: HashMap<String, Vec<DocEntry>>,
}

impl DocStore {
    pub fn new() -> Self {
        Self {
            docs: HashMap::new(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.entries(name)
            .iter()
            .max_by_key(|e| (e.rev, e.timestamp_ms))
            .map(|e| e.data.as_slice())
    }

    pub fn rev(&self, name: &str) -> u64 {
        self.entries(name).iter().map(|e| e.rev).max().unwrap_or(0)
    }

    pub fn entries(&self, name: &str) -> &[DocEntry] {
        self.docs.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn put(&mut self, name: &str, data: &[u8]) {
        let rev = self.rev(name) + 1;
        let entry = DocEntry {
            rev,
            data: data.to_vec(),
            timestamp_ms: now_ms(),
        };
        self.docs.entry(name.to_string()).or_default().push(entry);
    }

    pub fn replace(&mut self, name: &str, entries: Vec<DocEntry>) {
        if entries.is_empty() {
            self.docs.remove(name);
        } else {
            self.docs.insert(name.to_string(), entries);
        }
    }
}

impl Default for DocStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Last-writer-wins merge. First dedupe by `rev` (same rev → newer timestamp
/// wins), then by `timestamp_ms` (same timestamp → higher rev wins). Disjoint
/// revision histories on both sides are preserved. Result is sorted by `rev`.
pub fn merge(server: &[DocEntry], local: &[DocEntry]) -> Vec<DocEntry> {
    let mut by_rev: HashMap<u64, DocEntry> = HashMap::new();
    for e in server.iter().chain(local) {
        match by_rev.get(&e.rev) {
            Some(existing) if existing.timestamp_ms > e.timestamp_ms => {}
            _ => {
                by_rev.insert(e.rev, e.clone());
            }
        }
    }
    let mut by_ts: HashMap<u64, DocEntry> = HashMap::new();
    for e in by_rev.into_values() {
        match by_ts.get(&e.timestamp_ms) {
            Some(existing) if existing.rev >= e.rev => {}
            _ => {
                by_ts.insert(e.timestamp_ms, e);
            }
        }
    }
    let mut merged: Vec<DocEntry> = by_ts.into_values().collect();
    merged.sort_by_key(|e| e.rev);
    merged
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
