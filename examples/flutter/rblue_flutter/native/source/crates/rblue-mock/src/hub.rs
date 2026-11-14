//! Broadcast fan-out shared by adapter events and peripheral notifications.

use std::sync::{Mutex, MutexGuard};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};

/// Broadcast hub: every subscriber receives an independent stream of all
/// values published after it registered. Dropping the receiver unsubscribes.
///
/// The mock is a test utility with test-paced producers, so unbounded
/// channels are acceptable here; backpressure policies belong to the core
/// value-add layer layered on top.
#[derive(Debug)]
pub(crate) struct BroadcastHub<T> {
    subscribers: Mutex<Vec<UnboundedSender<T>>>,
}

impl<T> Default for BroadcastHub<T> {
    fn default() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
        }
    }
}

impl<T: Clone> BroadcastHub<T> {
    /// Publishes `value` to every live subscriber.
    pub(crate) fn publish(&self, value: T) {
        self.lock()
            .retain(|tx| tx.unbounded_send(value.clone()).is_ok());
    }

    /// Creates a channel and registers its sender under the hub lock, so no
    /// value published after the call is missed.
    pub(crate) fn subscribe(&self) -> UnboundedReceiver<T> {
        let (tx, rx) = unbounded();
        self.register(tx);
        rx
    }

    /// Registers an externally created sender (used for the
    /// snapshot-then-live subscription semantics of the adapter event stream).
    pub(crate) fn register(&self, tx: UnboundedSender<T>) {
        self.lock().push(tx);
    }

    fn lock(&self) -> MutexGuard<'_, Vec<UnboundedSender<T>>> {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
