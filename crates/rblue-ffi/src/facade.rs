//! The [`Rblue`] object and [`EventSink`] callback interface.
//!
//! [`Rblue`] wraps one `Arc<dyn Adapter>` selected at construction. Events
//! and notifications are forwarded to an installed sink by a pump task on a
//! facade-owned tokio runtime (see the crate documentation for the delivery
//! and backpressure contract).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use futures::StreamExt;
use rblue::prelude::{
    Adapter, AdapterEvent, CapabilitySet, CharacteristicId, DeviceId, Notification, Peripheral,
    RequestDeviceOptions, ScanFilter, ServiceSet, WriteMode,
};
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::RblueError;

/// Host-side receiver for adapter events and GATT notifications.
///
/// Implemented on the foreign side (Kotlin object / Swift class) and
/// registered with [`Rblue::set_event_sink`]. Methods are invoked from the
/// facade's pump worker threads: implementations must not block, must not
/// re-enter the facade, and must not throw - an exception unwinds into the
/// pump and terminates event delivery. Buffering and drop accounting are the
/// binding layer's job (`callbackFlow(BUFFERED)` / `AsyncStream`; see the
/// crate documentation).
#[uniffi::export(callback_interface)]
pub trait EventSink: Send + Sync {
    /// One adapter event (a serialized `rblue_api::AdapterEvent`): state
    /// changes, discoveries, connection state. Values are opaque - parse the
    /// JSON with the schema the `rblue-api` serde types define.
    fn on_event(&self, event_json: String);

    /// One GATT notification/indication (a serialized
    /// `rblue_api::Notification`), delivered only for subscribed
    /// characteristics.
    fn on_notification(&self, notification_json: String);
}

/// Messages multiplexed into the pump: adapter events from the single event
/// stream and notifications from the per-device forwarder tasks.
enum PumpMessage {
    Event(AdapterEvent),
    Notification(Notification),
}

/// The notification-forwarding machinery.
///
/// The channel exists for the facade's lifetime, independent of any sink:
/// every `subscribe` starts a per-device forwarder task that pushes values
/// into it (waiting until the backend broadcast is attached, so values
/// published after `subscribe` resolves are never missed). The first
/// `set_event_sink` starts the central task that drains the channel into the
/// current sink; later `set_event_sink` calls only swap the sink.
struct Pump {
    tx: tokio::sync::mpsc::Sender<PumpMessage>,
    rx: Option<tokio::sync::mpsc::Receiver<PumpMessage>>,
    central: Option<tokio::task::JoinHandle<()>>,
    forwarders: HashMap<String, tokio::task::JoinHandle<()>>,
}

/// Subscribed characteristics per device (canonical `DeviceId` JSON -> set
/// of canonical `CharacteristicId` JSON), so forwarders can be stopped when
/// a device's last subscription goes away.
type Subscriptions = HashMap<String, HashSet<String>>;

/// The BLE facade: one adapter, wrapped for Kotlin and Swift.
///
/// Construct with [`Rblue::new`] (platform-selected backend) or, from Rust
/// code only, [`Rblue::with_adapter`] (explicit backend). Async methods are
/// exported as Kotlin `suspend` / Swift `async`; identifiers and structured
/// payloads cross the boundary as JSON strings (see the crate documentation).
#[derive(uniffi::Object)]
pub struct Rblue {
    adapter: Arc<dyn Adapter>,
    subscriptions: Mutex<Subscriptions>,
    pump: Mutex<Pump>,
    sink: Arc<Mutex<Option<Box<dyn EventSink>>>>,
}

/// The capacity of the pump's event/notification channel. Bounded on
/// purpose: a stalled sink applies backpressure into the backend's broadcast
/// buffers instead of growing memory without limit.
const PUMP_CHANNEL_CAPACITY: usize = 256;

/// The facade-owned runtime that drives the pump. Built once on first use;
/// FFI-called async methods run on UniFFI's tokio assist instead, and Rust
/// callers await them on their own runtime.
fn pump_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("rblue-ffi-pump")
            .enable_all()
            .build()
            .expect("rblue-ffi event pump runtime must start")
    })
}

