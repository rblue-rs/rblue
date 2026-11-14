//! Per-connection GATT operation serialization with priorities.
//!
//! Backends make no cross-operation ordering promises when calls race
//! (`Peripheral` methods are safe to call concurrently, but the underlying
//! platform stacks - Android binder timing in particular - are not), so this
//! layer serializes every GATT procedure through a single worker per
//! connection and lets callers opt into priority ordering.
//!
//! Priority policy (most urgent first): subscribe/unsubscribe > write >
//! read/discover. Rationale: subscriptions gate notification flow and are
//! cheap; writes carry application payloads and may be latency-sensitive;
//! discovery and reads are one-shot setup/inspection work. Within one
//! priority class, operations run in submission order (FIFO).
//!
//! # Backpressure and cancellation semantics
//!
//! The queue is bounded ([`QueueConfig::capacity`]). `submit` suspends the
//! caller until capacity is available (bounded-channel backpressure);
//! `try_submit` fails immediately with [`Error::InvalidState`] instead. A
//! queued operation can be cancelled through its [`OpHandle`] as long as it
//! has not started executing; cancellation resolves the operation with
//! [`Error::InvalidState`] and it never reaches the peripheral. An operation
//! that already started cannot be cancelled - wait for its result.
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//! use futures::StreamExt;
//! use rblue_api::prelude::*;
//! use rblue_api::queue::{GattOpQueue, GattOperation, QueueConfig};
//!
//! # async fn demo(peripheral: Arc<dyn Peripheral>) -> Result<(), Error> {
//! let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig::default());
//! tokio::spawn(worker.run());
//!
//! let submission = queue
//!     .submit(GattOperation::discover_all())
//!     .await
//!     .expect("submit discovery");
//! let services = submission.result.await.expect("reply");
//! let _ = services?;
//! # Ok(())
//! # }
//! ```

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, StreamExt};

use crate::error::Error;
use crate::gatt::{CharacteristicId, ServiceSet, WriteMode};
use crate::peripheral::Peripheral;

/// Error resolving a cancelled operation.
///
/// [`Error::InvalidState`] is the closest entry in the unified error model:
/// the operation was never executed, so no transport-level failure occurred.
pub(crate) fn cancelled_error() -> Error {
    Error::InvalidState("gatt operation cancelled before execution".to_owned())
}

/// Error resolving an operation submitted to a queue whose worker is gone.
pub(crate) fn queue_closed_error() -> Error {
    Error::InvalidState("gatt operation queue closed".to_owned())
}

/// Scheduling class of a [`GattOperation`]; lower discriminant runs first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum GattOpPriority {
    /// CCCD management: subscribe/unsubscribe. Runs before anything else so
    /// notification flow is never delayed behind bulk transfers.
    Subscribe,
    /// Characteristic writes.
    Write,
    /// Discovery and reads: one-shot setup and inspection work.
    Discovery,
}

/// One serializable GATT procedure.
///
/// The variants mirror the [`Peripheral`] methods the value-add layer needs
/// to order; session restore re-issues these after a reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum GattOperation {
    /// [`Peripheral::discover_services`].
    Discover {
        /// Service UUID filter; empty discovers the full tree.
        filter: Vec<uuid::Uuid>,
    },
    /// [`Peripheral::read`].
    Read {
        /// Target characteristic (session-scoped identifier).
        ch: CharacteristicId,
    },
    /// [`Peripheral::write`].
    Write {
        /// Target characteristic (session-scoped identifier).
        ch: CharacteristicId,
        /// Payload; rejected with [`Error::InvalidLength`] when oversize.
        data: Vec<u8>,
        /// Acknowledged vs unacknowledged write.
        mode: WriteMode,
    },
    /// [`Peripheral::subscribe`].
    Subscribe {
        /// Target characteristic (session-scoped identifier).
        ch: CharacteristicId,
    },
    /// [`Peripheral::unsubscribe`].
    Unsubscribe {
        /// Target characteristic (session-scoped identifier).
        ch: CharacteristicId,
    },
}

impl GattOperation {
    /// Convenience constructor: full-tree discovery (`filter` empty).
    #[must_use]
    pub fn discover_all() -> Self {
        Self::Discover { filter: Vec::new() }
    }

    /// Scheduling class of this operation; see [`GattOpPriority`].
    #[must_use]
    pub const fn priority(&self) -> GattOpPriority {
        match self {
            Self::Subscribe { .. } | Self::Unsubscribe { .. } => GattOpPriority::Subscribe,
            Self::Write { .. } => GattOpPriority::Write,
            Self::Discover { .. } | Self::Read { .. } => GattOpPriority::Discovery,
        }
    }

