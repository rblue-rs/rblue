//! Bounded, runtime-agnostic broadcast queues.
//!
//! Streams on OHOS are consumed by Rust workers and (via the napi facade) by
//! JS callbacks, so a chatty peripheral must not be able to grow a subscriber
//! queue without bound. Each subscriber owns a bounded `VecDeque`; on
//! overflow the queue drops the oldest *droppable* item (for adapter events
//! only intermediate `DeviceUpdated` values are droppable, never terminal
//! ones such as `DeviceDisconnected`) and otherwise drops the incoming item.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};

use futures::stream::Stream;

struct QueueState<T> {
    items: VecDeque<T>,
    waker: Option<Waker>,
}

impl<T> Default for QueueState<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
            waker: None,
        }
    }
}

impl<T> QueueState<T> {
    /// Pushes `item`, applying the overflow policy: when full, drop the
    /// oldest item for which `droppable` returns `true`, otherwise drop the
    /// incoming item. Returns `false` when the incoming item was dropped.
    fn push(&mut self, item: T, capacity: usize, droppable: fn(&T) -> bool) -> bool {
        if self.items.len() >= capacity {
            let Some(oldest_droppable) = (0..self.items.len()).find(|i| droppable(&self.items[*i]))
            else {
                return false;
            };
            self.items.remove(oldest_droppable);
        }
        self.items.push_back(item);
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
        true
    }
}

/// Receive end of a [`BoundedBroadcast`]. Clones of the broadcast share one
/// subscriber list, while every `subscribe()` call yields an independent
/// bounded queue.
pub(crate) struct Subscription<T> {
    shared: Arc<Mutex<QueueState<T>>>,
}

impl<T> Subscription<T> {
    /// Enqueues `item` bypassing the capacity bound (used for the
    /// snapshot-then-live replay of already-known devices, which happens
    /// before any live event can reach the fresh queue).
    pub(crate) fn push_snapshot(&self, item: T) {
        let mut queue = self.lock();
        queue.items.push_back(item);
        if let Some(waker) = queue.waker.take() {
            waker.wake();
        }
    }

    fn lock(&self) -> MutexGuard<'_, QueueState<T>> {
        self.shared
            .lock()
            .expect("subscription queue poisoned by a panicking producer")
    }
}

impl<T> Stream for Subscription<T> {
    type Item = T;

    fn poll_next(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut queue = self.lock();
        match queue.items.pop_front() {
            Some(item) => Poll::Ready(Some(item)),
            None => {
                queue.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

impl<T> std::fmt::Debug for BoundedBroadcast<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundedBroadcast")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

/// Bounded broadcast with per-subscriber queues and a drop policy.
pub(crate) struct BoundedBroadcast<T> {
    capacity: usize,
    droppable: fn(&T) -> bool,
    subscribers: Mutex<Vec<Arc<Mutex<QueueState<T>>>>>,
}

impl<T: Clone> BoundedBroadcast<T> {
    /// Creates a broadcast where every subscriber keeps at most `capacity`
    /// items; on overflow `droppable` decides which item yields.
    pub(crate) fn new(capacity: usize, droppable: fn(&T) -> bool) -> Self {
        Self {
            capacity,
            droppable,
            subscribers: Mutex::new(Vec::new()),
        }
    }

    /// Opens a new subscription. The queue starts empty; a snapshot can be
    /// replayed into it with [`Subscription::push_snapshot`].
    pub(crate) fn subscribe(&self) -> Subscription<T> {
        let shared = Arc::new(Mutex::new(QueueState::default()));
        self.lock_subscribers().push(Arc::clone(&shared));
        Subscription { shared }
    }

    /// Delivers `item` to every live subscriber.
    pub(crate) fn push(&self, item: &T) {
        let mut subscribers = self.lock_subscribers();
        // A subscription holds exactly one strong reference besides the
        // subscriber list, so count == 1 means the receiver was dropped.
        subscribers.retain(|shared| Arc::strong_count(shared) > 1);
        for shared in subscribers.iter() {
            let _delivered = shared
                .lock()
                .expect("subscription queue poisoned by a panicking producer")
                .push(item.clone(), self.capacity, self.droppable);
        }
    }

    fn lock_subscribers(&self) -> MutexGuard<'_, Vec<Arc<Mutex<QueueState<T>>>>> {
        self.subscribers
            .lock()
            .expect("broadcast subscriber list poisoned by a panicking producer")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn droppable(_: &u32) -> bool {
        true
    }

    #[tokio::test]
    async fn delivers_to_every_subscriber_in_order() {
        let broadcast = BoundedBroadcast::new(8, droppable);
        let first = broadcast.subscribe();
        let second = broadcast.subscribe();
        for value in 0..4 {
            broadcast.push(&value);
        }
        for subscriber in [first, second] {
            let collected: Vec<u32> = futures::StreamExt::take(subscriber, 4)
                .collect::<Vec<_>>()
                .await;
            assert_eq!(collected, [0, 1, 2, 3]);
        }
    }

    #[tokio::test]
    async fn overflow_drops_oldest_when_droppable() {
        let broadcast = BoundedBroadcast::new(2, droppable);
        let subscriber = broadcast.subscribe();
        for value in 0..5 {
            broadcast.push(&value);
        }
        let collected: Vec<u32> = futures::StreamExt::take(subscriber, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(collected, [3, 4]);
    }

    #[tokio::test]
    async fn overflow_drops_incoming_when_nothing_is_droppable() {
        fn nothing(_: &u32) -> bool {
            false
        }
        let broadcast = BoundedBroadcast::new(2, nothing);
        let subscriber = broadcast.subscribe();
        for value in 0..5 {
            broadcast.push(&value);
        }
        let collected: Vec<u32> = futures::StreamExt::take(subscriber, 2)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(collected, [0, 1]);
    }

    #[tokio::test]
    async fn prunes_dropped_subscribers() {
        let broadcast = BoundedBroadcast::new(8, droppable);
        {
            let _transient = broadcast.subscribe();
        }
        let survivor = broadcast.subscribe();
        broadcast.push(&1);
        // The dropped subscriber must have been pruned without affecting the
        // live one (this also exercises the strong-count heuristic).
        let collected: Vec<u32> = futures::StreamExt::take(survivor, 1)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(collected, [1]);
    }

    #[tokio::test]
    async fn snapshot_push_bypasses_capacity() {
        let broadcast: BoundedBroadcast<u32> = BoundedBroadcast::new(2, droppable);
        let subscriber = broadcast.subscribe();
        for value in 0..5 {
            subscriber.push_snapshot(value);
        }
        let collected: Vec<u32> = futures::StreamExt::take(subscriber, 5)
            .collect::<Vec<_>>()
            .await;
        assert_eq!(collected, [0, 1, 2, 3, 4]);
    }
}
