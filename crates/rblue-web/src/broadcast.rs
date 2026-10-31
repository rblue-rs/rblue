//! Bounded broadcast bus backing the adapter event stream and the
//! notification stream.
//!
//! Producers are platform callbacks that must never block, and consumers may
//! stall, so the bus is a fan-out of small bounded queues instead of an
//! unbounded channel: a slow consumer applies backpressure by losing data
//! according to the per-item [`Delivery`] class, never by growing memory.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use futures::Stream;

use crate::sync::lock;

/// Per-subscriber queue capacity. Large enough for scan bursts and
/// notification bursts, small enough that a stalled consumer cannot inflate
/// memory.
const SUBSCRIBER_CAPACITY: usize = 64;

/// Delivery guarantee of a published item under backpressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// May be dropped (oldest first) when a subscriber's queue is full.
    /// Intermediate sightings and notification payloads use this class.
    BestEffort,
    /// Outlives best-effort items in a full queue: those are evicted first.
    /// Terminal state changes (connect/disconnect/power state) use this class
    /// so a stalled consumer still learns about state transitions.
    Guaranteed,
}

struct Subscriber<T> {
    id: u64,
    queue: VecDeque<(T, Delivery)>,
    capacity: usize,
    waker: Option<Waker>,
}

struct Shared<T> {
    next_id: u64,
    subscribers: Vec<Subscriber<T>>,
}

/// Broadcast bus with one bounded, drop-oldest queue per subscriber.
pub(crate) struct BroadcastBus<T> {
    shared: Arc<Mutex<Shared<T>>>,
}

impl<T> Clone for BroadcastBus<T> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<T> Default for BroadcastBus<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> BroadcastBus<T> {
    /// Creates an empty bus.
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                next_id: 0,
                subscribers: Vec::new(),
            })),
        }
    }
}

impl<T: Clone> BroadcastBus<T> {
    /// Publishes `item` to every current subscriber.
    ///
    /// A full subscriber queue evicts items instead of blocking the producer:
    /// [`Delivery::BestEffort`] drops the oldest entry, [`Delivery::Guaranteed`]
    /// first evicts the oldest best-effort entry and only then the oldest
    /// guaranteed one, keeping the queue bounded in every case.
    pub(crate) fn publish(&self, item: T, delivery: Delivery) {
        let mut shared = lock(&self.shared);
        for subscriber in &mut shared.subscribers {
            let queue = &mut subscriber.queue;
            if queue.len() >= subscriber.capacity {
                let evicted = match delivery {
                    Delivery::BestEffort => queue.pop_front().is_some(),
                    Delivery::Guaranteed => {
                        match queue
                            .iter()
                            .position(|(_, class)| *class == Delivery::BestEffort)
                        {
                            Some(index) => queue.remove(index).is_some(),
                            None => queue.pop_front().is_some(),
                        }
                    }
                };
                debug_assert!(evicted, "a full queue always has an entry to evict");
            }
            queue.push_back((item.clone(), delivery));
            if let Some(waker) = subscriber.waker.take() {
                waker.wake();
            }
        }
    }

    /// Subscribes with an empty queue.
    pub(crate) fn subscribe(&self) -> BusStream<T> {
        self.subscribe_with([])
    }

    /// Subscribes, delivering `snapshot` before any event published after the
    /// call. This implements the snapshot-then-live semantics of the adapter
    /// event stream.
    pub(crate) fn subscribe_with<I>(&self, snapshot: I) -> BusStream<T>
    where
        I: IntoIterator<Item = T>,
    {
        let mut shared = lock(&self.shared);
        let id = shared.next_id;
        shared.next_id += 1;
        let mut queue = VecDeque::with_capacity(SUBSCRIBER_CAPACITY);
        for item in snapshot {
            if queue.len() == SUBSCRIBER_CAPACITY {
                queue.pop_front();
            }
            queue.push_back((item, Delivery::BestEffort));
        }
        shared.subscribers.push(Subscriber {
            id,
            queue,
            capacity: SUBSCRIBER_CAPACITY,
            waker: None,
        });
        BusStream {
            shared: Arc::clone(&self.shared),
            id,
        }
    }

    /// Number of live subscribers; a testing seam for the unsubscribe path.
    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        lock(&self.shared).subscribers.len()
    }
}

/// The `Stream` handle handed out by [`BroadcastBus::subscribe`]. Dropping it
/// unsubscribes.
pub(crate) struct BusStream<T> {
    shared: Arc<Mutex<Shared<T>>>,
    id: u64,
}

impl<T> Drop for BusStream<T> {
    fn drop(&mut self) {
        lock(&self.shared).subscribers.retain(|s| s.id != self.id);
    }
}

