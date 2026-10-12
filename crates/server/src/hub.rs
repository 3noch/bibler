//! Per-space fan-out to that space's open sockets (`tokio::sync::broadcast`): committed changes (published on the
//! store's writer thread, in seq order) and kicks (a member's role changed or they left: their sockets close).

use ops::Change;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const CAPACITY: usize = 256;

#[derive(Debug, Clone)]
pub enum Event {
    Change(Arc<Change>),
    /// Close this owner's sockets.
    Kick(String),
}

#[derive(Default)]
pub struct Hub {
    spaces: Mutex<HashMap<String, broadcast::Sender<Event>>>,
}

impl Hub {
    pub fn subscribe(&self, space: &str) -> broadcast::Receiver<Event> {
        let mut spaces = self.spaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        spaces.entry(space.to_string()).or_insert_with(|| broadcast::channel(CAPACITY).0).subscribe()
    }

    pub fn publish(&self, space: &str, change: &Change) {
        self.send(space, Event::Change(Arc::new(change.clone())));
    }

    pub fn kick(&self, space: &str, owner: &str) {
        self.send(space, Event::Kick(owner.to_string()));
    }

    fn send(&self, space: &str, event: Event) {
        let mut spaces = self.spaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(tx) = spaces.get(space) {
            if tx.send(event).is_err() {
                spaces.remove(space); // nobody listening any more
            }
        }
    }
}
