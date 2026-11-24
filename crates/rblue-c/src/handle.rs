//! The facade handle: a private tokio runtime plus the adapter and the
//! event/notification pump.
//!
//! Mirrors the `rblue-ffi` facade's pump design (bounded channel, per-device
//! forwarders, lazy central task) with two deliberate differences: the runtime
//! is owned by the handle instead of being a process-wide singleton, so
//! `rblue_free` can shut everything down deterministically; and forwarder
//! liveness is re-checked on every `subscribe`, so resubscribing after a
//! reconnect self-heals instead of relying on the subscription bookkeeping
//! alone.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use rblue::prelude::*;

use crate::callbacks::Callbacks;
use crate::dispatch::{self, Request};
use crate::event::EventEnvelope;

/// Capacity of the event/notification channel. Bounded on purpose: a stalled
/// host callback applies backpressure into the backend's broadcast buffers
/// instead of growing memory without limit.
const PUMP_CHANNEL_CAPACITY: usize = 256;

/// Messages multiplexed into the central pump task.
enum PumpMessage {
    /// One adapter event from the single event stream.
    Event(AdapterEvent),
    /// One local GATT server event.
    GattServer(GattServerEvent),
    /// One notification from a per-device forwarder task, with the device it
    /// belongs to (the `Notification` itself only names the characteristic).
    Notification {
        device: DeviceId,
        notification: Notification,
    },
    /// One event from the connection supervisor (`SupervisorHandle`).
    Supervisor(SupervisorEvent),
}

/// The notification-forwarding machinery: the bounded channel every forwarder
/// pushes into, the central task draining it into the event callback, and one
/// live forwarder task per device with subscriptions.
struct Pump {
    tx: tokio::sync::mpsc::Sender<PumpMessage>,
    rx: Option<tokio::sync::mpsc::Receiver<PumpMessage>>,
    central: Option<tokio::task::JoinHandle<()>>,
    forwarders: HashMap<String, tokio::task::JoinHandle<()>>,
}

/// Subscribed characteristics per device (canonical `DeviceId` JSON -> set of
/// canonical `CharacteristicId` JSON), so forwarders stop when a device's
/// last subscription goes away.
type Subscriptions = HashMap<String, HashSet<String>>;

/// Shared state of one facade handle, handed to runtime tasks by `Arc`.
/// Live connection supervisor, created lazily on the first `auto_connect`.
struct SupervisorSlot {
    handle: SupervisorHandle,
    policy: ReconnectPolicy,
    run_task: tokio::task::JoinHandle<()>,
    event_task: tokio::task::JoinHandle<()>,
}

pub(crate) struct CoreInner {
    adapter: Arc<dyn Adapter>,
    callbacks: Arc<Callbacks>,
    pump: Mutex<Pump>,
    /// Clone of the pump channel sender, so the supervisor can publish
    /// events without taking the pump lock while other locks are held.
    pump_tx: tokio::sync::mpsc::Sender<PumpMessage>,
    subscriptions: Mutex<Subscriptions>,
    supervisor: Mutex<Option<SupervisorSlot>>,
}

impl CoreInner {
    /// The backend adapter every dispatched request runs against.
    pub(crate) fn adapter(&self) -> &Arc<dyn Adapter> {
        &self.adapter
    }

    /// Resolves a device to its peripheral handle.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] for a device unknown to the adapter, backend
    /// failures otherwise.
    pub(crate) async fn peripheral(&self, device: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        self.adapter.peripheral(device).await
    }

    /// Records a subscription and starts the device's notification forwarder
    /// unless a live one exists, then resolves once the backend broadcast is
    /// attached, so values published after `subscribe` resolves are never
    /// missed. Runs inside the facade runtime (every caller is a dispatched
    /// request).
    pub(crate) async fn attach_subscriber(
        &self,
        device: &DeviceId,
        characteristic: &CharacteristicId,
        peripheral: Arc<dyn Peripheral>,
    ) {
        let device_key = canonical_json(device);
        let characteristic_key = canonical_json(characteristic);
        let _first = {
            let mut subscriptions = self
                .subscriptions
                .lock()
                .expect("subscription state must not be poisoned");
            subscriptions
                .entry(device_key.clone())
                .or_default()
                .insert(characteristic_key)
        };
        self.ensure_forwarder(peripheral, device.clone()).await;
    }