    /// Executes the operation directly against `peripheral`, bypassing any
    /// queue. This is the single place mapping operations onto the trait
    /// surface; the queue worker and session restore both go through it.
    ///
    /// # Errors
    ///
    /// Whatever the underlying [`Peripheral`] method returns.
    pub async fn execute(&self, peripheral: &dyn Peripheral) -> Result<OpOutcome, Error> {
        match self {
            Self::Discover { filter } => Ok(OpOutcome::Discovered(
                peripheral.discover_services(filter).await?,
            )),
            Self::Read { ch } => Ok(OpOutcome::Read(peripheral.read(ch).await?)),
            Self::Write { ch, data, mode } => {
                peripheral.write(ch, data, *mode).await?;
                Ok(OpOutcome::Done)
            }
            Self::Subscribe { ch } => {
                peripheral.subscribe(ch).await?;
                Ok(OpOutcome::Done)
            }
            Self::Unsubscribe { ch } => {
                peripheral.unsubscribe(ch).await?;
                Ok(OpOutcome::Done)
            }
        }
    }

    /// Rewrites the session-scoped characteristic identifiers of the
    /// operation after a reconnect. Targets that no longer exist keep
    /// their old id; detect them with the same mapping (see
    /// `session::reindex_characteristic`) and drop the operation.
    pub fn reindex(&mut self, reindex_ch: impl Fn(CharacteristicId) -> Option<CharacteristicId>) {
        let rewrite = |ch: &mut CharacteristicId| {
            if let Some(mapped) = reindex_ch(*ch) {
                *ch = mapped;
            }
        };
        match self {
            Self::Discover { .. } => {}
            Self::Read { ch } | Self::Subscribe { ch } | Self::Unsubscribe { ch } => rewrite(ch),
            Self::Write { ch, .. } => rewrite(ch),
        }
    }
}

/// Result of a completed [`GattOperation`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OpOutcome {
    /// Discovery returned the GATT tree.
    Discovered(ServiceSet),
    /// Read returned the characteristic value.
    Read(Vec<u8>),
    /// The operation completed without a payload (write, subscribe,
    /// unsubscribe).
    Done,
}

/// Configuration for [`GattOpQueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueConfig {
    /// Maximum number of queued operations before `submit` blocks and
    /// `try_submit` rejects. This is the backpressure bound that keeps a
    /// chatty application from growing the queue without limit.
    pub capacity: usize,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self { capacity: 32 }
    }
}

struct Request {
    id: u64,
    op: GattOperation,
    reply: oneshot::Sender<Result<OpOutcome, Error>>,
}

/// Submit side of the per-connection operation queue.
///
/// One instance per connection, `&self` for all methods. The paired
/// [`QueueWorker`] executes operations in priority order.
#[derive(Debug)]
pub struct GattOpQueue {
    // A single long-lived sender: futures-channel parks a sender only after
    // the buffer is exceeded (and grants each *fresh* clone one free
    // message), so sharing one sender is what makes the capacity bound
    // exact. The async mutex lets every caller submit through `&self` and
    // carries the backpressure wait.
    tx: futures::lock::Mutex<mpsc::Sender<Request>>,
    cancelled: Arc<Mutex<HashSet<u64>>>,
    next_id: AtomicU64,
    // Requests sent and not yet replied. futures-channel's slot accounting
    // for one long-lived sender is not exact enough to bound in-flight work
    // (the initial guaranteed slot is consumed once and only replenished
    // after a parked send), so the capacity check for `try_submit` is done
    // explicitly against this counter.
    outstanding: Arc<AtomicUsize>,
    // Copy of `QueueConfig::capacity` for the `try_submit` check.
    capacity: usize,
}

/// Execution side of the queue: drains submitted operations and runs them
/// serially against one peripheral.
pub struct QueueWorker {
    rx: mpsc::Receiver<Request>,
    peripheral: Arc<dyn Peripheral>,
    cancelled: Arc<Mutex<HashSet<u64>>>,
    outstanding: Arc<AtomicUsize>,
}

/// Handle for cancelling one queued operation.
///
/// Cancellation only affects operations still waiting in the queue; an
/// operation already handed to the peripheral runs to completion (GATT
/// procedures cannot be aborted mid-flight through the trait surface).
#[derive(Debug, Clone)]
pub struct OpHandle {
    id: u64,
    cancelled: Arc<Mutex<HashSet<u64>>>,
}

impl OpHandle {
    /// Cancels the operation if it has not started yet. Returns `true` when
    /// this call cancelled it; `false` when it was already running,
    /// already cancelled, or already completed.
    pub fn cancel(&self) -> bool {
        lock(&self.cancelled).insert(self.id)
    }

