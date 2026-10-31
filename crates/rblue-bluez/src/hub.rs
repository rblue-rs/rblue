//! Broadcast fan-out for adapter events and GATT notifications.
//
// The consumers of these items live behind the Linux-only glue (`mod adapter`
// / `mod peripheral`); on host builds only the tests below touch them, which
// is where the pure fan-out semantics get verified.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::{Mutex, MutexGuard};

use futures::stream::StreamExt;
use tokio::sync::broadcast;

/// Capacity of each adapter-event subscription, in events. A consumer that
/// falls further behind loses the oldest events (the stream skips the gap
/// silently); terminal events are only lost under sustained overload, which
/// the core value-add layer's buffering is expected to prevent.
pub(crate) const ADAPTER_EVENT_CAPACITY: usize = 64;

/// Capacity of each notification subscription, in values. Same drop-oldest
/// policy as [`ADAPTER_EVENT_CAPACITY`].
pub(crate) const NOTIFICATION_CAPACITY: usize = 32;

/// Broadcast hub: every registered sender is an independent subscription;
/// dropping the receiver retires the sender on the next publish.
pub(crate) struct EventHub {
    senders: Mutex<Vec<broadcast::Sender<rblue_api::event::AdapterEvent>>>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self {
            senders: Mutex::new(Vec::new()),
        }
    }
}

impl EventHub {
    /// Publishes `event` to every live subscription. Must be called while
    /// holding the adapter state lock so the snapshot-then-live replay can
    /// never interleave with a live publish.
    pub(crate) fn publish(&self, event: rblue_api::event::AdapterEvent) {
        self.lock()
            .retain(|sender| sender.send(event.clone()).is_ok());
    }

    /// Registers an externally created sender under the hub lock. Callers
    /// hold the adapter state lock and replay their snapshot after this
    /// returns.
    pub(crate) fn register(&self, sender: broadcast::Sender<rblue_api::event::AdapterEvent>) {
        self.lock().push(sender);
    }

    fn lock(&self) -> MutexGuard<'_, Vec<broadcast::Sender<rblue_api::event::AdapterEvent>>> {
        self.senders
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Turns a broadcast receiver into a boxed `Stream`, skipping lagged gaps.
pub(crate) fn broadcast_stream<T: Clone + Send + 'static>(
    receiver: broadcast::Receiver<T>,
) -> futures::stream::BoxStream<'static, T> {
    futures::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(value) => return Some((value, receiver)),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use rblue_api::device::{DeviceId, Rssi};
    use rblue_api::event::AdapterEvent;

    fn updated(id: u8) -> AdapterEvent {
        AdapterEvent::DeviceUpdated(
            DeviceId::Mac([id, 0, 0, 0, 0, id], rblue_api::AddressType::Public),
            Rssi::new(-40),
        )
    }

    #[tokio::test]
    async fn subscribers_receive_independent_copies() {
        let hub = EventHub::default();
        let (tx_a, mut rx_a) = broadcast::channel(8);
        let (tx_b, mut rx_b) = broadcast::channel(8);
        hub.register(tx_a);
        hub.register(tx_b);

        hub.publish(updated(1));
        assert!(matches!(
            rx_a.recv().await,
            Ok(AdapterEvent::DeviceUpdated(..))
        ));
        assert!(matches!(
            rx_b.recv().await,
            Ok(AdapterEvent::DeviceUpdated(..))
        ));
    }

    #[tokio::test]
    async fn dropped_receivers_are_retired_on_publish() {
        let hub = EventHub::default();
        let (tx, rx) = broadcast::channel(8);
        hub.register(tx);
        drop(rx);

        hub.publish(updated(1));
        assert_eq!(hub.lock().len(), 0);
    }

    #[tokio::test]
    async fn lagged_gaps_are_skipped_not_fatal() {
        let (sender, receiver) = broadcast::channel(2);
        for id in 1..=5 {
            let _ = sender.send(updated(id));
        }
        // The receiver lagged behind capacity; the stream must still yield
        // the buffered values instead of ending.
        let mut values = broadcast_stream(receiver);
        assert!(values.next().await.is_some());
        assert!(values.next().await.is_some());
    }
}
