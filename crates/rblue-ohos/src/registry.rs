//! [`DriverRegistry`]: request correlation, event routing, and the binary
//! data plane of the `BleDriver` protocol.
//!
//! The registry is the Rust heart of the OHOS backend. It is pure Rust (no
//! Node-API) and therefore unit-tested on the host: requests carry a
//! monotonically increasing `req_id` (`u64`), park on a oneshot channel until
//! the driver completes them, and fail with [`rblue_api::Error::Timeout`]
//! after the configured deadline. Driver events are routed into
//! [`rblue_api::AdapterEvent`] broadcasts; notification payloads arrive on
//! the binary data plane (`rblue_driver::frame`), never through JSON.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use futures::channel::oneshot;
use futures::future::{select, Either};
use futures::stream::BoxStream;
use futures_timer::Delay;
use rblue_api::{
    AdapterEvent, AdapterState, CharacteristicId, DeviceId, DiscoveredDevice, Error, Notification,
    Peripheral, Rssi, ScanFilter, ServiceSet,
};
use rblue_driver::{
    decode_frame_header, BleDriverCommand, DriverEvent, DriverRequest, DriverResponse, DriverValue,
    FrameKind, RequestId,
};
use tracing::{debug, warn};

use crate::broadcast::BoundedBroadcast;
use crate::peripheral::OhosPeripheral;
use crate::transport::DriverTransport;

/// Default ATT_MTU per the Bluetooth Core Spec; the negotiated value reported
/// by the driver replaces it after `requestMtu`.
pub(crate) const DEFAULT_ATT_MTU: u16 = 23;

/// Registry configuration.
///
/// The defaults are tuned for interactive GATT use; applications driving
/// long-running procedures (slow connects on flaky links) may raise
/// [`RegistryConfig::request_timeout`].
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    /// Deadline for every driver request. The pending entry is removed and
    /// the awaiting future resolves with [`Error::Timeout`] when the driver
    /// does not complete in time; a late completion is logged and dropped.
    pub request_timeout: Duration,
    /// Per-subscriber capacity of the adapter-event queue. Overflow drops
    /// oldest `DeviceUpdated` values first, never terminal events.
    pub event_capacity: usize,
    /// Per-subscriber capacity of the notification queue (drop-oldest).
    pub notification_capacity: usize,
    /// Bound on queued notification routing entries (JSON event seen, binary
    /// payload not yet arrived). Protects against drivers that emit routing
    /// events without ever sending the paired frame.
    pub notification_route_limit: usize,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(15),
            // The bounded-hub sizes shared with the sibling backends: 64
            // adapter events per subscriber, 32 notification slots.
            event_capacity: 64,
            notification_capacity: 32,
            notification_route_limit: 64,
        }
    }
}

/// One parked request in the pending table.
struct PendingRequest {
    sender: oneshot::Sender<Result<DriverResponse, Error>>,
    /// Device the command targets, when it has one; used to fail in-flight
    /// requests fast on `DeviceDisconnected`.
    device: Option<DeviceId>,
}

/// Shared state of the registry.
pub(crate) struct RegistryInner {
    config: RegistryConfig,
    transport: Mutex<Option<Arc<dyn DriverTransport>>>,
    requests: Mutex<HashMap<RequestId, PendingRequest>>,
    next_request_id: AtomicU64,
    adapter_state: RwLock<AdapterState>,
    scanning: AtomicBool,
    scan_filter: Mutex<Option<ScanFilter>>,
    devices: Mutex<HashMap<DeviceId, Arc<OhosPeripheral>>>,
    /// Insertion order of the device table, so snapshot replays and
    /// `peripherals()` are deterministic (the map itself is unordered).
    device_order: Mutex<Vec<DeviceId>>,
    events: BoundedBroadcast<AdapterEvent>,
    notification_routes: Mutex<VecDeque<(DeviceId, CharacteristicId)>>,
}

/// The Rust side of the `BleDriver` protocol.
///
/// One registry drives one adapter. The host transport is injected once at
/// startup (`registerDriver` in ArkTS terms); requests and completions are
/// correlated by `req_id`, so completions may arrive in any order.
///
/// Cloneable: all clones share one registry state.
#[derive(Clone)]
pub struct DriverRegistry {
    inner: Arc<RegistryInner>,
}