    /// Returns `true` once the operation has been cancelled (whether or not
    /// it had already started when `cancel` arrived).
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        lock(&self.cancelled).contains(&self.id)
    }
}

/// One submitted operation: the cancel handle plus the result slot.
#[derive(Debug)]
pub struct OpSubmission {
    /// Handle to cancel this operation while it is still queued.
    pub handle: OpHandle,
    /// Resolves with the outcome once the worker executed (or cancelled) it.
    pub result: oneshot::Receiver<Result<OpOutcome, Error>>,
}

impl GattOpQueue {
    /// Creates a queue and its worker for one connection. Drop (or stop
    /// polling) the worker to shut the queue down; pending submitters then
    /// fail with [`Error::InvalidState`].
    #[must_use]
    pub fn new(peripheral: Arc<dyn Peripheral>, config: QueueConfig) -> (Self, QueueWorker) {
        // futures-channel parks a sender once the buffer is exceeded, so a
        // buffer of capacity - 1 with a single long-lived sender yields an
        // in-flight bound of `capacity` (floored at one slot). The exact
        // accounting for `try_submit` is done by the `outstanding` counter,
        // not by channel state.
        let (tx, rx) = mpsc::channel(config.capacity.max(1).saturating_sub(1));
        let cancelled = Arc::new(Mutex::new(HashSet::new()));
        let outstanding = Arc::new(AtomicUsize::new(0));
        let queue = Self {
            tx: futures::lock::Mutex::new(tx),
            cancelled: Arc::clone(&cancelled),
            next_id: AtomicU64::new(1),
            outstanding: Arc::clone(&outstanding),
            capacity: config.capacity.max(1),
        };
        let worker = QueueWorker {
            rx,
            peripheral,
            cancelled,
            outstanding,
        };
        (queue, worker)
    }

    /// Submits `op`, suspending until the queue has capacity (bounded-channel
    /// backpressure). The returned [`OpSubmission`] carries the cancel handle
    /// and the result slot.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when the worker has been dropped.
    pub async fn submit(&self, op: GattOperation) -> Result<OpSubmission, Error> {
        let (reply, result) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut tx = self.tx.lock().await;
        tx.send(Request { id, op, reply })
            .await
            .map_err(|_| queue_closed_error())?;
        self.outstanding.fetch_add(1, Ordering::Relaxed);
        Ok(OpSubmission {
            handle: OpHandle {
                id,
                cancelled: Arc::clone(&self.cancelled),
            },
            result,
        })
    }