impl<T> Stream for BusStream<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let this = self.get_mut();
        let mut shared = lock(&this.shared);
        match shared.subscribers.iter_mut().find(|s| s.id == this.id) {
            Some(subscriber) => match subscriber.queue.pop_front() {
                Some((item, _)) => Poll::Ready(Some(item)),
                None => {
                    subscriber.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            },
            // Unreachable through public use: streams only see `None` if the
            // bus forgot them, which cannot happen while they are alive.
            None => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::task::{self, ArcWake};

    use super::*;

    fn noop_context() -> Context<'static> {
        Context::from_waker(futures::task::noop_waker_ref())
    }

    fn ready<T>(poll: Poll<Option<T>>) -> Option<T> {
        match poll {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("expected the stream to be ready"),
        }
    }

    struct CountingWake(AtomicUsize);

    impl ArcWake for CountingWake {
        fn wake_by_ref(arc: &Arc<Self>) {
            arc.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn delivers_to_every_subscriber() {
        let bus = BroadcastBus::new();
        let mut first = std::pin::pin!(bus.subscribe());
        let mut second = std::pin::pin!(bus.subscribe());
        let mut cx = noop_context();

        bus.publish(7, Delivery::BestEffort);
        bus.publish(9, Delivery::Guaranteed);

        assert_eq!(ready(first.as_mut().poll_next(&mut cx)), Some(7));
        assert_eq!(ready(second.as_mut().poll_next(&mut cx)), Some(7));
        assert_eq!(ready(first.as_mut().poll_next(&mut cx)), Some(9));
        assert_eq!(ready(second.as_mut().poll_next(&mut cx)), Some(9));
        assert!(first.as_mut().poll_next(&mut cx).is_pending());
    }

    #[test]
    fn prefills_snapshot_before_live_events() {
        let bus = BroadcastBus::new();
        let mut stream = std::pin::pin!(bus.subscribe_with([1u8, 2]));
        let mut cx = noop_context();

        bus.publish(3, Delivery::BestEffort);

        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(1));
        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(2));
        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(3));
        assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    }

    #[test]
    fn best_effort_drops_oldest_when_full() {
        let bus = BroadcastBus::new();
        let mut stream = std::pin::pin!(bus.subscribe());
        let mut cx = noop_context();

        for value in 0..(SUBSCRIBER_CAPACITY as u32 * 2) {
            bus.publish(value, Delivery::BestEffort);
        }

        for value in (SUBSCRIBER_CAPACITY as u32)..(SUBSCRIBER_CAPACITY as u32 * 2) {
            assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(value));
        }
        assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    }

    #[test]
    fn guaranteed_evicts_best_effort_first() {
        let bus = BroadcastBus::new();
        let mut stream = std::pin::pin!(bus.subscribe());
        let mut cx = noop_context();

        for value in 0..(SUBSCRIBER_CAPACITY as u32) {
            bus.publish(value, Delivery::BestEffort);
        }
        bus.publish(1_000, Delivery::Guaranteed);

        // The oldest best-effort item (0) was evicted to make room; the
        // guaranteed item is queued last and delivered after the survivors.
        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(1));
        let mut last = 1;
        for _ in 1..SUBSCRIBER_CAPACITY {
            last = ready(stream.as_mut().poll_next(&mut cx)).expect("item");
        }
        assert_eq!(last, 1_000);
        assert!(stream.as_mut().poll_next(&mut cx).is_pending());
    }

    #[test]
    fn guaranteed_evicts_oldest_guaranteed_when_saturated() {
        let bus = BroadcastBus::new();
        let mut stream = std::pin::pin!(bus.subscribe());
        let mut cx = noop_context();

        for value in 0..(SUBSCRIBER_CAPACITY as u32) {
            bus.publish(value, Delivery::Guaranteed);
        }
        bus.publish(1_000, Delivery::Guaranteed);

        // No best-effort victim exists, so the oldest guaranteed item goes.
        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(1));
        let mut last = 1;
        for _ in 1..SUBSCRIBER_CAPACITY {
            last = ready(stream.as_mut().poll_next(&mut cx)).expect("item");
        }
        assert_eq!(last, 1_000);
    }

    #[test]
    fn dropping_stream_unsubscribes() {
        let bus = BroadcastBus::new();
        {
            let _stream = bus.subscribe();
            assert_eq!(bus.subscriber_count(), 1);
        }

        assert_eq!(bus.subscriber_count(), 0);
        bus.publish(1, Delivery::BestEffort);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn wakes_a_waiting_subscriber() {
        let bus = BroadcastBus::new();
        let mut stream = std::pin::pin!(bus.subscribe());
        let wake = Arc::new(CountingWake(AtomicUsize::new(0)));
        let waker = task::waker(Arc::clone(&wake));
        let mut cx = Context::from_waker(&waker);

        assert!(stream.as_mut().poll_next(&mut cx).is_pending());
        assert_eq!(wake.0.load(Ordering::Relaxed), 0);

        bus.publish(1, Delivery::BestEffort);
        assert!(wake.0.load(Ordering::Relaxed) >= 1);
        assert_eq!(ready(stream.as_mut().poll_next(&mut cx)), Some(1));
    }
}