impl DriverRegistry {
    /// Creates a registry with default [`RegistryConfig`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_config(RegistryConfig::default())
    }

    /// Creates a registry with an explicit configuration.
    #[must_use]
    pub fn with_config(config: RegistryConfig) -> Self {
        let event_capacity = config.event_capacity;
        Self {
            inner: Arc::new(RegistryInner {
                config,
                transport: Mutex::new(None),
                requests: Mutex::new(HashMap::new()),
                next_request_id: AtomicU64::new(1),
                adapter_state: RwLock::new(AdapterState::Unknown),
                scanning: AtomicBool::new(false),
                scan_filter: Mutex::new(None),
                devices: Mutex::new(HashMap::new()),
                device_order: Mutex::new(Vec::new()),
                events: BoundedBroadcast::new(event_capacity, event_droppable),
                notification_routes: Mutex::new(VecDeque::new()),
            }),
        }
    }

    /// Registry configuration in effect.
    #[must_use]
    pub fn config(&self) -> &RegistryConfig {
        &self.inner.config
    }

    /// Shared inner state (crate-internal seam for the adapter/peripheral
    /// implementations).
    #[must_use]
    pub(crate) fn inner(&self) -> &Arc<RegistryInner> {
        &self.inner
    }

    /// Wraps existing inner state (crate-internal seam used by
    /// `OhosAdapter::registry`).
    #[must_use]
    pub(crate) fn from_inner(inner: Arc<RegistryInner>) -> Self {
        Self { inner }
    }

    /// Injects the host transport. Called once at startup by the napi facade
    /// (`registerDriver`); tests inject a recording or scripted transport.
    ///
    /// Registering again replaces the previous transport (driver restart);
    /// the protocol version gate belongs to the caller (see `DriverHello`).
    pub fn register_transport(&self, transport: Arc<dyn DriverTransport>) {
        *self.inner.lock_transport() = Some(transport);
    }

    /// Currently registered transport, if any.
    #[must_use]
    pub fn transport(&self) -> Option<Arc<dyn DriverTransport>> {
        self.inner.lock_transport().clone()
    }

    /// Sends `command` to the driver without a data-plane payload and awaits
    /// the completion.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidState`] when no transport is registered,
    /// [`Error::Timeout`] when the driver misses the deadline, the transport
    /// error itself when delivery fails, and [`Error::Platform`] (with the
    /// raw code preserved) for driver-reported failures.
    pub async fn request(&self, command: BleDriverCommand) -> Result<DriverValue, Error> {
        self.request_with_payload(command, &[]).await
    }

    /// [`DriverRegistry::request`] with a binary data-plane payload (the
    /// write bytes of a `Write` command).
    pub async fn request_with_payload(
        &self,
        command: BleDriverCommand,
        payload: &[u8],
    ) -> Result<DriverValue, Error> {
        self.inner.request_with_payload(command, payload).await
    }

    /// Completes a pending request with the driver's response.
    ///
    /// Responses for unknown or already timed-out request ids are logged and
    /// dropped (the awaiting side has already failed with
    /// [`Error::Timeout`]); this is an expected race, not an error.
    pub fn complete_request(&self, response: DriverResponse) {
        let req_id = match &response {
            DriverResponse::Success { req_id, .. } | DriverResponse::Failure { req_id, .. } => {
                *req_id
            }
        };
        let pending = self.inner.lock_requests().remove(&req_id);
        match pending {
            Some(pending) => {
                let _receiver_gone = pending.sender.send(Ok(response));
            }
            None => debug!(req_id, "response for unknown or timed-out request"),
        }
    }

    /// Ingests one binary data-plane frame from the driver.
    ///
    /// `ReadResult` frames complete the pending read request with their
    /// payload (exactly one completion per request: either this frame or a
    /// control-plane response, never both). `Notification` frames deliver the
    /// payload of the characteristic announced by the most recent
    /// zero-data `DriverEvent::Notification` routing event (the driver must
    /// emit the pair in order on one channel, which the napi bridge
    /// guarantees). `WritePayload` frames are host-to-core only and rejected.
    ///
    /// # Errors
    ///
    /// [`Error::Backend`] for undecodable frames and wrong-direction frames.
    pub fn handle_frame(&self, buf: &[u8]) -> Result<(), Error> {
        let (header, payload) = decode_frame_header(buf)
            .map_err(|err| Error::Backend(format!("invalid driver frame: {err}")))?;
        match header.kind {
            FrameKind::ReadResult => {
                self.complete_request(DriverResponse::Success {
                    req_id: header.req_id,
                    value: DriverValue::Data {
                        data: payload.to_vec(),
                    },
                });
                Ok(())
            }
            FrameKind::Notification => match self.inner.lock_notification_routes().pop_front() {
                Some((device, characteristic)) => {
                    self.deliver_notification(device, characteristic, payload.to_vec());
                    Ok(())
                }
                None => {
                    warn!("notification frame without a paired routing event; dropped");
                    Ok(())
                }
            },
            FrameKind::WritePayload => Err(Error::Backend(
                "inbound WritePayload frame, but frames of that kind are host-to-core only"
                    .to_owned(),
            )),
        }
    }

    /// Ingests one control-plane event from the driver (`emitEvent`).
    ///
    /// Updates the registry's device table and adapter state, applies the
    /// active scan filter's software-only fields (name prefix, RSSI
    /// threshold) to discovery traffic, and broadcasts the mapped
    /// [`AdapterEvent`].
    pub fn emit_event(&self, event: DriverEvent) {
        match event {
            DriverEvent::StateChanged { state } => {
                *self
                    .inner
                    .adapter_state
                    .write()
                    .expect("adapter state lock poisoned") = state;
                self.broadcast(AdapterEvent::StateChanged(state));
            }
            DriverEvent::DeviceDiscovered {
                device,
                advertisement,
            } => {
                if !self.inner.passes_software_filter(&advertisement) {
                    return;
                }
                let discovered = DiscoveredDevice {
                    id: device.clone(),
                    advertisement: advertisement.clone(),
                };
                match self.inner.record_advertisement(&device, advertisement) {
                    AdvertisementOutcome::New => {
                        self.broadcast(AdapterEvent::DeviceDiscovered(discovered));
                    }
                    AdvertisementOutcome::Known { rssi: Some(rssi) } => {
                        self.broadcast(AdapterEvent::DeviceUpdated(device, Rssi::new(rssi)));
                    }
                    AdvertisementOutcome::Known { rssi: None } => {}
                }
            }
            DriverEvent::DeviceUpdated { device, rssi } => {
                let known = {
                    let devices = self.inner.lock_devices();
                    match devices.get(&device) {
                        Some(existing) => {
                            existing.update_rssi(rssi);
                            true
                        }
                        None => false,
                    }
                };
                if known {
                    self.broadcast(AdapterEvent::DeviceUpdated(device, Rssi::new(rssi)));
                } else {
                    // The snapshot-then-live contract needs every subscriber
                    // to see a discovery first; synthesize one from the
                    // update.
                    let advertisement = rblue_api::AdvertisementData {
                        rssi: Some(rssi),
                        ..Default::default()
                    };
                    self.inner
                        .record_advertisement(&device, advertisement.clone());
                    self.broadcast(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                        id: device,
                        advertisement,
                    }));
                }
            }
            DriverEvent::DeviceConnected { device } => {
                let peripheral = self.inner.ensure_peripheral(&device);
                peripheral.set_connected(true);
                self.broadcast(AdapterEvent::DeviceConnected(device));
            }
            DriverEvent::DeviceDisconnected { device, reason } => {
                if let Some(peripheral) = self.inner.lock_devices().get(&device).cloned() {
                    peripheral.set_connected(false);
                }
                self.fail_requests_for_device(&device);
                self.broadcast(AdapterEvent::DeviceDisconnected(device, reason));
            }
            DriverEvent::Notification {
                device,
                characteristic,
                data,
            } => {
                if data.is_empty() {
                    // Payload rides the binary frame; park the routing key
                    // for the paired `handle_frame` call.
                    let mut routes = self.inner.lock_notification_routes();
                    if routes.len() >= self.inner.config.notification_route_limit {
                        routes.pop_front();
                    }
                    routes.push_back((device, characteristic));
                } else {
                    self.deliver_notification(device, characteristic, data);
                }
            }
        }
    }

    /// Subscribes to the adapter event stream. The stream first replays every
    /// device known so far as a fresh `DeviceDiscovered` (snapshot), then
    /// continues with live events.
    pub fn subscribe_events(&self) -> BoxStream<'static, AdapterEvent> {
        self.inner.subscribe_events()
    }

    /// Current adapter state as reported through `StateChanged` events and
    /// the `initialize` handshake.
    #[must_use]
    pub fn adapter_state(&self) -> AdapterState {
        self.inner.adapter_state()
    }

    /// `true` while a scan started through the adapter is still running.
    #[must_use]
    pub fn is_scanning(&self) -> bool {
        self.inner.is_scanning()
    }

    /// Snapshot of all devices known to the registry: scan history plus
    /// connected devices.
    #[must_use]
    pub fn known_peripherals(&self) -> Vec<Arc<dyn Peripheral>> {
        self.inner.known_peripherals()
    }

    /// Handle for a known device.
    ///
    /// # Errors
    ///
    /// [`Error::NotFound`] when the device was never seen.
    pub fn peripheral(&self, id: &DeviceId) -> Result<Arc<OhosPeripheral>, Error> {
        self.inner.peripheral(id)
    }

    pub(crate) fn broadcast(&self, event: AdapterEvent) {
        self.inner.events.push(&event);
    }

    pub(crate) fn deliver_notification(
        &self,
        device: DeviceId,
        characteristic: CharacteristicId,
        data: Vec<u8>,
    ) {
        let peripheral = self.inner.lock_devices().get(&device).cloned();
        match peripheral {
            Some(peripheral) => peripheral.push_notification(Notification {
                id: characteristic,
                data,
            }),
            None => warn!("notification for unknown device dropped"),
        }
    }

    /// Fails every in-flight request targeting `device` with
    /// [`Error::Disconnected`] so callers do not wait out the full deadline.
    fn fail_requests_for_device(&self, device: &DeviceId) {
        let mut requests = self.inner.lock_requests();
        let orphaned: Vec<RequestId> = requests
            .iter()
            .filter(|(_, pending)| pending.device.as_ref() == Some(device))
            .map(|(req_id, _)| *req_id)
            .collect();
        for req_id in orphaned {
            if let Some(pending) = requests.remove(&req_id) {
                let _receiver_gone = pending.sender.send(Err(Error::Disconnected));
            }
        }
    }
}