    /// Submits `op` without waiting: fails immediately instead of
    /// backpressuring when the queue is at capacity.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when the number of submitted-but-unanswered
    /// operations has reached the configured capacity (this includes an
    /// operation currently executing and the moment another submit is parked
    /// waiting for space), or the worker has been dropped.
    pub fn try_submit(&self, op: GattOperation) -> Result<OpSubmission, Error> {
        if self.outstanding.load(Ordering::Relaxed) >= self.capacity {
            return Err(Error::InvalidState("gatt operation queue full".to_owned()));
        }
        let (reply, result) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut tx = self.tx.try_lock().ok_or_else(|| {
            // A submit holds the sender while parked exactly when the queue
            // is at capacity, so contention means "full or momentarily
            // busy" - the same rejection for the caller.
            Error::InvalidState("gatt operation queue full".to_owned())
        })?;
        tx.try_send(Request { id, op, reply }).map_err(|err| {
            if err.is_full() {
                Error::InvalidState("gatt operation queue full".to_owned())
            } else {
                queue_closed_error()
            }
        })?;
        self.outstanding.fetch_add(1, Ordering::Relaxed);
        Ok(OpSubmission {
            handle: OpHandle {
                id,
                cancelled: Arc::clone(&self.cancelled),
            },
            result,
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl QueueWorker {
    /// Runs the queue until the submit side is dropped: every operation is
    /// executed serially, highest priority first, FIFO within a priority.
    /// Attempts to execute while the peripheral is disconnected surface the
    /// backend's own [`Error::NotConnected`] to the operation's submitter.
    pub async fn run(mut self) {
        let mut pending: Vec<Request> = Vec::new();
        loop {
            if pending.is_empty() {
                match self.rx.next().await {
                    Some(request) => pending.push(request),
                    None => return,
                }
            }
            // Drain everything already queued without blocking, so a
            // subscribe submitted right behind a write is picked first.
            while let Ok(request) = self.rx.try_recv() {
                pending.push(request);
            }
            let best = pending
                .iter()
                .enumerate()
                .min_by_key(|(_, request)| (request.op.priority(), request.id))
                .map(|(index, _)| index);
            let Some(index) = best else { continue };
            let request = pending.swap_remove(index);
            if lock(&self.cancelled).remove(&request.id) {
                let _ = request.reply.send(Err(cancelled_error()));
                self.outstanding.fetch_sub(1, Ordering::Relaxed);
                continue;
            }
            let outcome = request.op.execute(self.peripheral.as_ref()).await;
            let _ = request.reply.send(outcome);
            self.outstanding.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{CONTRACT_CHARACTERISTIC_UUID, CONTRACT_SERVICE_UUID};
    use crate::device::DeviceId;
    use crate::gatt::ServiceId;
    use std::sync::atomic::AtomicUsize;

    const ID: DeviceId = DeviceId::Mac([1, 2, 3, 4, 5, 6], crate::AddressType::RandomStatic);

    /// Peripheral double that records the execution order of operations.
    /// With `gated()`, the first `read` blocks until `release_gate()` is
    /// called - used to pin an operation in flight.
    struct RecordingPeripheral {
        executed: Mutex<Vec<&'static str>>,
        subscribe_calls: AtomicUsize,
        gate_tx: Mutex<Option<oneshot::Sender<()>>>,
        gate_rx: Mutex<Option<oneshot::Receiver<()>>>,
    }

    impl RecordingPeripheral {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                executed: Mutex::new(Vec::new()),
                subscribe_calls: AtomicUsize::new(0),
                gate_tx: Mutex::new(None),
                gate_rx: Mutex::new(None),
            })
        }

        fn gated() -> Arc<Self> {
            let (gate_tx, gate_rx) = oneshot::channel();
            Arc::new(Self {
                executed: Mutex::new(Vec::new()),
                subscribe_calls: AtomicUsize::new(0),
                gate_tx: Mutex::new(Some(gate_tx)),
                gate_rx: Mutex::new(Some(gate_rx)),
            })
        }

        fn release_gate(&self) {
            if let Some(tx) = self.gate_tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
        }

        fn executed(&self) -> Vec<&'static str> {
            lock(&self.executed).clone()
        }
    }

    fn char_id() -> CharacteristicId {
        CharacteristicId {
            service: ServiceId {
                uuid: CONTRACT_SERVICE_UUID,
                instance: 0,
            },
            uuid: CONTRACT_CHARACTERISTIC_UUID,
            instance: 0,
        }
    }

    #[async_trait::async_trait]
    impl Peripheral for RecordingPeripheral {
        fn id(&self) -> &DeviceId {
            &ID
        }

        fn name(&self) -> Option<String> {
            None
        }

        async fn is_connected(&self) -> Result<bool, Error> {
            Ok(true)
        }

        async fn connect(&self) -> Result<(), Error> {
            Ok(())
        }

        async fn disconnect(&self) -> Result<(), Error> {
            Ok(())
        }

        async fn discover_services(&self, _filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
            lock(&self.executed).push("discover");
            Ok(ServiceSet::default())
        }

        async fn read(&self, _ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
            lock(&self.executed).push("read");
            let gate = lock(&self.gate_rx).take();
            if let Some(rx) = gate {
                let _ = rx.await;
            }
            Ok(Vec::new())
        }

        async fn write(
            &self,
            _ch: &CharacteristicId,
            _data: &[u8],
            _mode: WriteMode,
        ) -> Result<(), Error> {
            lock(&self.executed).push("write");
            Ok(())
        }

        async fn max_write_len(
            &self,
            _ch: &CharacteristicId,
            _mode: WriteMode,
        ) -> Result<usize, Error> {
            Ok(20)
        }

        async fn subscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            lock(&self.executed).push("subscribe");
            self.subscribe_calls.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn unsubscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            lock(&self.executed).push("unsubscribe");
            Ok(())
        }

        fn notifications(&self) -> crate::NotificationStream {
            futures::stream::empty().boxed()
        }

        async fn pair(&self) -> Result<(), Error> {
            Err(Error::Unsupported {
                platform: "test",
                feature: "pair",
            })
        }

        async fn rssi(&self) -> Result<i16, Error> {
            Ok(-40)
        }

        async fn mtu(&self) -> Result<u16, Error> {
            Ok(23)
        }
    }

    #[tokio::test]
    async fn priority_order_is_subscribe_write_then_discovery() {
        let peripheral = RecordingPeripheral::new();
        let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig::default());
        let ch = char_id();

        // Submitted in reverse priority order; the worker must reorder.
        let read = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        let write = queue
            .submit(GattOperation::Write {
                ch,
                data: vec![1],
                mode: WriteMode::WithResponse,
            })
            .await
            .expect("submit");
        let subscribe = queue
            .submit(GattOperation::Subscribe { ch })
            .await
            .expect("submit");
        let discover = queue
            .submit(GattOperation::discover_all())
            .await
            .expect("submit");

        tokio::spawn(worker.run());

        for submission in [read, write, subscribe, discover] {
            submission.result.await.expect("reply").expect("op ok");
        }
        assert_eq!(
            peripheral.executed(),
            ["subscribe", "write", "read", "discover"]
        );
    }