/// Parses a JSON method argument.
///
/// # Errors
///
/// [`RblueError::InvalidArgument`] naming `what` when `json` is not valid
/// for `T`.
fn parse_json<T: DeserializeOwned>(json: &str, what: &str) -> Result<T, RblueError> {
    serde_json::from_str(json).map_err(|error| RblueError::InvalidArgument {
        message: format!("invalid {what}: {error}"),
    })
}

/// Parses a JSON method argument for a type with a `Default`, overlaying the
/// given object on the serialized default so omitted fields keep their
/// default (`{}` is the zero-value request, e.g. the unfiltered
/// `ScanFilter`).
///
/// # Errors
///
/// [`RblueError::InvalidArgument`] naming `what` when `json` is not valid
/// for `T`.
fn parse_with_defaults<T>(json: &str, what: &str) -> Result<T, RblueError>
where
    T: DeserializeOwned + serde::Serialize + Default,
{
    let value: serde_json::Value = parse_json(json, what)?;
    let defaults =
        serde_json::to_value(T::default()).expect("default request JSON serialization cannot fail");
    let merged = match (defaults, value) {
        (serde_json::Value::Object(mut base), serde_json::Value::Object(overrides)) => {
            for (key, value) in overrides {
                base.insert(key, value);
            }
            serde_json::Value::Object(base)
        }
        (_, value) => value,
    };
    serde_json::from_value(merged).map_err(|error| RblueError::InvalidArgument {
        message: format!("invalid {what}: {error}"),
    })
}

/// Canonical JSON for a parsed identifier, so map keys and error payloads are
/// spelling-independent. Serialization of the identifier types cannot fail.
fn canonical_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("identifier JSON serialization cannot fail")
}

/// Parses a `DeviceId` argument.
///
/// # Errors
///
/// [`RblueError::InvalidArgument`] for malformed JSON.
fn parse_device_id(device_json: &str) -> Result<DeviceId, RblueError> {
    parse_json(device_json, "DeviceId JSON")
}

/// Parses a `CharacteristicId` argument.
///
/// # Errors
///
/// [`RblueError::InvalidArgument`] for malformed JSON.
fn parse_characteristic_id(characteristic_json: &str) -> Result<CharacteristicId, RblueError> {
    parse_json(characteristic_json, "CharacteristicId JSON")
}

/// Parses a service-UUID method argument.
///
/// # Errors
///
/// [`RblueError::InvalidArgument`] for a malformed UUID.
fn parse_uuid(uuid: &str) -> Result<Uuid, RblueError> {
    Uuid::parse_str(uuid).map_err(|error| RblueError::InvalidArgument {
        message: format!("invalid service UUID {uuid:?}: {error}"),
    })
}

/// Serializes an adapter event / notification for the sink. Serialization of
/// the event model cannot fail.
fn event_json(event: &AdapterEvent) -> String {
    serde_json::to_string(event).expect("adapter event JSON serialization cannot fail")
}

/// See [`event_json`].
fn notification_json(notification: &Notification) -> String {
    serde_json::to_string(notification).expect("notification JSON serialization cannot fail")
}

/// Delivers one pump message to the currently installed sink. Called only
/// from the central task, which starts with the first `set_event_sink`, so
/// the slot is always occupied in practice; without a sink the message is
/// dropped (forwarders keep the channel bounded).
fn deliver(sink_slot: &Mutex<Option<Box<dyn EventSink>>>, message: PumpMessage) {
    let sink = sink_slot
        .lock()
        .expect("event sink state must not be poisoned");
    let Some(sink) = sink.as_deref() else {
        return;
    };
    match message {
        PumpMessage::Event(event) => sink.on_event(event_json(&event)),
        PumpMessage::Notification(notification) => {
            sink.on_notification(notification_json(&notification));
        }
    }
}

/// Forwards one peripheral's notification broadcast into the pump channel
/// until either end goes away.
///
/// The backend broadcast subscription (the attachment point) happens before
/// the `attached` signal is answered, so a caller awaiting that signal knows
/// values published from then on are observed.
async fn forward(
    peripheral: Arc<dyn Peripheral>,
    tx: tokio::sync::mpsc::Sender<PumpMessage>,
    device_key: String,
    attached: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let mut notifications = peripheral.notifications();
    if let Some(attached) = attached {
        let _ = attached.send(());
    }
    while let Some(notification) = notifications.next().await {
        if tx
            .send(PumpMessage::Notification(notification))
            .await
            .is_err()
        {
            break;
        }
    }
    tracing::debug!(device = %device_key, "notification stream ended");
}