impl Default for DriverRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("config", &self.inner.config)
            .field("adapter_state", &self.inner.adapter_state)
            .field("scanning", &self.inner.scanning)
            .finish_non_exhaustive()
    }
}

/// Result of recording an advertisement in the device table.
enum AdvertisementOutcome {
    New,
    Known {
        /// RSSI of the freshly stored advertisement, when it carries one.
        rssi: Option<i16>,
    },
}

impl RegistryInner {
    pub(crate) fn lock_transport(&self) -> MutexGuard<'_, Option<Arc<dyn DriverTransport>>> {
        self.transport
            .lock()
            .expect("transport slot poisoned by a panicking registrar")
    }

    /// Notification queue capacity for new peripherals.
    pub(crate) fn notification_capacity(&self) -> usize {
        self.config.notification_capacity
    }

    fn lock_requests(&self) -> MutexGuard<'_, HashMap<RequestId, PendingRequest>> {
        self.requests
            .lock()
            .expect("request table poisoned by a panicking request")
    }

    pub(crate) fn lock_devices(&self) -> MutexGuard<'_, HashMap<DeviceId, Arc<OhosPeripheral>>> {
        self.devices
            .lock()
            .expect("device table poisoned by a panicking event")
    }

    fn lock_notification_routes(&self) -> MutexGuard<'_, VecDeque<(DeviceId, CharacteristicId)>> {
        self.notification_routes
            .lock()
            .expect("notification route queue poisoned by a panicking event")
    }

    /// Sends `command` without a data-plane payload and awaits completion.
    pub(crate) async fn request(&self, command: BleDriverCommand) -> Result<DriverValue, Error> {
        self.request_with_payload(command, &[]).await
    }

    /// Sends `command` with a binary data-plane payload and awaits
    /// completion. Locks are never held across the await: the pending entry
    /// is parked first, the transport call is lock-free, and the timeout
    /// cleanup re-acquires the table.
    pub(crate) async fn request_with_payload(
        &self,
        command: BleDriverCommand,
        payload: &[u8],
    ) -> Result<DriverValue, Error> {
        let transport = self
            .lock_transport()
            .clone()
            .ok_or_else(|| Error::InvalidState("no driver transport registered".to_owned()))?;
        let device = command_device(&command);
        let requests_mtu = matches!(command, BleDriverCommand::RequestMtu { .. });
        let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.lock_requests().insert(
            req_id,
            PendingRequest {
                sender,
                device: device.clone(),
            },
        );
        let request = DriverRequest { req_id, command };
        if let Err(err) = transport.send(request, payload) {
            self.lock_requests().remove(&req_id);
            return Err(err);
        }
        debug!(req_id, "driver request dispatched");
        let deadline = Delay::new(self.config.request_timeout);
        match select(receiver, deadline).await {
            Either::Left((completion, _)) => match completion {
                Ok(Ok(response)) => {
                    let value = response.into_result()?;
                    if requests_mtu {
                        self.apply_mtu_response(&device, &value);
                    }
                    Ok(value)
                }
                Ok(Err(err)) => Err(err),
                Err(_canceled) => Err(Error::Backend(
                    "driver request canceled before completion".to_owned(),
                )),
            },
            Either::Right(_) => {
                self.lock_requests().remove(&req_id);
                Err(Error::Timeout)
            }
        }
    }

    /// Records the negotiated MTU of a completed `RequestMtu` command in the
    /// session state, so `Peripheral::mtu` and `max_write_len` track it.
    fn apply_mtu_response(&self, device: &Option<DeviceId>, value: &DriverValue) {
        if let (Some(device), DriverValue::Mtu { mtu }) = (device, value) {
            if let Some(peripheral) = self.lock_devices().get(device) {
                peripheral.set_mtu(*mtu);
            }
        }
    }

    /// Applies the software-only scan filter fields (`name_prefix`,
    /// `rssi_threshold`); everything else is pushed down to the driver with
    /// the `StartScan` command.
    pub(crate) fn passes_software_filter(
        &self,
        advertisement: &rblue_api::AdvertisementData,
    ) -> bool {
        let filter = self.scan_filter.lock().expect("scan filter lock poisoned");
        let Some(filter) = filter.as_ref() else {
            return true;
        };
        if let Some(prefix) = &filter.name_prefix {
            let matches = advertisement
                .local_name
                .as_ref()
                .is_some_and(|name| name.starts_with(prefix));
            if !matches {
                return false;
            }
        }
        if let Some(threshold) = filter.rssi_threshold {
            let matches = advertisement.rssi.is_some_and(|rssi| rssi >= threshold);
            if !matches {
                return false;
            }
        }
        true
    }

    /// Inserts or updates the device table entry for `device`.
    fn record_advertisement(
        self: &Arc<Self>,
        device: &DeviceId,
        advertisement: rblue_api::AdvertisementData,
    ) -> AdvertisementOutcome {
        let mut devices = self.lock_devices();
        match devices.get(device) {
            Some(existing) => {
                existing.update_advertisement(&advertisement);
                AdvertisementOutcome::Known {
                    rssi: advertisement.rssi,
                }
            }
            None => {
                devices.insert(
                    device.clone(),
                    OhosPeripheral::new(self, device.clone(), advertisement),
                );
                drop(devices);
                self.record_device_order(device);
                AdvertisementOutcome::New
            }
        }
    }

    /// Appends `device` to the insertion-order ledger when not present.
    fn record_device_order(&self, device: &DeviceId) {
        let mut order = self
            .device_order
            .lock()
            .expect("device order poisoned by a panicking event");
        if !order.iter().any(|known| known == device) {
            order.push(device.clone());
        }
    }

    /// Returns the peripheral handle, creating a bare entry for unknown
    /// devices (drivers may report connection changes for devices the core
    /// has not seen advertised).
    pub(crate) fn ensure_peripheral(self: &Arc<Self>, device: &DeviceId) -> Arc<OhosPeripheral> {
        let created = {
            let mut devices = self.lock_devices();
            match devices.get(device) {
                Some(existing) => return existing.clone(),
                None => {
                    let peripheral = OhosPeripheral::new(
                        self,
                        device.clone(),
                        rblue_api::AdvertisementData::default(),
                    );
                    devices.insert(device.clone(), Arc::clone(&peripheral));
                    true
                }
            }
        };
        if created {
            self.record_device_order(device);
        }
        self.peripheral(device).expect("entry was just inserted")
    }

    /// Snapshot-then-live event subscription; see [`DriverRegistry::subscribe_events`].
    pub(crate) fn subscribe_events(&self) -> BoxStream<'static, AdapterEvent> {
        // The snapshot replays in device insertion order (first sighting
        // first), matching the mock's contract; the devices lock is held so
        // no advertisement can slip between subscription and replay.
        let devices = self.lock_devices();
        let subscription = self.events.subscribe();
        for id in self
            .device_order
            .lock()
            .expect("device order poisoned by a panicking event")
            .iter()
        {
            if let Some(peripheral) = devices.get(id) {
                subscription.push_snapshot(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                    id: id.clone(),
                    advertisement: peripheral.advertisement(),
                }));
            }
        }
        drop(devices);
        Box::pin(subscription)
    }

    /// Current adapter state.
    pub(crate) fn adapter_state(&self) -> AdapterState {
        *self
            .adapter_state
            .read()
            .expect("adapter state lock poisoned")
    }

    /// Updates the adapter state (from `initialize` or `StateChanged`).
    pub(crate) fn set_adapter_state(&self, state: AdapterState) {
        *self
            .adapter_state
            .write()
            .expect("adapter state lock poisoned") = state;
    }

    /// Whether a scan started through the adapter is running.
    pub(crate) fn is_scanning(&self) -> bool {
        self.scanning.load(Ordering::Relaxed)
    }

    pub(crate) fn set_scanning(&self, scanning: bool) {
        self.scanning.store(scanning, Ordering::Relaxed);
    }

    /// Sets the active scan filter used for software-side filtering of
    /// discovery events.
    pub(crate) fn set_scan_filter(&self, filter: Option<ScanFilter>) {
        *self.scan_filter.lock().expect("scan filter lock poisoned") = filter;
    }

    /// Snapshot of the known devices, in insertion order.
    pub(crate) fn known_peripherals(&self) -> Vec<Arc<dyn Peripheral>> {
        let devices = self.lock_devices();
        self.device_order
            .lock()
            .expect("device order poisoned by a panicking event")
            .iter()
            .filter_map(|id| devices.get(id).cloned())
            .map(|peripheral| peripheral as Arc<dyn Peripheral>)
            .collect()
    }

    /// Handle for a known device.
    pub(crate) fn peripheral(&self, id: &DeviceId) -> Result<Arc<OhosPeripheral>, Error> {
        self.lock_devices()
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(id.clone()))
    }
}

