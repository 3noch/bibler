//! Per-space fan-out of committed changes to that space's open sockets (`tokio::sync::broadcast`).
//! `publish` is called on the store's writer thread, in seq order.

use ops::Change;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const CAPACITY: usize = 256;

#[derive(Default)]
pub struct Hub {
    spaces: Mutex<HashMap<String, broadcast::Sender<Arc<Change>>>>,
}

impl Hub {
    pub fn subscribe(&self, space: &str) -> broadcast::Receiver<Arc<Change>> {
        let mut spaces = self.spaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        spaces.entry(space.to_string()).or_insert_with(|| broadcast::channel(CAPACITY).0).subscribe()
    }

    pub fn publish(&self, space: &str, change: &Change) {
        let mut spaces = self.spaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(tx) = spaces.get(space) {
            if tx.send(Arc::new(change.clone())).is_err() {
                spaces.remove(space); // nobody listening any more
            }
        }
    }
}