#[uniffi::export]
impl Rblue {
    /// Constructs the facade over the platform-selected backend
    /// (`rblue::platform::default_adapter`, feature-driven exactly like the
    /// aggregate `rblue` crate).
    ///
    /// # Errors
    ///
    /// [`RblueError::Unsupported`] when the selected backend cannot run on
    /// this target (e.g. the web backend on a non-wasm host).
    #[uniffi::constructor]
    pub fn new() -> Result<Arc<Self>, RblueError> {
        let adapter = rblue::platform::default_adapter()?;
        Ok(Self::with_adapter(adapter))
    }

    /// Initializes the adapter (radio/permission setup; on some platforms
    /// this is where permission prompts happen).
    ///
    /// # Errors
    ///
    /// Backend failures as [`RblueError`]; [`RblueError::Permission`] when a
    /// required Bluetooth permission is missing.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn initialize(&self) -> Result<(), RblueError> {
        Ok(self.adapter.initialize().await?)
    }

    /// The adapter's capability set as JSON
    /// (`rblue_api::CapabilitySet` bitflags: `|`-separated capability names).
    /// Inspect it to degrade gracefully instead of catching
    /// [`RblueError::Unsupported`] after the fact.
    #[uniffi::method]
    pub fn capabilities(&self) -> String {
        let capabilities: CapabilitySet = self.adapter.capabilities();
        serde_json::to_string(&capabilities).expect("capability set JSON serialization cannot fail")
    }

    /// Identity of the bound adapter (`"default"` in 1.0; multi-adapter
    /// selection is post-1.0).
    #[uniffi::method]
    pub fn adapter_id(&self) -> String {
        self.adapter.id().as_str().to_owned()
    }

    /// The version string of the selected backend, for diagnostics and
    /// backend-swap probes.
    #[uniffi::method]
    pub fn backend_version(&self) -> String {
        rblue::platform::backend_version().to_owned()
    }

    /// Starts a scan with the given filter (`ScanFilter` JSON; `{}` accepts
    /// the defaults of every field, i.e. no filtering). Discoveries arrive
    /// through [`EventSink::on_event`] as `device_discovered` /
    /// `device_updated` events.
    ///
    /// # Errors
    ///
    /// [`RblueError::InvalidArgument`] for malformed JSON,
    /// [`RblueError::Permission`] without scan permission,
    /// [`RblueError::Unsupported`] where the platform has no background
    /// scanning (use `request_device` there).
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn start_scan(&self, filter_json: String) -> Result<(), RblueError> {
        let filter: ScanFilter = parse_with_defaults(&filter_json, "ScanFilter JSON")?;
        Ok(self.adapter.start_scan(&filter).await?)
    }

    /// Stops the running scan. Idempotent.
    ///
    /// # Errors
    ///
    /// Backend failures as [`RblueError`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn stop_scan(&self) -> Result<(), RblueError> {
        Ok(self.adapter.stop_scan().await?)
    }

    /// Connects to the device identified by `device_json` (a `DeviceId`
    /// JSON string taken from a discovery event). Resolves once the link is
    /// up; call `discover_services` next.
    ///
    /// # Errors
    ///
    /// [`RblueError::NotFound`] for an unknown device,
    /// [`RblueError::Platform`] with raw status codes such as Android GATT
    /// status 133.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn connect(&self, device_json: String) -> Result<(), RblueError> {
        Ok(self.peripheral(&device_json).await?.connect().await?)
    }

    /// Disconnects the device. Idempotent. The device's notification
    /// forwarder is stopped; `subscribe` again after reconnecting.
    ///
    /// # Errors
    ///
    /// Backend failures as [`RblueError`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn disconnect(&self, device_json: String) -> Result<(), RblueError> {
        let peripheral = self.peripheral(&device_json).await?;
        let result = peripheral.disconnect().await;
        self.stop_forwarder(&device_json);
        Ok(result?)
    }

    /// Discovers the GATT tree of the connected device. Must run on every
    /// connection (iOS requires it; Android's stack cache can be stale).
    ///
    /// # Arguments
    ///
    /// * `device_json` - `DeviceId` JSON of the connected device.
    /// * `service_uuids` - restrict discovery to these UUID strings; empty
    ///   or absent discovers everything.
    ///
    /// # Returns
    ///
    /// The discovered tree as `ServiceSet` JSON. Its characteristic
    /// identifiers are valid only for the current connection.
    ///
    /// # Errors
    ///
    /// [`RblueError::NotConnected`] before connecting,
    /// [`RblueError::InvalidArgument`] for malformed UUIDs.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn discover_services(
        &self,
        device_json: String,
        service_uuids: Option<Vec<String>>,
    ) -> Result<String, RblueError> {
        let uuids = service_uuids.unwrap_or_default();
        let mut filter = Vec::with_capacity(uuids.len());
        for uuid in &uuids {
            filter.push(parse_uuid(uuid)?);
        }
        let services: ServiceSet = self
            .peripheral(&device_json)
            .await?
            .discover_services(&filter)
            .await?;
        Ok(serde_json::to_string(&services).expect("service set JSON serialization cannot fail"))
    }

    /// Reads the current value of the characteristic (`CharacteristicId`
    /// JSON from the `discover_services` result).
    ///
    /// # Errors
    ///
    /// [`RblueError::NotConnected`], [`RblueError::Att`] when the procedure
    /// fails.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn read(
        &self,
        device_json: String,
        characteristic_json: String,
    ) -> Result<Vec<u8>, RblueError> {
        let characteristic = parse_characteristic_id(&characteristic_json)?;
        Ok(self
            .peripheral(&device_json)
            .await?
            .read(&characteristic)
            .await?)
    }

    /// Writes `data` to the characteristic; `with_response` selects the
    /// acknowledged or the best-effort mode. Oversize payloads are rejected
    /// with [`RblueError::InvalidLength`] - never truncated, never chunked.
    ///
    /// # Errors
    ///
    /// [`RblueError::InvalidLength`], [`RblueError::NotConnected`],
    /// [`RblueError::Att`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn write(
        &self,
        device_json: String,
        characteristic_json: String,
        data: Vec<u8>,
        with_response: bool,
    ) -> Result<(), RblueError> {
        let characteristic = parse_characteristic_id(&characteristic_json)?;
        let mode = if with_response {
            WriteMode::WithResponse
        } else {
            WriteMode::WithoutResponse
        };
        Ok(self
            .peripheral(&device_json)
            .await?
            .write(&characteristic, &data, mode)
            .await?)
    }

    /// Maximum payload `write` accepts for this characteristic and mode, in
    /// bytes. Re-query before long transfers instead of caching.
    ///
    /// # Errors
    ///
    /// [`RblueError::NotConnected`], platform failures as
    /// [`RblueError::Platform`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn max_write_len(
        &self,
        device_json: String,
        characteristic_json: String,
        with_response: bool,
    ) -> Result<u64, RblueError> {
        let characteristic = parse_characteristic_id(&characteristic_json)?;
        let mode = if with_response {
            WriteMode::WithResponse
        } else {
            WriteMode::WithoutResponse
        };
        let len = self
            .peripheral(&device_json)
            .await?
            .max_write_len(&characteristic, mode)
            .await?;
        Ok(len as u64)
    }

    /// Subscribes to value changes of the characteristic. Notification vs.
    /// indication follows the characteristic's properties. Values arrive
    /// through [`EventSink::on_notification`]; a sink installed later picks
    /// the values up from the pump channel. Idempotent.
    ///
    /// When `subscribe` resolves, the backend broadcast is attached - values
    /// published afterwards are never missed, even if the sink is installed
    /// later.
    ///
    /// # Errors
    ///
    /// [`RblueError::NotConnected`], [`RblueError::Att`] when the CCCD write
    /// fails, [`RblueError::InvalidArgument`] for malformed identifiers.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn subscribe(
        &self,
        device_json: String,
        characteristic_json: String,
    ) -> Result<(), RblueError> {
        let device = parse_device_id(&device_json)?;
        let characteristic = parse_characteristic_id(&characteristic_json)?;
        let device_key = canonical_json(&device);
        let characteristic_key = canonical_json(&characteristic);
        let peripheral = self.peripheral(&device_json).await?;
        peripheral.subscribe(&characteristic).await?;

        let first = {
            let mut subscriptions = self
                .subscriptions
                .lock()
                .expect("subscription state must not be poisoned");
            subscriptions
                .entry(device_key.clone())
                .or_default()
                .insert(characteristic_key)
        };
        if first {
            // Wait for the backend broadcast to be attached so the
            // post-`subscribe` guarantee above holds before returning.
            self.ensure_forwarder(Arc::clone(&peripheral), device_key.clone())
                .await;
        }
        Ok(())
    }

    /// Cancels the subscription. Idempotent. When it was the device's last
    /// subscription, its forwarder task is stopped.
    ///
    /// # Errors
    ///
    /// [`RblueError::NotConnected`], platform failures as
    /// [`RblueError::Platform`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn unsubscribe(
        &self,
        device_json: String,
        characteristic_json: String,
    ) -> Result<(), RblueError> {
        let device = parse_device_id(&device_json)?;
        let characteristic = parse_characteristic_id(&characteristic_json)?;
        let device_key = canonical_json(&device);
        let characteristic_key = canonical_json(&characteristic);
        let peripheral = self.peripheral(&device_json).await?;
        let result = peripheral.unsubscribe(&characteristic).await;

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
        Ok(result?)
    }

    /// The device's advertised name, when known. `None` for unknown devices
    /// or platforms without cached names.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn name(&self, device_json: String) -> Option<String> {
        self.peripheral(&device_json)
            .await
            .ok()
            .and_then(|p| p.name())
    }

    /// Connection state of the device.
    ///
    /// # Errors
    ///
    /// Platform failures as [`RblueError::Platform`].
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn is_connected(&self, device_json: String) -> Result<bool, RblueError> {
        Ok(self.peripheral(&device_json).await?.is_connected().await?)
    }

    /// Reads the RSSI of the connected device.
    ///
    /// # Errors
    ///
    /// [`RblueError::Unsupported`] where RSSI is scan-time only.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn rssi(&self, device_json: String) -> Result<i16, RblueError> {
        Ok(self.peripheral(&device_json).await?.rssi().await?)
    }

    /// Current ATT MTU.
    ///
    /// # Errors
    ///
    /// [`RblueError::Unsupported`] where the platform does not expose the
    /// value; use `max_write_len` instead.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn mtu(&self, device_json: String) -> Result<u16, RblueError> {
        Ok(self.peripheral(&device_json).await?.mtu().await?)
    }

    /// Actively initiates pairing/bonding where the platform has an API.
    ///
    /// # Errors
    ///
    /// [`RblueError::Pairing`] with the normalized failure status,
    /// [`RblueError::Unsupported`] on platforms where pairing is implicit.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn pair(&self, device_json: String) -> Result<(), RblueError> {
        Ok(self.peripheral(&device_json).await?.pair().await?)
    }

    /// Web-only entry point: shows the browser's device picker. Must be
    /// called from a user-gesture handler (the TS binding layer provides the
    /// helper for that).
    ///
    /// # Returns
    ///
    /// The picked device as `DeviceId` JSON.
    ///
    /// # Errors
    ///
    /// [`RblueError::Unsupported`] on every non-web platform,
    /// [`RblueError::InvalidArgument`] for malformed JSON.
    #[uniffi::method(async_runtime = "tokio")]
    pub async fn request_device(&self, options_json: String) -> Result<String, RblueError> {
        let options: RequestDeviceOptions =
            parse_with_defaults(&options_json, "RequestDeviceOptions JSON")?;
        let device = self.adapter.request_device(&options).await?;
        Ok(canonical_json(device.id()))
    }

    /// Installs the event sink (replacing any previous one) and starts the
    /// pump on first install. See the crate documentation for the delivery
    /// and backpressure contract, and for the no-blocking/no-throw rules
    /// implementations must follow.
    ///
    /// Values subscribed before the first install stay buffered in the pump
    /// channel (capacity 256, bounded backpressure beyond that) and flush to
    /// the sink once it is installed. Replacing a sink changes nothing else:
    /// channel and forwarders keep running.
    #[uniffi::method]
    pub fn set_event_sink(&self, sink: Box<dyn EventSink>) {
        *self
            .sink
            .lock()
            .expect("event sink state must not be poisoned") = Some(sink);
        let mut pump = self.pump.lock().expect("pump state must not be poisoned");
        if pump.central.is_none() {
            let Some(rx) = pump.rx.take() else {
                return;
            };
            let adapter = Arc::clone(&self.adapter);
            let sink_slot = Arc::clone(&self.sink);
            pump.central = Some(pump_runtime().spawn(async move {
                let mut events = adapter.events();
                let mut rx = rx;
                loop {
                    tokio::select! {
                        event = events.next() => match event {
                            Some(event) => deliver(&sink_slot, PumpMessage::Event(event)),
                            None => break,
                        },
                        message = rx.recv() => match message {
                            Some(message) => deliver(&sink_slot, message),
                            None => break,
                        },
                    }
                }
            }));
        }
    }
}