/// Device a command targets, when it has one.
fn command_device(command: &BleDriverCommand) -> Option<DeviceId> {
    match command {
        BleDriverCommand::Connect { device }
        | BleDriverCommand::Disconnect { device }
        | BleDriverCommand::DiscoverServices { device }
        | BleDriverCommand::Read { device, .. }
        | BleDriverCommand::Write { device, .. }
        | BleDriverCommand::Subscribe { device, .. }
        | BleDriverCommand::Unsubscribe { device, .. }
        | BleDriverCommand::ReadRssi { device }
        | BleDriverCommand::RequestMtu { device, .. } => Some(device.clone()),
        BleDriverCommand::Init
        | BleDriverCommand::GetAdapterState
        | BleDriverCommand::StartScan { .. }
        | BleDriverCommand::StopScan => None,
    }
}

/// Terminal adapter events must never be dropped in favor of newer traffic.
fn event_droppable(event: &AdapterEvent) -> bool {
    matches!(event, AdapterEvent::DeviceUpdated(..))
}

/// Unwraps a `DriverValue::None`, mapping any other kind to a protocol
/// violation.
pub(crate) fn expect_none(value: DriverValue) -> Result<(), Error> {
    match value {
        DriverValue::None => Ok(()),
        other => Err(unexpected_value("none", &other)),
    }
}

/// Unwraps a `DriverValue::Data`.
pub(crate) fn expect_data(value: DriverValue) -> Result<Vec<u8>, Error> {
    match value {
        DriverValue::Data { data } => Ok(data),
        other => Err(unexpected_value("data", &other)),
    }
}

/// Unwraps a `DriverValue::Services`.
pub(crate) fn expect_services(value: DriverValue) -> Result<ServiceSet, Error> {
    match value {
        DriverValue::Services { services } => Ok(services),
        other => Err(unexpected_value("services", &other)),
    }
}

/// Unwraps a `DriverValue::AdapterState`.
pub(crate) fn expect_adapter_state(value: DriverValue) -> Result<AdapterState, Error> {
    match value {
        DriverValue::AdapterState { state } => Ok(state),
        other => Err(unexpected_value("adapterState", &other)),
    }
}