    #[tokio::test]
    async fn same_priority_runs_in_submission_order() {
        let peripheral = RecordingPeripheral::new();
        let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig::default());
        let ch = char_id();

        let first = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        let second = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        let third = queue
            .submit(GattOperation::discover_all())
            .await
            .expect("submit");

        tokio::spawn(worker.run());

        for submission in [first, second, third] {
            submission.result.await.expect("reply").expect("op ok");
        }
        assert_eq!(peripheral.executed(), ["read", "read", "discover"]);
    }

    #[tokio::test]
    async fn queued_operation_can_be_cancelled() {
        let peripheral = RecordingPeripheral::new();
        let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig::default());
        let ch = char_id();

        // Two reads: the first runs immediately once the worker starts, the
        // second is still queued when cancellation lands.
        let first = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        let second = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        assert!(second.handle.cancel());

        tokio::spawn(worker.run());

        first.result.await.expect("reply").expect("first ok");
        let outcome = second.result.await.expect("reply");
        assert!(matches!(outcome, Err(Error::InvalidState(_))));
        assert_eq!(peripheral.executed(), ["read"]);
    }

    #[tokio::test]
    async fn try_submit_rejects_when_full() {
        // capacity 1 means a rendezvous channel (buffer 0): a submit only
        // completes once the worker has taken the request, so the worker
        // must be running before the first submit.
        let peripheral = RecordingPeripheral::gated();
        let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig { capacity: 1 });
        let ch = char_id();
        tokio::spawn(worker.run());

        // Op 1 goes in flight and blocks on the gate, consuming the capacity.
        let first = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit");
        let err = queue.try_submit(GattOperation::Read { ch }).unwrap_err();
        assert!(matches!(err, Error::InvalidState(_)));

        // Once op 1 completes, the capacity frees up and submissions work
        // again - both the non-blocking and the backpressured path.
        peripheral.release_gate();
        first.result.await.expect("reply").expect("op ok");
        let second = queue
            .try_submit(GattOperation::Read { ch })
            .expect("capacity available");
        second.result.await.expect("reply").expect("op ok");
        let late = queue
            .submit(GattOperation::Read { ch })
            .await
            .expect("submit after drain");
        late.result.await.expect("reply").expect("op ok");
        assert_eq!(peripheral.executed(), ["read", "read", "read"]);
    }

    #[tokio::test]
    async fn discover_returns_service_set() {
        let peripheral = RecordingPeripheral::new();
        let (queue, worker) = GattOpQueue::new(peripheral.clone(), QueueConfig::default());
        tokio::spawn(worker.run());

        let submission = queue
            .submit(GattOperation::discover_all())
            .await
            .expect("submit");
        let outcome = submission.result.await.expect("reply").expect("op ok");
        assert!(matches!(outcome, OpOutcome::Discovered(_)));
    }

    #[test]
    fn operation_priority_classes() {
        let ch = char_id();
        assert_eq!(
            GattOperation::Subscribe { ch }.priority(),
            GattOpPriority::Subscribe
        );
        assert_eq!(
            GattOperation::Write {
                ch,
                data: Vec::new(),
                mode: WriteMode::WithoutResponse
            }
            .priority(),
            GattOpPriority::Write
        );
        assert_eq!(
            GattOperation::Read { ch }.priority(),
            GattOpPriority::Discovery
        );
        assert_eq!(
            GattOperation::discover_all().priority(),
            GattOpPriority::Discovery
        );
        assert!(GattOpPriority::Subscribe < GattOpPriority::Write);
        assert!(GattOpPriority::Write < GattOpPriority::Discovery);
    }

    #[test]
    fn reindex_rewrites_characteristic_ids() {
        let mut op = GattOperation::Read { ch: char_id() };
        op.reindex(|_| {
            Some(CharacteristicId {
                service: ServiceId {
                    uuid: CONTRACT_SERVICE_UUID,
                    instance: 0,
                },
                uuid: CONTRACT_CHARACTERISTIC_UUID,
                instance: 1,
            })
        });
        match &op {
            GattOperation::Read { ch } => assert_eq!(ch.instance, 1),
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