impl Rblue {
    /// Constructs the facade over an explicit adapter (Rust-side only: the
    /// adapter is not an FFI type). Tests and embedders use this; foreign
    /// code goes through [`Rblue::new`].
    #[must_use]
    pub fn with_adapter(adapter: Arc<dyn Adapter>) -> Arc<Self> {
        let (tx, rx) = tokio::sync::mpsc::channel(PUMP_CHANNEL_CAPACITY);
        Arc::new(Self {
            adapter,
            subscriptions: Mutex::new(HashMap::new()),
            pump: Mutex::new(Pump {
                tx,
                rx: Some(rx),
                central: None,
                forwarders: HashMap::new(),
            }),
            sink: Arc::new(Mutex::new(None)),
        })
    }

    /// Resolves a `DeviceId` JSON argument to the adapter's peripheral
    /// handle.
    ///
    /// # Errors
    ///
    /// [`RblueError::InvalidArgument`] for malformed JSON,
    /// [`RblueError::NotFound`] for an unknown device.
    async fn peripheral(&self, device_json: &str) -> Result<Arc<dyn Peripheral>, RblueError> {
        let device = parse_device_id(device_json)?;
        Ok(self.adapter.peripheral(&device).await?)
    }

    /// Starts the device's notification forwarder unless one is already
    /// running, then resolves once the backend broadcast is attached.
    ///
    /// Runs on the pump runtime; the caller (always `subscribe`) awaits the
    /// attach signal so values published after the call are never missed.
    async fn ensure_forwarder(&self, peripheral: Arc<dyn Peripheral>, device_key: String) {
        let (attached, attach_signal) = tokio::sync::oneshot::channel();
        if !self.spawn_forwarder(peripheral, device_key, Some(attached)) {
            return;
        }
        // The forwarder always answers the signal unless it is aborted
        // (facade teardown), which no caller observes.
        let _ = attach_signal.await;
    }

    /// Spawns the forwarder task for a resolved peripheral; returns `false`
    /// when a live forwarder already exists.
    fn spawn_forwarder(
        &self,
        peripheral: Arc<dyn Peripheral>,
        device_key: String,
        attached: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> bool {
        let mut pump = self.pump.lock().expect("pump state must not be poisoned");
        if pump
            .forwarders
            .get(&device_key)
            .is_some_and(|task| !task.is_finished())
        {
            return false;
        }
        let tx = pump.tx.clone();
        let handle = pump_runtime().spawn(forward(peripheral, tx, device_key.clone(), attached));
        pump.forwarders.insert(device_key, handle);
        true
    }

    /// Stops and forgets the device's notification forwarder, if any.
    fn stop_forwarder(&self, device_key: &str) {
        let mut pump = self.pump.lock().expect("pump state must not be poisoned");
        if let Some(task) = pump.forwarders.remove(device_key) {
            task.abort();
        }
    }
}

impl Drop for Rblue {
    fn drop(&mut self) {
        let mut pump = self.pump.lock().expect("pump state must not be poisoned");
        if let Some(central) = pump.central.take() {
            central.abort();
        }
        for (_, task) in pump.forwarders.drain() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests;