fn unexpected_value(expected: &str, got: &DriverValue) -> Error {
    Error::Backend(format!(
        "driver returned {got:?} where {expected} was expected"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::OhosAdapter;
    use crate::peripheral::ATT_WRITE_NOT_PERMITTED;
    use crate::transport::RecordingTransport;
    use futures::future::join_all;
    use futures::{FutureExt, StreamExt};
    use rblue_api::{Adapter, AddressType, CharacteristicId, ServiceId, WriteMode};
    use rblue_driver::{encode_frame_header, FrameHeader, FRAME_HEADER_LEN};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Weak;

    const DEVICE: DeviceId =
        DeviceId::Mac([0x02, 0x00, 0x00, 0xCA, 0xFE, 0x01], AddressType::Public);
    const OTHER_DEVICE: DeviceId =
        DeviceId::Mac([0x02, 0x00, 0x00, 0xCA, 0xFE, 0x02], AddressType::Public);
    const SERVICE_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x1234);
    const CHARACTERISTIC_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x5678);
    const CHARACTERISTIC: CharacteristicId = CharacteristicId {
        service: ServiceId {
            uuid: SERVICE_UUID,
            instance: 0,
        },
        uuid: CHARACTERISTIC_UUID,
        instance: 0,
    };
    /// The read/write (non-notifying) characteristic of the test tree.
    const WRITABLE: CharacteristicId = CharacteristicId {
        service: ServiceId {
            uuid: SERVICE_UUID,
            instance: 0,
        },
        uuid: uuid::Uuid::from_u128(0x9999),
        instance: 0,
    };

    /// The two-characteristic test tree: a read/write-only one and a notify
    /// one.
    fn test_service_set() -> rblue_api::ServiceSet {
        use rblue_api::{Characteristic, CharacteristicProperties, Service};
        rblue_api::ServiceSet {
            services: vec![Service {
                uuid: SERVICE_UUID,
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: WRITABLE.uuid,
                        properties: CharacteristicProperties::READ
                            | CharacteristicProperties::WRITE,
                        descriptors: Vec::new(),
                    },
                    Characteristic {
                        uuid: CHARACTERISTIC_UUID,
                        properties: CharacteristicProperties::READ
                            | CharacteristicProperties::NOTIFY,
                        descriptors: Vec::new(),
                    },
                ],
            }],
        }
    }

    /// Completes a `DiscoverServices` request with [`test_service_set`].
    fn complete_discovery(core: &DriverRegistry, req_id: RequestId) {
        core.complete_request(DriverResponse::Success {
            req_id,
            value: DriverValue::Services {
                services: test_service_set(),
            },
        });
    }

    /// Builds one binary data-plane frame the way the napi transport would.
    fn test_frame(req_id: RequestId, kind: FrameKind, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0u8; FRAME_HEADER_LEN];
        encode_frame_header(
            FrameHeader {
                req_id,
                kind,
                payload_len: payload.len() as u32,
            },
            &mut frame,
        )
        .expect("test frame header");
        frame.extend_from_slice(payload);
        frame
    }

    fn advertisement(name: &str, rssi: i16) -> rblue_api::AdvertisementData {
        rblue_api::AdvertisementData {
            local_name: Some(name.to_owned()),
            rssi: Some(rssi),
            ..Default::default()
        }
    }

    /// Behavior script of a [`ScriptedTransport`].
    type ScriptedBehavior = dyn FnMut(&DriverRequest, &[u8], &DriverRegistry) + Send;

    /// Transport whose behavior is scripted per request; stands in for the
    /// ArkTS driver in host tests.
    struct ScriptedTransport {
        registry: Mutex<Weak<DriverRegistry>>,
        behavior: Mutex<Box<ScriptedBehavior>>,
    }

    impl ScriptedTransport {
        fn new(
            registry: &Arc<DriverRegistry>,
            behavior: impl FnMut(&DriverRequest, &[u8], &DriverRegistry) + Send + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                registry: Mutex::new(Arc::downgrade(registry)),
                behavior: Mutex::new(Box::new(behavior)),
            })
        }
    }

    impl DriverTransport for ScriptedTransport {
        fn send(&self, request: DriverRequest, payload: &[u8]) -> Result<(), Error> {
            let registry = self
                .registry
                .lock()
                .expect("registry slot poisoned")
                .upgrade()
                .ok_or_else(|| Error::Backend("registry gone".to_owned()))?;
            (self.behavior.lock().expect("behavior poisoned"))(&request, payload, &registry);
            Ok(())
        }
    }

    /// Registers a transport that completes every request with
    /// `DriverValue::None`.
    fn none_responder(registry: &Arc<DriverRegistry>) {
        registry.register_transport(ScriptedTransport::new(registry, |request, _, core| {
            core.complete_request(DriverResponse::Success {
                req_id: request.req_id,
                value: DriverValue::None,
            });
        }));
    }

    /// Registers a transport that answers discovery with the test tree and
    /// every other request with `DriverValue::None`.
    fn discover_with_tree(registry: &Arc<DriverRegistry>) {
        registry.register_transport(ScriptedTransport::new(registry, |request, _, core| {
            if matches!(request.command, BleDriverCommand::DiscoverServices { .. }) {
                complete_discovery(core, request.req_id);
            } else {
                core.complete_request(DriverResponse::Success {
                    req_id: request.req_id,
                    value: DriverValue::None,
                });
            }
        }));
    }

    fn connected_registry() -> (Arc<DriverRegistry>, Arc<OhosPeripheral>) {
        let registry = Arc::new(DriverRegistry::new());
        none_responder(&registry);
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        (registry, peripheral)
    }

    #[tokio::test]
    async fn request_completes_with_driver_value() {
        let registry = Arc::new(DriverRegistry::new());
        registry.register_transport(ScriptedTransport::new(
            &registry,
            |request, payload, core| {
                core.complete_request(DriverResponse::Success {
                    req_id: request.req_id,
                    value: DriverValue::Data {
                        data: payload.to_vec(),
                    },
                });
            },
        ));
        let value = registry
            .request_with_payload(
                BleDriverCommand::Write {
                    device: DEVICE,
                    characteristic: CHARACTERISTIC,
                    with_response: true,
                },
                b"ping",
            )
            .await
            .expect("request");
        assert_eq!(
            value,
            DriverValue::Data {
                data: b"ping".to_vec()
            }
        );
    }

    #[tokio::test]
    async fn requests_correlate_when_completed_out_of_order() {
        let registry = Arc::new(DriverRegistry::new());
        // The driver completes every group of 7 parked requests in reverse
        // order once the next request arrives.
        let parked: Arc<Mutex<Vec<RequestId>>> = Arc::new(Mutex::new(Vec::new()));
        {
            let parked = Arc::clone(&parked);
            registry.register_transport(ScriptedTransport::new(
                &registry,
                move |request, _, core| {
                    let mut parked = parked.lock().expect("parked requests poisoned");
                    parked.push(request.req_id);
                    if parked.len() >= 7 {
                        for req_id in parked.drain(..).rev() {
                            core.complete_request(DriverResponse::Success {
                                req_id,
                                value: DriverValue::Data {
                                    data: req_id.to_le_bytes().to_vec(),
                                },
                            });
                        }
                    }
                },
            ));
        }
        let futures: Vec<_> = (0..21)
            .map(|_| Box::pin(registry.request(BleDriverCommand::Init)))
            .collect();
        let results = join_all(futures).await;
        let mut issued: Vec<RequestId> = (1..=21).collect();
        for value in results {
            let DriverValue::Data { data } = value.expect("request") else {
                panic!("expected data value");
            };
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data);
            let req_id = u64::from_le_bytes(bytes);
            let position = issued
                .iter()
                .position(|id| *id == req_id)
                .expect("every completed req_id must match an issued request");
            issued.remove(position);
        }
        assert!(
            issued.is_empty(),
            "every request must complete exactly once"
        );
    }

    #[tokio::test]
    async fn timeout_cleans_pending_table_and_ignores_late_completion() {
        let registry = Arc::new(DriverRegistry::with_config(RegistryConfig {
            request_timeout: Duration::from_millis(50),
            ..RegistryConfig::default()
        }));
        let recording = Arc::new(RecordingTransport::new());
        registry.register_transport(Arc::clone(&recording) as Arc<dyn DriverTransport>);
        let result = registry.request(BleDriverCommand::Init).await;
        assert!(matches!(result, Err(Error::Timeout)));
        // The late completion must be dropped silently, not panic.
        registry.complete_request(DriverResponse::Success {
            req_id: 1,
            value: DriverValue::None,
        });
        assert_eq!(recording.len(), 1);
        // The next request proceeds with a fresh id.
        assert!(matches!(
            registry.request(BleDriverCommand::Init).await,
            Err(Error::Timeout)
        ));
        assert_eq!(recording.sent()[1].request.req_id, 2);
    }

    #[tokio::test]
    async fn transport_error_propagates_and_cleans_up() {
        struct FailingTransport;
        impl DriverTransport for FailingTransport {
            fn send(&self, _request: DriverRequest, _payload: &[u8]) -> Result<(), Error> {
                Err(Error::Backend("delivery failed".to_owned()))
            }
        }
        let registry = Arc::new(DriverRegistry::with_config(RegistryConfig {
            request_timeout: Duration::from_millis(50),
            ..RegistryConfig::default()
        }));
        registry.register_transport(Arc::new(FailingTransport));
        let result = registry.request(BleDriverCommand::Init).await;
        assert!(matches!(result, Err(Error::Backend(message)) if message == "delivery failed"));
        // The pending entry was removed; a working transport takes over and
        // the next request gets the next id.
        none_responder(&registry);
        assert!(registry.request(BleDriverCommand::Init).await.is_ok());
        let recording = Arc::new(RecordingTransport::new());
        registry.register_transport(Arc::clone(&recording) as Arc<dyn DriverTransport>);
        let _ignored = registry.request(BleDriverCommand::Init).await;
        assert_eq!(recording.sent()[0].request.req_id, 3);
    }

    #[tokio::test]
    async fn failure_response_preserves_platform_code() {
        let registry = Arc::new(DriverRegistry::new());
        registry.register_transport(ScriptedTransport::new(&registry, |request, _, core| {
            core.complete_request(DriverResponse::Failure {
                req_id: request.req_id,
                code: 290_003,
                message: "Bluetooth switch is off".to_owned(),
            });
        }));
        let result = registry
            .request(BleDriverCommand::StartScan {
                filter: ScanFilter::default(),
            })
            .await;
        match result {
            Err(Error::Platform { code, message }) => {
                assert_eq!(code, 290_003);
                assert_eq!(message, "Bluetooth switch is off");
            }
            other => panic!("expected Error::Platform, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_without_transport_is_invalid_state() {
        let registry = DriverRegistry::new();
        let result = registry.request(BleDriverCommand::Init).await;
        assert!(matches!(result, Err(Error::InvalidState(_))));
    }

    #[tokio::test]
    async fn read_result_frame_completes_pending_read() {
        let registry = Arc::new(DriverRegistry::new());
        registry.register_transport(ScriptedTransport::new(&registry, |request, _, core| {
            // The driver answers a read with the binary frame alone.
            let frame = test_frame(request.req_id, FrameKind::ReadResult, &[7, 8, 9]);
            core.handle_frame(&frame).expect("frame");
        }));
        let value = registry
            .request(BleDriverCommand::Read {
                device: DEVICE,
                characteristic: CHARACTERISTIC,
            })
            .await
            .expect("read");
        assert_eq!(
            value,
            DriverValue::Data {
                data: vec![7, 8, 9]
            }
        );
    }

    #[tokio::test]
    async fn undecodable_and_wrong_direction_frames_error() {
        let registry = DriverRegistry::new();
        assert!(registry.handle_frame(&[0u8; 5]).is_err());
        // Valid header, but WritePayload is host-to-core only.
        assert!(registry
            .handle_frame(&test_frame(0, FrameKind::WritePayload, b"x"))
            .is_err());
    }

    #[tokio::test]
    async fn notification_frame_routes_through_paired_event() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        peripheral
            .subscribe(&CHARACTERISTIC)
            .await
            .expect("subscribe");
        let mut notifications = peripheral.notifications();
        // Routing event (JSON, empty data) followed by the paired frame.
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: Vec::new(),
        });
        registry
            .handle_frame(&test_frame(0, FrameKind::Notification, b"hello"))
            .expect("frame");
        let notification = notifications.next().await.expect("notification");
        assert_eq!(notification.id, CHARACTERISTIC);
        assert_eq!(notification.data, b"hello");
    }

    #[tokio::test]
    async fn notification_event_with_inline_data_needs_no_frame() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        peripheral
            .subscribe(&CHARACTERISTIC)
            .await
            .expect("subscribe");
        let mut notifications = peripheral.notifications();
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"inline".to_vec(),
        });
        let notification = notifications.next().await.expect("notification");
        assert_eq!(notification.data, b"inline");
        // No routing entry was parked for a frame; a stray frame is dropped
        // with a warning instead of misrouting.
        registry
            .handle_frame(&test_frame(0, FrameKind::Notification, b"stray"))
            .expect("stray frame is reported, not propagated");
    }

    #[tokio::test]
    async fn unsubscribed_characteristic_values_are_not_delivered() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        let mut notifications = peripheral.notifications();
        // No subscribe ran: the driver-side value (inline here) is dropped.
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"unsubscribed".to_vec(),
        });
        registry
            .handle_frame(&test_frame(0, FrameKind::Notification, b"framed"))
            .expect("frame accepted, delivery gated");
        assert!(
            notifications.next().now_or_never().is_none(),
            "values must not flow before subscribe"
        );
    }

    #[tokio::test]
    async fn disconnect_clears_subscriptions_until_resubscribed() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        peripheral
            .subscribe(&CHARACTERISTIC)
            .await
            .expect("subscribe");
        let mut notifications = peripheral.notifications();

        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"before".to_vec(),
        });
        let notification = notifications.next().await.expect("subscribed delivery");
        assert_eq!(notification.data, b"before");

        // The link drops: CCCD state dies with it, so a subsequent value is
        // not delivered even though the stream is still alive.
        registry.emit_event(DriverEvent::DeviceDisconnected {
            device: DEVICE,
            reason: None,
        });
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"while-down".to_vec(),
        });
        assert!(
            notifications.next().now_or_never().is_none(),
            "subscription state must be cleared on disconnect"
        );

        // Reconnect: the session is fresh, discovery must run again, and
        // notifications stay gated until the resubscribe completes.
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"after-reconnect".to_vec(),
        });
        assert!(
            notifications.next().now_or_never().is_none(),
            "CCCD state must not survive a reconnect"
        );
        peripheral.discover_services(&[]).await.expect("rediscover");
        peripheral
            .subscribe(&CHARACTERISTIC)
            .await
            .expect("resubscribe");
        registry.emit_event(DriverEvent::Notification {
            device: DEVICE,
            characteristic: CHARACTERISTIC,
            data: b"after".to_vec(),
        });
        let notification = notifications.next().await.expect("resubscribed delivery");
        assert_eq!(notification.data, b"after");
    }

    #[tokio::test]
    async fn events_snapshot_then_live() {
        let registry = DriverRegistry::new();
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("first", -40),
        });
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: OTHER_DEVICE,
            advertisement: advertisement("second", -60),
        });
        let mut events = registry.subscribe_events();
        // Snapshot: both known devices replay as fresh discoveries.
        let AdapterEvent::DeviceDiscovered(discovered) = events.next().await.expect("snapshot")
        else {
            panic!("expected snapshot discovery");
        };
        assert_eq!(discovered.id, DEVICE);
        let AdapterEvent::DeviceDiscovered(discovered) = events.next().await.expect("snapshot")
        else {
            panic!("expected snapshot discovery");
        };
        assert_eq!(discovered.id, OTHER_DEVICE);
        // Live: an update flows to the already-subscribed stream.
        registry.emit_event(DriverEvent::DeviceUpdated {
            device: DEVICE,
            rssi: -70,
        });
        let AdapterEvent::DeviceUpdated(id, rssi) = events.next().await.expect("live") else {
            panic!("expected live update");
        };
        assert_eq!(id, DEVICE);
        assert_eq!(rssi.as_i16(), -70);
    }

    #[tokio::test]
    async fn repeated_discovery_becomes_device_updated() {
        let registry = DriverRegistry::new();
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("first", -40),
        });
        let mut events = registry.subscribe_events();
        let _snapshot = events.next().await.expect("snapshot");
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("first", -45),
        });
        let AdapterEvent::DeviceUpdated(id, rssi) = events.next().await.expect("update") else {
            panic!("expected DeviceUpdated for a known device");
        };
        assert_eq!(id, DEVICE);
        assert_eq!(rssi.as_i16(), -45);
    }

    #[tokio::test]
    async fn scan_filter_software_fields_apply_to_discoveries() {
        let registry = Arc::new(DriverRegistry::new());
        none_responder(&registry);
        let adapter = OhosAdapter::new(&registry);
        adapter
            .start_scan(&ScanFilter {
                name_prefix: Some("rblue-".to_owned()),
                rssi_threshold: Some(-60),
                ..ScanFilter::default()
            })
            .await
            .expect("start scan");
        assert!(registry.is_scanning());

        let mut events = registry.subscribe_events();
        // Fails the name prefix.
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("other", -40),
        });
        // Fails the RSSI threshold.
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: OTHER_DEVICE,
            advertisement: advertisement("rblue-test", -90),
        });
        // Passes both.
        let matching = DeviceId::Mac([0x02, 0, 0, 0xCA, 0xFE, 0x03], AddressType::Public);
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: matching.clone(),
            advertisement: advertisement("rblue-test", -50),
        });
        let AdapterEvent::DeviceDiscovered(discovered) = events.next().await.expect("event") else {
            panic!("expected the matching discovery");
        };
        assert_eq!(discovered.id, matching);
        // Filtered devices never enter the device table.
        assert!(matches!(
            registry.peripheral(&DEVICE),
            Err(Error::NotFound(_))
        ));
        assert_eq!(registry.known_peripherals().len(), 1);
        adapter.stop_scan().await.expect("stop scan");
        assert!(!registry.is_scanning());
    }

    #[tokio::test]
    async fn stop_scan_without_start_is_invalid_state() {
        let registry = DriverRegistry::new();
        let adapter = OhosAdapter::new(&registry);
        let result = Adapter::stop_scan(&adapter).await;
        assert!(matches!(result, Err(Error::InvalidState(_))));
    }

    #[tokio::test]
    async fn disconnect_event_fails_in_flight_requests_for_that_device() {
        let registry = Arc::new(DriverRegistry::new());
        let sent = Arc::new(AtomicUsize::new(0));
        {
            let sent = Arc::clone(&sent);
            registry.register_transport(ScriptedTransport::new(&registry, move |_, _, _| {
                sent.fetch_add(1, Ordering::Relaxed);
            }));
        }
        let task = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .request(BleDriverCommand::Connect { device: DEVICE })
                    .await
            })
        };
        while sent.load(Ordering::Relaxed) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        registry.emit_event(DriverEvent::DeviceDisconnected {
            device: DEVICE,
            reason: Some(rblue_api::DisconnectReason::Remote),
        });
        let result = task.await.expect("join");
        assert!(matches!(result, Err(Error::Disconnected)));
    }

    #[tokio::test]
    async fn mtu_response_updates_session_state() {
        let registry = Arc::new(DriverRegistry::new());
        registry.register_transport(ScriptedTransport::new(&registry, |request, _, core| {
            let mtu = match request.command {
                BleDriverCommand::RequestMtu { mtu, .. } => mtu + 11,
                _ => 0,
            };
            core.complete_request(DriverResponse::Success {
                req_id: request.req_id,
                value: DriverValue::Mtu { mtu },
            });
        }));
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        let negotiated = registry
            .request(BleDriverCommand::RequestMtu {
                device: DEVICE,
                mtu: 247,
            })
            .await
            .expect("request mtu");
        assert_eq!(negotiated, DriverValue::Mtu { mtu: 258 });
        assert_eq!(peripheral.mtu().await.expect("mtu"), 258);
        assert_eq!(
            peripheral
                .max_write_len(&CHARACTERISTIC, WriteMode::WithResponse)
                .await
                .expect("max write len"),
            255
        );
    }

    #[tokio::test]
    async fn connection_events_track_state_and_clear_session() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        assert!(peripheral.is_connected().await.expect("connected"));
        peripheral.discover_services(&[]).await.expect("discover");
        registry.emit_event(DriverEvent::DeviceDisconnected {
            device: DEVICE,
            reason: None,
        });
        assert!(!peripheral.is_connected().await.expect("connected"));
        // Session-scoped state is gone: the negotiated MTU is reset, and
        // mtu reports NotConnected.
        assert!(matches!(peripheral.mtu().await, Err(Error::NotConnected)));
    }

    #[tokio::test]
    async fn subscribe_rejects_characteristics_without_notify_or_indicate() {
        let registry = Arc::new(DriverRegistry::new());
        discover_with_tree(&registry);
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        let read_only = CharacteristicId {
            service: ServiceId {
                uuid: SERVICE_UUID,
                instance: 0,
            },
            uuid: uuid::Uuid::from_u128(0x9999),
            instance: 0,
        };
        let result = peripheral.subscribe(&read_only).await;
        assert!(matches!(result, Err(Error::InvalidState(_))));
    }

    #[tokio::test]
    async fn subscribe_accepts_notify_characteristic() {
        let (registry, peripheral) = connected_registry();
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        discover_with_tree(&registry);
        peripheral.discover_services(&[]).await.expect("discover");
        peripheral
            .subscribe(&CHARACTERISTIC)
            .await
            .expect("subscribe");
        peripheral
            .unsubscribe(&CHARACTERISTIC)
            .await
            .expect("unsubscribe");
    }

    #[tokio::test]
    async fn write_rejects_oversize_payload_before_dispatch() {
        let registry = Arc::new(DriverRegistry::new());
        let writes = Arc::new(AtomicUsize::new(0));
        registry.register_transport(ScriptedTransport::new(&registry, {
            let writes = Arc::clone(&writes);
            move |request, _, core| match &request.command {
                BleDriverCommand::DiscoverServices { .. } => {
                    complete_discovery(core, request.req_id);
                }
                BleDriverCommand::Write { .. } => {
                    writes.fetch_add(1, Ordering::Relaxed);
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
                _ => {
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
            }
        }));
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        // Default ATT MTU 23 -> max write 20; the length check runs before
        // any driver dispatch.
        let oversize = vec![0u8; 21];
        let result = peripheral
            .write(&WRITABLE, &oversize, WriteMode::WithResponse)
            .await;
        match result {
            Err(Error::InvalidLength { len, max }) => {
                assert_eq!(len, 21);
                assert_eq!(max, 20);
            }
            other => panic!("expected InvalidLength, got {other:?}"),
        }
        assert_eq!(
            writes.load(Ordering::Relaxed),
            0,
            "oversize writes must not reach the driver"
        );
    }

    #[tokio::test]
    async fn write_mode_mismatch_beats_length_check() {
        let registry = Arc::new(DriverRegistry::new());
        let writes = Arc::new(AtomicUsize::new(0));
        registry.register_transport(ScriptedTransport::new(&registry, {
            let writes = Arc::clone(&writes);
            move |request, _, core| match &request.command {
                BleDriverCommand::DiscoverServices { .. } => {
                    complete_discovery(core, request.req_id);
                }
                BleDriverCommand::Write { .. } => {
                    writes.fetch_add(1, Ordering::Relaxed);
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
                _ => {
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
            }
        }));
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        // `WRITABLE` (READ|WRITE) gets a WithoutResponse write: the property
        // mismatch is ATT 0x03 even though the payload is also oversize -
        // property check runs first, and nothing reaches the driver.
        let result = peripheral
            .write(&WRITABLE, &[0u8; 21], WriteMode::WithoutResponse)
            .await;
        match result {
            Err(Error::Att(att)) => assert_eq!(att.code, ATT_WRITE_NOT_PERMITTED),
            other => panic!("expected Att(0x03), got {other:?}"),
        }
        assert_eq!(writes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn write_exact_max_len_reaches_driver_with_payload() {
        let registry = Arc::new(DriverRegistry::new());
        let payloads = Arc::new(Mutex::new(Vec::new()));
        registry.register_transport(ScriptedTransport::new(&registry, {
            let payloads = Arc::clone(&payloads);
            move |request, payload, core| match &request.command {
                BleDriverCommand::DiscoverServices { .. } => {
                    complete_discovery(core, request.req_id);
                }
                BleDriverCommand::Write { .. } => {
                    payloads
                        .lock()
                        .expect("payloads poisoned")
                        .push(payload.to_vec());
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
                _ => {
                    core.complete_request(DriverResponse::Success {
                        req_id: request.req_id,
                        value: DriverValue::None,
                    });
                }
            }
        }));
        registry.emit_event(DriverEvent::DeviceConnected { device: DEVICE });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        peripheral.discover_services(&[]).await.expect("discover");
        // Exactly max_write_len (23 - 3 = 20) is accepted.
        let exact = vec![0xA5u8; 20];
        peripheral
            .write(&WRITABLE, &exact, WriteMode::WithResponse)
            .await
            .expect("write at max length");
        assert_eq!(
            payloads.lock().expect("payloads poisoned").as_slice(),
            [exact.as_slice()],
            "payload rides the binary data plane to the driver"
        );
    }

    #[tokio::test]
    async fn gatt_ops_fail_fast_when_disconnected_without_dispatch() {
        let registry = Arc::new(DriverRegistry::new());
        let sent = Arc::new(AtomicUsize::new(0));
        registry.register_transport(ScriptedTransport::new(&registry, {
            let sent = Arc::clone(&sent);
            move |_, _, _| {
                sent.fetch_add(1, Ordering::Relaxed);
            }
        }));
        registry.emit_event(DriverEvent::DeviceDiscovered {
            device: DEVICE,
            advertisement: advertisement("echo", -50),
        });
        let peripheral = registry.peripheral(&DEVICE).expect("peripheral");
        // A known-but-never-connected device fails every GATT operation with
        // NotConnected before anything reaches the driver.
        assert!(matches!(
            peripheral.read(&CHARACTERISTIC).await,
            Err(Error::NotConnected)
        ));
        assert!(matches!(
            peripheral
                .write(&CHARACTERISTIC, b"x", WriteMode::WithResponse)
                .await,
            Err(Error::NotConnected)
        ));
        assert!(matches!(
            peripheral.subscribe(&CHARACTERISTIC).await,
            Err(Error::NotConnected)
        ));
        assert!(matches!(
            peripheral.unsubscribe(&CHARACTERISTIC).await,
            Err(Error::NotConnected)
        ));
        assert_eq!(
            sent.load(Ordering::Relaxed),
            0,
            "no command may leave the core"
        );
    }

    #[tokio::test]
    async fn write_command_payload_travels_beside_the_json_command() {
        let registry = Arc::new(DriverRegistry::with_config(RegistryConfig {
            request_timeout: Duration::from_millis(5),
            ..RegistryConfig::default()
        }));
        let recording = Arc::new(RecordingTransport::new());
        registry.register_transport(Arc::clone(&recording) as Arc<dyn DriverTransport>);
        let payload = b"frame-payload";
        let result = registry
            .request_with_payload(
                BleDriverCommand::Write {
                    device: DEVICE,
                    characteristic: CHARACTERISTIC,
                    with_response: false,
                },
                payload,
            )
            .await;
        assert!(
            matches!(result, Err(Error::Timeout)),
            "no completion is scripted"
        );
        let sent = recording.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].payload, payload, "payload rides the data plane");
        assert_eq!(sent[0].request.req_id, 1);
    }

    #[tokio::test]
    async fn re_registration_replaces_transport() {
        let registry = Arc::new(DriverRegistry::new());
        let first = Arc::new(RecordingTransport::new());
        registry.register_transport(Arc::clone(&first) as Arc<dyn DriverTransport>);
        none_responder(&registry);
        assert!(registry.request(BleDriverCommand::Init).await.is_ok());
        assert!(first.is_empty(), "requests must go to the latest transport");
    }
}