    /// Drops a subscription; when it was the device's last, stops its
    /// forwarder.
    pub(crate) fn detach_subscriber(&self, device: &DeviceId, characteristic: &CharacteristicId) {
        let device_key = canonical_json(device);
        let characteristic_key = canonical_json(characteristic);
        let last = {
            let mut subscriptions = self
                .subscriptions
                .lock()
                .expect("subscription state must not be poisoned");
            let mut last = false;
            if let Some(characteristics) = subscriptions.get_mut(&device_key) {
                if characteristics.remove(&characteristic_key) && characteristics.is_empty() {
                    subscriptions.remove(&device_key);
                    last = true;
                }
            }
            last
        };
        if last {
            self.stop_forwarder(&device_key);
        }
    }

    /// Removes every subscription record of a device (after an explicit
    /// disconnect) and stops its forwarder.
    pub(crate) fn drop_device_subscriptions(&self, device: &DeviceId) {
        let device_key = canonical_json(device);
        let mut subscriptions = self
            .subscriptions
            .lock()
            .expect("subscription state must not be poisoned");
        subscriptions.remove(&device_key);
        drop(subscriptions);
        self.stop_forwarder(&device_key);
    }

    /// Starts the device's notification forwarder unless a live one exists,
    /// then resolves once the backend broadcast is attached. Checking for
    /// liveness (not mere presence) makes `subscribe` after a reconnect
    /// self-healing: a forwarder killed by a disconnect is replaced.
    async fn ensure_forwarder(&self, peripheral: Arc<dyn Peripheral>, device: DeviceId) {
        let (attached, attach_signal) = tokio::sync::oneshot::channel();
        if !self.spawn_forwarder(peripheral, device, Some(attached)) {
            return;
        }
        // The forwarder always answers the signal unless it is aborted during
        // teardown, which no dispatched caller observes.
        let _ = attach_signal.await;
    }

    /// Spawns the forwarder task for one peripheral; returns `false` when a
    /// live forwarder already exists.
    fn spawn_forwarder(
        &self,
        peripheral: Arc<dyn Peripheral>,
        device: DeviceId,
        attached: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> bool {
        let device_key = canonical_json(&device);
        let mut pump = self.lock_pump();
        if pump
            .forwarders
            .get(&device_key)
            .is_some_and(|task| !task.is_finished())
        {
            return false;
        }
        let tx = pump.tx.clone();
        let task = tokio::spawn(forward_notifications(
            peripheral,
            device.clone(),
            tx,
            attached,
        ));
        pump.forwarders.insert(device_key, task);
        true
    }

    /// Stops and forgets the device's notification forwarder, if any.
    fn stop_forwarder(&self, device_key: &str) {
        let mut pump = self.lock_pump();
        if let Some(task) = pump.forwarders.remove(device_key) {
            task.abort();
        }
    }

    /// Registers the event callback and starts the central pump task on the
    /// first registration; later registrations only swap the callback.
    pub(crate) fn register_event_callback(
        &self,
        callback: Option<crate::callbacks::RblueEventCallback>,
        user_data: *mut std::os::raw::c_void,
    ) {
        self.callbacks.set_event(callback, user_data);
        if callback.is_some() {
            self.ensure_central_task();
        }
    }

    /// Starts the central task draining adapter events and forwarder messages
    /// into the event callback, unless it is already running.
    fn ensure_central_task(&self) {
        let mut pump = self.lock_pump();
        let (Some(rx), None) = (pump.rx.take(), pump.central.as_ref()) else {
            return;
        };
        let adapter = Arc::clone(&self.adapter);
        let callbacks = Arc::clone(&self.callbacks);
        pump.central = Some(tokio::spawn(async move {
            let mut events = adapter.events();
            // Unsupported backends get a pending stream so the pump is not
            // torn down when the unused branch yields None.
            let mut server_events = match adapter.server_events() {
                Ok(stream) => stream,
                Err(_) => futures::stream::pending().boxed(),
            };
            let mut rx = rx;
            loop {
                tokio::select! {
                    event = events.next() => match event {
                        Some(event) => deliver(&callbacks, PumpMessage::Event(event)),
                        None => break,
                    },
                    event = server_events.next() => match event {
                        Some(event) => deliver(&callbacks, PumpMessage::GattServer(event)),
                        None => {
                            server_events = futures::stream::pending().boxed();
                        }
                    },
                    message = rx.recv() => match message {
                        Some(message) => deliver(&callbacks, message),
                        None => break,
                    },
                }
            }
        }));
    }

    /// Delivers one completed request through the response callback.
    pub(crate) fn deliver_response(
        &self,
        req_id: u64,
        outcome: Result<Vec<u8>, dispatch::RequestFailure>,
    ) {
        let status = match &outcome {
            Err(failure) => Some(failure.to_status_cstring()),
            Ok(_) => None,
        };
        self.callbacks
            .deliver_response(req_id, status, outcome.unwrap_or_default());
    }

    fn lock_pump(&self) -> std::sync::MutexGuard<'_, Pump> {
        self.pump
            .lock()
            .expect("event pump state must not be poisoned")
    }
}

