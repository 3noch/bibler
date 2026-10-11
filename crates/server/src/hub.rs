//! Per-owner fan-out of committed changes to that owner's open sockets (`tokio::sync::broadcast`).
//! `publish` is called on the store's writer thread, in seq order.

use ops::Change;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const CAPACITY: usize = 256;

#[derive(Default)]
pub struct Hub {
    owners: Mutex<HashMap<String, broadcast::Sender<Arc<Change>>>>,
}

impl Hub {
    pub fn subscribe(&self, owner: &str) -> broadcast::Receiver<Arc<Change>> {
        let mut owners = self.owners.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        owners.entry(owner.to_string()).or_insert_with(|| broadcast::channel(CAPACITY).0).subscribe()
    }

    pub fn publish(&self, owner: &str, change: &Change) {
        let mut owners = self.owners.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(tx) = owners.get(owner) {
            if tx.send(Arc::new(change.clone())).is_err() {
                owners.remove(owner); // nobody listening any more
            }
        }
    }
}
