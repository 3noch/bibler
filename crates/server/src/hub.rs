//! Per-space fan-out to that space's open sockets (`tokio::sync::broadcast`): committed changes (published on the
//! store's writer thread, in seq order) and kicks (a member's role changed or they left: their sockets close).
//! A space has a channel only while some socket is subscribed.

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
    /// This owner's reading history changed (`space::put_history`): their sockets pass it on.
    History(String, Arc<Vec<space::HistoryEntry>>),
}

#[derive(Default)]
pub struct Hub {
    spaces: Mutex<HashMap<String, broadcast::Sender<Event>>>,
}

/// A socket's subscription; the space's channel is freed when its last subscription drops.
pub struct Subscription<'a> {
    rx: Option<broadcast::Receiver<Event>>,
    hub: &'a Hub,
    space: String,
}

impl Subscription<'_> {
    /// The next event, as `broadcast::Receiver::recv`.
    ///
    /// # Errors
    /// `Lagged` if this socket fell behind the channel; `Closed` never in practice (the hub keeps the sender).
    pub async fn recv(&mut self) -> Result<Event, broadcast::error::RecvError> {
        match &mut self.rx {
            Some(rx) => rx.recv().await,
            None => Err(broadcast::error::RecvError::Closed),
        }
    }
}

impl Drop for Subscription<'_> {
    fn drop(&mut self) {
        drop(self.rx.take());
        // Under the lock, so a subscribe in between keeps the channel (it counts as a receiver).
        let mut spaces = self.hub.lock();
        if spaces.get(&self.space).is_some_and(|tx| tx.receiver_count() == 0) {
            spaces.remove(&self.space);
        }
    }
}

impl Hub {
    pub fn subscribe(&self, space: &str) -> Subscription<'_> {
        let rx = self.lock().entry(space.to_string()).or_insert_with(|| broadcast::channel(CAPACITY).0).subscribe();
        Subscription { rx: Some(rx), hub: self, space: space.to_string() }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, broadcast::Sender<Event>>> {
        self.spaces.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn publish(&self, space: &str, change: &Change) {
        self.send(space, Event::Change(Arc::new(change.clone())));
    }

    pub fn kick(&self, space: &str, owner: &str) {
        self.send(space, Event::Kick(owner.to_string()));
    }

    pub fn history(&self, space: &str, owner: &str, entries: Vec<space::HistoryEntry>) {
        self.send(space, Event::History(owner.to_string(), Arc::new(entries)));
    }

    fn send(&self, space: &str, event: Event) {
        let mut spaces = self.lock();
        if let Some(tx) = spaces.get(space) {
            if tx.send(event).is_err() {
                spaces.remove(space); // nobody listening any more
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, Hub};

    #[tokio::test]
    async fn a_space_has_a_channel_only_while_subscribed() {
        let hub = Hub::default();
        let channels = || hub.lock().len();
        let a = hub.subscribe("s");
        let mut b = hub.subscribe("s");
        let other = hub.subscribe("t");
        assert_eq!(channels(), 2);
        drop(a);
        assert_eq!(channels(), 2, "b still listens");
        hub.kick("s", "x");
        assert!(matches!(b.recv().await, Ok(Event::Kick(who)) if who == "x"));
        drop(b);
        assert_eq!(channels(), 1, "s is freed");
        drop(other);
        assert_eq!(channels(), 0);
        let mut again = hub.subscribe("s");
        hub.kick("s", "y");
        assert!(matches!(again.recv().await, Ok(Event::Kick(who)) if who == "y"), "a fresh channel works");
    }
}