impl CoreInner {
    /// Aborts the central pump and every forwarder. Called on teardown while
    /// the runtime is still alive, so no pump callback can race the shutdown.
    pub(crate) fn abort_pump(&self) {
        self.abort_supervisor();
        let mut pump = self.lock_pump();
        if let Some(central) = pump.central.take() {
            central.abort();
        }
        for (_, task) in pump.forwarders.drain() {
            task.abort();
        }
    }

    /// Stops the supervisor loop and its event forwarder, if they were started.
    fn abort_supervisor(&self) {
        let slot = self
            .supervisor
            .lock()
            .expect("supervisor state must not be poisoned")
            .take();
        if let Some(slot) = slot {
            slot.run_task.abort();
            slot.event_task.abort();
        }
    }

    /// Starts the supervisor with `policy` (or the default policy) if needed,
    /// then registers `device` for application-level auto-reconnect.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when a supervisor is already running with a
    /// different policy, or when the supervisor loop is gone.
    pub(crate) async fn supervisor_auto_connect(
        &self,
        device: DeviceId,
        policy: Option<ReconnectPolicy>,
    ) -> Result<(), Error> {
        let handle = self.ensure_supervisor(policy)?;
        handle.auto_connect(device).await
    }

    /// Stops supervising `device`. Succeeds if the supervisor was never started.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when the supervisor loop is gone.
    pub(crate) async fn supervisor_unwatch(&self, device: DeviceId) -> Result<(), Error> {
        let handle = {
            let slot = self
                .supervisor
                .lock()
                .expect("supervisor state must not be poisoned");
            slot.as_ref().map(|slot| slot.handle.clone())
        };
        match handle {
            Some(handle) => handle.unwatch(device).await,
            None => Ok(()),
        }
    }

    fn ensure_supervisor(
        &self,
        policy: Option<ReconnectPolicy>,
    ) -> Result<SupervisorHandle, Error> {
        let mut slot = self
            .supervisor
            .lock()
            .expect("supervisor state must not be poisoned");
        if let Some(existing) = slot.as_ref() {
            if let Some(policy) = policy.as_ref() {
                if &existing.policy != policy {
                    return Err(Error::InvalidState(
                        "supervisor already running with a different policy".to_owned(),
                    ));
                }
            }
            return Ok(existing.handle.clone());
        }
        let policy = policy.unwrap_or_default();
        let (supervisor, handle, mut events) =
            ConnectionSupervisor::new(Arc::clone(&self.adapter), policy.clone());
        let run_task = tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                tracing::warn!(?error, "supervisor loop ended with error");
            }
        });
        let tx = self.pump_tx.clone();
        let event_task = tokio::spawn(async move {
            while let Some(event) = events.next().await {
                if tx.send(PumpMessage::Supervisor(event)).await.is_err() {
                    break;
                }
            }
        });
        self.ensure_central_task();
        *slot = Some(SupervisorSlot {
            handle: handle.clone(),
            policy,
            run_task,
            event_task,
        });
        Ok(handle)
    }
}

impl Drop for CoreInner {
    fn drop(&mut self) {
        self.abort_pump();
    }
}

/// Forwards one peripheral's notification broadcast into the pump channel
/// until either end goes away, tagging values with the device they belong to.
///
/// The backend broadcast subscription (the attachment point) happens before
/// the `attached` signal is answered, so a caller awaiting that signal knows
/// values published from then on are observed.
async fn forward_notifications(
    peripheral: Arc<dyn Peripheral>,
    device: DeviceId,
    tx: tokio::sync::mpsc::Sender<PumpMessage>,
    attached: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let mut notifications = peripheral.notifications();
    if let Some(attached) = attached {
        let _ = attached.send(());
    }
    while let Some(notification) = notifications.next().await {
        if tx
            .send(PumpMessage::Notification {
                device: device.clone(),
                notification,
            })
            .await
            .is_err()
        {
            break;
        }
    }
    tracing::debug!(device = %canonical_json(&device), "notification stream ended");
}

/// Delivers one pump message to the registered event callback. Called only
/// from the central task; without a callback the message is dropped (the
/// channel stays bounded).
fn deliver(callbacks: &Callbacks, message: PumpMessage) {
    let json = match &message {
        PumpMessage::Event(event) => EventEnvelope::AdapterEvent { event }.to_json(),
        PumpMessage::Notification {
            device,
            notification,
        } => EventEnvelope::Notification {
            device,
            notification,
        }
        .to_json(),
        PumpMessage::Supervisor(event) => EventEnvelope::SupervisorEvent { event }.to_json(),
        PumpMessage::GattServer(event) => EventEnvelope::GattServerEvent { event }.to_json(),
    };
    callbacks.deliver_event(cstring(json));
}

/// Turns an envelope JSON string into the owned C string the callback
/// receives. JSON never contains interior NUL bytes.
fn cstring(json: String) -> CString {
    CString::new(json).expect("JSON strings never contain NUL bytes")
}

/// Canonical JSON for a parsed identifier, so map keys are
/// spelling-independent. Serialization of the identifier types cannot fail.
pub(crate) fn canonical_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("identifier JSON serialization cannot fail")
}

/// Runtime and shared state of one facade, boxed so teardown can move the
/// whole unit onto a helper thread when needed.
struct CoreState {
    /// Drives dispatched requests and pump tasks; dropping it cancels any
    /// request still in flight, after which no callback can fire.
    rt: tokio::runtime::Runtime,
    /// Shared state; dropped before the runtime so pump tasks are aborted
    /// while the runtime is still alive.
    inner: Arc<CoreInner>,
}

/// The facade over one backend adapter: a private multi-thread tokio runtime
/// plus the shared core. Constructed through `rblue_new`.
pub(crate) struct Core {
    /// `Some` until teardown; `Option` so shutdown can move the state.
    state: Mutex<Option<Box<CoreState>>>,
    /// Final callback counts cached for idempotent shutdown calls.
    shutdown_counts: Mutex<Option<(u64, u64)>>,
}

impl Core {
    /// Builds the facade over the platform-selected backend.
    ///
    /// # Errors
    ///
    /// [`Error::Unsupported`] when the compiled-in backend cannot run on this
    /// target (only possible on deliberately mismatched builds; the mock
    /// backend never fails).
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self::with_adapter(rblue::platform::default_adapter()?))
    }

    /// Builds the facade over an explicit adapter (Rust-side only: the
    /// adapter is not an FFI type). Tests and embedders use this.
    pub(crate) fn with_adapter(adapter: Arc<dyn Adapter>) -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(PUMP_CHANNEL_CAPACITY);
        let inner = Arc::new(CoreInner {
            adapter,
            callbacks: Arc::new(Callbacks::default()),
            pump: Mutex::new(Pump {
                tx: tx.clone(),
                rx: Some(rx),
                central: None,
                forwarders: HashMap::new(),
            }),
            pump_tx: tx,
            subscriptions: Mutex::new(HashMap::new()),
            supervisor: Mutex::new(None),
        });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .thread_name("rblue-c")
            .enable_all()
            .build()
            .expect("rblue-c facade runtime must start");
        Self {
            state: Mutex::new(Some(Box::new(CoreState { rt, inner }))),
            shutdown_counts: Mutex::new(None),
        }
    }

    /// Registers the response callback slot.
    pub(crate) fn set_response_callback(
        &self,
        callback: Option<crate::callbacks::RblueResponseCallback>,
        user_data: *mut std::os::raw::c_void,
        owned_payload: bool,
    ) {
        let state = self
            .state
            .lock()
            .expect("facade state must not be poisoned");
        let state = state.as_ref().expect("facade state lives until teardown");
        state
            .inner
            .callbacks
            .set_response(callback, user_data, owned_payload);
    }

    /// Registers the event callback slot (and starts the pump on first use).
    pub(crate) fn set_event_callback(
        &self,
        callback: Option<crate::callbacks::RblueEventCallback>,
        user_data: *mut std::os::raw::c_void,
    ) {
        let state = self
            .state
            .lock()
            .expect("facade state must not be poisoned");
        let state = state.as_ref().expect("facade state lives until teardown");
        // Host FFI threads do not have a Tokio reactor. The pump must belong
        // to this facade, even if registration happens inside another runtime.
        let _runtime = state.rt.enter();
        state.inner.register_event_callback(callback, user_data);
    }

    /// Dispatches one request on the facade runtime. The request's buffers
    /// are copied by the caller before this point, so ownership moves freely.
    pub(crate) fn spawn_request(&self, req_id: u64, request: Request) {
        let state = self
            .state
            .lock()
            .expect("facade state must not be poisoned");
        let state = state.as_ref().expect("facade state lives until teardown");
        let inner = Arc::clone(&state.inner);
        state.rt.spawn(async move {
            let outcome = dispatch::execute(&inner, request).await;
            inner.deliver_response(req_id, outcome);
        });
    }

    /// Stops every native producer and returns the final delivered callback
    /// counts. Repeated calls return the same counts.
    pub(crate) fn shutdown(&self) -> (u64, u64) {
        if let Some(counts) = *self
            .shutdown_counts
            .lock()
            .expect("shutdown count state must not be poisoned")
        {
            return counts;
        }
        let state = self
            .state
            .lock()
            .expect("facade state must not be poisoned")
            .take();
        let Some(state) = state else {
            return self
                .shutdown_counts
                .lock()
                .expect("shutdown count state must not be poisoned")
                .unwrap_or_default();
        };
        state.inner.abort_pump();
        let callbacks = Arc::clone(&state.inner.callbacks);
        drop_runtime_state(state);
        let counts = callbacks.delivered_counts();
        *self
            .shutdown_counts
            .lock()
            .expect("shutdown count state must not be poisoned") = Some(counts);
        counts
    }
}

fn drop_runtime_state(state: Box<CoreState>) {
    if tokio::runtime::Handle::try_current().is_ok() {
        let shutdown = std::thread::Builder::new()
            .name("rblue-c-shutdown".to_owned())
            .spawn(move || drop(state))
            .expect("shutdown helper thread must spawn");
        let _ = shutdown.join();
    } else {
        drop(state);
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::Core;
    use crate::testing::Recorder;
    use std::sync::Arc;

    #[test]
    fn callback_registration_without_host_runtime_uses_facade_runtime() {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let adapter = Arc::new(rblue_mock::MockAdapter::new());
        let peripheral = adapter.add_device(crate::testing::device_id(), "fixture");
        peripheral.advertise();
        let core = Core::with_adapter(adapter);
        let mut recorder = Recorder::attach(&core);
        // Drive only the recorder on a separate runtime, just as a host event
        // loop awaits callbacks while the facade's own workers run the pump.
        let observer = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test observer runtime must start");
        observer.block_on(async {
            let event = recorder.next_event().await;
            assert!(event.contains("device_discovered"), "{event}");
        });
    }
}
