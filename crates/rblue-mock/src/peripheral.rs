//! Mock [`Peripheral`]: a scripted device with a controllable GATT tree, a
//! forced-disconnect switch, failure injection, and manual notification
//! injection.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use futures::channel::oneshot;
use futures::stream::StreamExt;
use uuid::Uuid;

use rblue_api::device::DeviceId;
use rblue_api::error::{AttError, Error};
use rblue_api::event::{AdapterEvent, DisconnectReason};
use rblue_api::gatt::{
    descriptors_from_uuids, Characteristic, CharacteristicId, CharacteristicProperties,
    DescriptorId, Notification, Service, ServiceId, ServiceSet, WriteMode,
};
use rblue_api::peripheral::{ConnectOptions, ConnectionPriority, NotificationStream, Peripheral};
use rblue_api::scan::{AdvertisementData, RequestDeviceOptions, ScanFilter};

use crate::adapter::{AdapterShared, DEFAULT_MAX_WRITE_LEN, DEFAULT_RSSI, MOCK_ATT_MTU};
use crate::hub::BroadcastHub;

/// ATT status for addressing a characteristic absent from the tree.
const ATT_INVALID_HANDLE: u8 = 0x01;
/// ATT status for reads on a characteristic without the `READ` property.
const ATT_READ_NOT_PERMITTED: u8 = 0x02;
/// ATT status for writes in a mode the property bits do not allow.
const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;

/// Configuration of one simulated characteristic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockCharacteristic {
    /// Characteristic UUID.
    pub uuid: Uuid,
    /// Operations the characteristic permits; drives write-mode and subscribe
    /// checks exactly like a real stack.
    pub properties: CharacteristicProperties,
    /// Value served to `read` calls until the first write or
    /// [`MockPeripheral::set_value`].
    pub initial_value: Vec<u8>,
    /// Whether every accepted write is echoed back as a notification (the
    /// behavior the contract fixture requires).
    pub echo: bool,
    /// Maximum accepted write payload in bytes; larger writes are rejected
    /// with `Error::InvalidLength`, never truncated. Descriptor writes use
    /// the same bound.
    pub max_write_len: usize,
    /// Descriptors of this characteristic, in discovery order.
    pub descriptors: Vec<MockDescriptor>,
}

/// Configuration of one simulated descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockDescriptor {
    /// Descriptor UUID.
    pub uuid: Uuid,
    /// Value served to `read_descriptor` until the first write.
    pub initial_value: Vec<u8>,
}

impl MockDescriptor {
    /// Creates a descriptor with an empty initial value.
    #[must_use]
    pub fn new(uuid: Uuid) -> Self {
        Self {
            uuid,
            initial_value: Vec::new(),
        }
    }

    /// Sets the value served to reads.
    #[must_use]
    pub fn with_initial_value(mut self, initial_value: Vec<u8>) -> Self {
        self.initial_value = initial_value;
        self
    }
}

impl MockCharacteristic {
    /// Creates the configuration with an empty initial value, no echo, and
    /// the default 514-byte write bound (ATT MTU 517 minus 3).
    #[must_use]
    pub fn new(uuid: Uuid, properties: CharacteristicProperties) -> Self {
        Self {
            uuid,
            properties,
            initial_value: Vec::new(),
            echo: false,
            max_write_len: DEFAULT_MAX_WRITE_LEN,
            descriptors: Vec::new(),
        }
    }

    /// Sets the value served to reads.
    #[must_use]
    pub fn with_initial_value(mut self, initial_value: Vec<u8>) -> Self {
        self.initial_value = initial_value;
        self
    }

    /// Echoes every accepted write back as a notification to subscribers.
    #[must_use]
    pub fn with_echo(mut self) -> Self {
        self.echo = true;
        self
    }

    /// Overrides the maximum accepted write payload.
    #[must_use]
    pub fn with_max_write_len(mut self, max_write_len: usize) -> Self {
        self.max_write_len = max_write_len;
        self
    }

    /// Appends a descriptor configuration.
    #[must_use]
    pub fn with_descriptor(mut self, descriptor: MockDescriptor) -> Self {
        self.descriptors.push(descriptor);
        self
    }
}

/// Configuration of one simulated service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MockService {
    /// Service UUID.
    pub uuid: Uuid,
    /// `true` for a primary service, `false` for a secondary service.
    pub primary: bool,
    /// Characteristics contained in the service, in discovery order.
    pub characteristics: Vec<MockCharacteristic>,
}

impl MockService {
    /// Creates a primary service without characteristics; chain
    /// [`MockService::characteristic`] to fill it.
    #[must_use]
    pub fn new(uuid: Uuid) -> Self {
        Self {
            uuid,
            primary: true,
            characteristics: Vec::new(),
        }
    }

    /// Marks the service secondary.
    #[must_use]
    pub fn secondary(mut self) -> Self {
        self.primary = false;
        self
    }

    /// Appends a characteristic configuration.
    #[must_use]
    pub fn characteristic(mut self, characteristic: MockCharacteristic) -> Self {
        self.characteristics.push(characteristic);
        self
    }
}

/// GATT client operation kind used to key injected failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MockOp {
    /// `Peripheral::connect`.
    Connect,
    /// `Peripheral::read`.
    Read,
    /// `Peripheral::write`, both modes.
    Write,
    /// `Peripheral::subscribe`.
    Subscribe,
}

/// A failure injected into mock operations, delivered as the unified
/// [`Error`] when the operation runs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MockFailure {
    /// ATT-level failure: becomes `Error::Att` with the given error.
    Att(AttError),
    /// Platform status preserved verbatim: becomes
    /// `Error::Platform { code, message }` (see `docs/error-mapping.md`).
    Platform {
        /// Raw platform status code (e.g. Android GATT status 133).
        code: i32,
        /// Human-readable context for the code.
        message: String,
    },
}

impl MockFailure {
    /// `MockFailure::Att` with the given code and no attribute handle.
    #[must_use]
    pub fn att(code: u8) -> Self {
        Self::Att(AttError { code, handle: None })
    }

    /// `MockFailure::Platform` with a generic message.
    #[must_use]
    pub fn platform(code: i32) -> Self {
        Self::Platform {
            code,
            message: "injected mock failure".to_owned(),
        }
    }

    /// Converts the injection into the unified error model.
    #[must_use]
    pub fn into_error(self) -> Error {
        match self {
            Self::Att(att) => Error::Att(att),
            Self::Platform { code, message } => Error::Platform { code, message },
        }
    }
}

/// Mutable state of one simulated peripheral.
#[derive(Debug)]
struct PeriphInner {
    connected: bool,
    connection_priority: Option<ConnectionPriority>,
    /// Distinct from the local rblue session: the OS would already consider
    /// this device connected, so `system_connected_peripherals` can return
    /// it before `connect()`.
    system_connected: bool,
    name: Option<String>,
    services: Vec<MockService>,
    values: HashMap<CharacteristicId, Vec<u8>>,
    descriptor_values: HashMap<DescriptorId, Vec<u8>>,
    subscriptions: HashSet<CharacteristicId>,
    failures: HashMap<(MockOp, Option<Uuid>), MockFailure>,
    advertisement: AdvertisementData,
    script: Vec<AdvertisementData>,
    script_position: usize,
    visible: bool,
    rssi: i16,
    mtu: u16,
    writes_held: bool,
    write_waiters: Vec<oneshot::Sender<()>>,
}

/// A simulated peripheral: implements the full [`Peripheral`] trait surface
/// and doubles as the test control plane for scripting its radio behavior.
///
/// Handles are created by [`MockAdapter::add_device`](crate::MockAdapter::add_device).
/// The device only becomes *known* to the adapter - visible in
/// `Adapter::peripherals`, `Adapter::peripheral`, and the event-stream
/// snapshot - once it is sighted (scan snapshot, [`MockPeripheral::advertise`],
/// or a picked `request_device`) or connected.
#[derive(Debug)]
pub struct MockPeripheral {
    id: DeviceId,
    shared: Arc<AdapterShared>,
    state: Mutex<PeriphInner>,
    notifications: BroadcastHub<Notification>,
}

impl MockPeripheral {
    pub(crate) fn new(id: DeviceId, name: &str, shared: Arc<AdapterShared>) -> Self {
        Self {
            id,
            shared,
            state: Mutex::new(PeriphInner {
                connected: false,
                connection_priority: None,
                system_connected: false,
                name: Some(name.to_owned()),
                services: Vec::new(),
                values: HashMap::new(),
                descriptor_values: HashMap::new(),
                subscriptions: HashSet::new(),
                failures: HashMap::new(),
                advertisement: AdvertisementData {
                    local_name: Some(name.to_owned()),
                    rssi: Some(DEFAULT_RSSI),
                    is_connectable: Some(true),
                    ..AdvertisementData::default()
                },
                script: Vec::new(),
                script_position: 0,
                visible: true,
                rssi: DEFAULT_RSSI,
                mtu: MOCK_ATT_MTU,
                writes_held: false,
                write_waiters: Vec::new(),
            }),
            notifications: BroadcastHub::default(),
        }
    }

    pub(crate) fn device_id(&self) -> &DeviceId {
        &self.id
    }

    pub(crate) fn is_visible(&self) -> bool {
        self.lock().visible
    }

    pub(crate) fn is_connected_now(&self) -> bool {
        self.lock().connected
    }

    /// Last priority accepted by [`Peripheral::set_connection_priority`], if any.
    #[must_use]
    pub fn connection_priority(&self) -> Option<ConnectionPriority> {
        self.lock().connection_priority
    }

    pub(crate) fn is_system_connected_now(&self) -> bool {
        self.lock().system_connected
    }

    /// Service UUIDs the mock treats as known to the system: the scripted
    /// GATT tree plus the current advertisement.
    pub(crate) fn system_service_uuids(&self) -> Vec<Uuid> {
        let inner = self.lock();
        let mut services: Vec<Uuid> = inner.services.iter().map(|service| service.uuid).collect();
        for uuid in &inner.advertisement.services {
            if !services.contains(uuid) {
                services.push(*uuid);
            }
        }
        services
    }

    /// Marks this device as already connected at the system level, without
    /// opening a local rblue session. Used by
    /// [`Adapter::system_connected_peripherals`](rblue_api::Adapter::system_connected_peripherals).
    pub fn set_system_connected(&self, connected: bool) {
        self.lock().system_connected = connected;
    }

    pub(crate) fn current_advertisement(&self) -> AdvertisementData {
        self.lock().advertisement.clone()
    }

    fn lock(&self) -> MutexGuard<'_, PeriphInner> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    // ---- GATT tree scripting ----

    /// Adds a service (with its characteristics) to the simulated GATT tree.
    /// The next successful `discover_services` reports it; characteristic
    /// values are seeded from each configured initial value.
    pub fn add_service(&self, service: MockService) {
        let mut inner = self.lock();
        seed_values(&mut inner, &service);
        inner.services.push(service);
    }

    /// Resolves the first characteristic with `uuid` in the tree to its
    /// session-scoped [`CharacteristicId`]. Control-plane helper: usable
    /// without a connection, unlike `discover_services`.
    #[must_use]
    pub fn find_characteristic(&self, uuid: Uuid) -> Option<CharacteristicId> {
        let inner = self.lock();
        first_characteristic_id(&inner.services, uuid)
    }

    /// Replaces the value a `read` returns for the first characteristic with
    /// `uuid`. Returns `false` when the tree has no such characteristic.
    pub fn set_value(&self, uuid: Uuid, value: Vec<u8>) -> bool {
        let mut inner = self.lock();
        match first_characteristic_id(&inner.services, uuid) {
            Some(id) => {
                inner.values.insert(id, value);
                true
            }
            None => false,
        }
    }

    // ---- advertisement scripting ----

    /// Sets the current advertisement without emitting an event; the value is
    /// reported by the next scan snapshot or [`MockPeripheral::advertise`].
    pub fn set_advertisement(&self, advertisement: AdvertisementData) {
        let mut inner = self.lock();
        apply_advertisement(&mut inner, &advertisement);
    }

    /// Emits a sighting of the current advertisement:
    /// `AdapterEvent::DeviceDiscovered` on the adapter event stream on the
    /// first sighting, `AdapterEvent::DeviceUpdated` afterwards. Re-appears a
    /// device previously hidden with [`MockPeripheral::disappear`].
    ///
    /// As a test convenience the mock emits sightings regardless of scan
    /// state; the active [`ScanFilter`], if any, is applied either way.
    pub fn advertise(&self) {
        let advertisement = self.lock().advertisement.clone();
        self.publish_sighting(advertisement);
    }

    /// Updates the RSSI of the current advertisement and emits a sighting.
    /// Convenience for scripting RSSI curves.
    pub fn set_rssi(&self, rssi: i16) {
        let advertisement = {
            let mut inner = self.lock();
            inner.rssi = rssi;
            inner.advertisement.rssi = Some(rssi);
            inner.advertisement.clone()
        };
        self.publish_sighting(advertisement);
    }

    /// Loads a scripted advertisement sequence;
    /// [`MockPeripheral::advance_script`] emits one entry per call.
    pub fn set_advertisement_script(&self, script: Vec<AdvertisementData>) {
        let mut inner = self.lock();
        inner.script = script;
        inner.script_position = 0;
    }

    /// Emits the next script entry as a sighting. Returns `false` when the
    /// script is exhausted; the last entry stays current.
    #[must_use]
    pub fn advance_script(&self) -> bool {
        let step = {
            let mut inner = self.lock();
            if inner.script_position >= inner.script.len() {
                return false;
            }
            let step = inner.script[inner.script_position].clone();
            inner.script_position += 1;
            step
        };
        self.publish_sighting(step);
        true
    }

    /// Stops advertising. The device stays known to the adapter (scan history
    /// semantics) but is skipped by future scan snapshots and cannot be picked
    /// by `request_device` until it advertises again.
    pub fn disappear(&self) {
        self.lock().visible = false;
    }

    pub(crate) fn sight_current_advertisement(&self) {
        let advertisement = self.lock().advertisement.clone();
        self.publish_sighting(advertisement);
    }

    /// Emits a sighting of `advertisement`: updates the peripheral state,
    /// then hands the event to the adapter, which applies the active scan
    /// filter and publishes the discovery/update event. The peripheral state
    /// lock is released before the adapter state lock is taken, so the two
    /// locks are never held simultaneously here.
    fn publish_sighting(&self, advertisement: AdvertisementData) {
        let rssi_fallback = {
            let mut inner = self.lock();
            inner.visible = true;
            apply_advertisement(&mut inner, &advertisement);
            inner.rssi
        };
        self.shared
            .record_sighting(&self.id, advertisement, rssi_fallback);
    }

    // ---- connection and failure control ----

    /// Forcefully ends the connection: emits
    /// `AdapterEvent::DeviceDisconnected` with `reason`, clears subscriptions
    /// (CCCD semantics), and makes subsequent GATT operations fail with
    /// `Error::NotConnected`. No-op when already disconnected.
    pub fn force_disconnect(&self, reason: DisconnectReason) {
        self.drop_connection(reason);
    }

    fn drop_connection(&self, reason: DisconnectReason) {
        let mut inner = self.lock();
        if !inner.connected {
            return;
        }
        inner.connected = false;
        inner.subscriptions.clear();
        inner.mtu = MOCK_ATT_MTU;
        self.shared.publish(AdapterEvent::DeviceDisconnected(
            self.id.clone(),
            Some(reason),
        ));
    }

    /// Injects a failure for a client operation. `characteristic = None`
    /// applies to every characteristic of this peripheral (and is the only
    /// meaningful key for [`MockOp::Connect`]); a specific UUID overrides
    /// the blanket entry. `failure = None` clears the injection.
    ///
    /// Precedence per operation: disconnected check, injected failure,
    /// property/permission synthesis, payload-length check.
    pub fn set_failure(
        &self,
        op: MockOp,
        characteristic: Option<Uuid>,
        failure: Option<MockFailure>,
    ) {
        let mut inner = self.lock();
        match failure {
            Some(failure) => {
                inner.failures.insert((op, characteristic), failure);
            }
            None => {
                inner.failures.remove(&(op, characteristic));
            }
        }
    }

    /// Clears every injected failure of this peripheral.
    pub fn clear_failures(&self) {
        self.lock().failures.clear();
    }

    /// Parks subsequent [`Peripheral::write`] calls until
    /// [`Self::release_writes`] so a test can disconnect mid-write.
    pub fn hold_writes(&self) {
        self.lock().writes_held = true;
    }

    /// Releases every write parked by [`Self::hold_writes`]. Writes that
    /// start after this call run immediately unless held again.
    pub fn release_writes(&self) {
        let mut inner = self.lock();
        inner.writes_held = false;
        for waiter in inner.write_waiters.drain(..) {
            let _ = waiter.send(());
        }
    }

    /// Number of `write` calls currently waiting on [`Self::hold_writes`].
    #[must_use]
    pub fn waiting_writes(&self) -> usize {
        self.lock().write_waiters.len()
    }

    async fn wait_if_writes_held(&self) {
        let rx = {
            let mut inner = self.lock();
            if !inner.writes_held {
                return;
            }
            let (tx, rx) = oneshot::channel();
            inner.write_waiters.push(tx);
            rx
        };
        let _ = rx.await;
    }

    fn injected_failure(
        inner: &PeriphInner,
        op: MockOp,
        characteristic: Uuid,
    ) -> Option<MockFailure> {
        inner
            .failures
            .get(&(op, Some(characteristic)))
            .or_else(|| inner.failures.get(&(op, None)))
            .cloned()
    }

    // ---- value and notification control ----

    /// Injects a notification as if the peripheral had sent it: updates the
    /// stored characteristic value and delivers a [`Notification`] to
    /// subscribers. Values for unsubscribed characteristics update the value
    /// but are not delivered - the trait guarantees delivery only after
    /// `subscribe`.
    ///
    /// Returns the resolved session id, or `None` when the tree has no
    /// characteristic with `uuid`.
    pub fn push_notify(&self, uuid: Uuid, data: &[u8]) -> Option<CharacteristicId> {
        let mut inner = self.lock();
        let id = first_characteristic_id(&inner.services, uuid)?;
        inner.values.insert(id, data.to_vec());
        let subscribed = inner.subscriptions.contains(&id);
        drop(inner);
        if subscribed {
            self.notifications.publish(Notification {
                id,
                data: data.to_vec(),
            });
        }
        Some(id)
    }

    pub(crate) fn matches_request_options(&self, opts: &RequestDeviceOptions) -> bool {
        let inner = self.lock();
        matches_request_options(&inner.advertisement, opts)
    }
}

/// Applies `advertisement` to the peripheral state: current payload, name,
/// and last known RSSI.
fn apply_advertisement(inner: &mut PeriphInner, advertisement: &AdvertisementData) {
    if let Some(rssi) = advertisement.rssi {
        inner.rssi = rssi;
    }
    if let Some(name) = &advertisement.local_name {
        inner.name = Some(name.clone());
    }
    inner.advertisement = advertisement.clone();
}

/// Software scan filter, applied to every sighting exactly as documented on
/// [`ScanFilter`]: name prefix, service membership, manufacturer prefix, and
/// minimum RSSI.
pub(crate) fn matches_scan_filter(advertisement: &AdvertisementData, filter: &ScanFilter) -> bool {
    if let Some(prefix) = &filter.name_prefix {
        if !advertisement
            .local_name
            .as_deref()
            .is_some_and(|name| name.starts_with(prefix))
        {
            return false;
        }
    }
    if !filter.services.is_empty()
        && !filter
            .services
            .iter()
            .any(|uuid| advertisement.services.contains(uuid))
    {
        return false;
    }
    if let Some(expected) = &filter.manufacturer_data {
        match &advertisement.manufacturer_data {
            Some((company_id, data))
                if *company_id == expected.company_id
                    && data.starts_with(&expected.data_prefix) => {}
            _ => return false,
        }
    }
    if let Some(threshold) = filter.rssi_threshold {
        if advertisement.rssi.is_some_and(|rssi| rssi < threshold) {
            return false;
        }
    }
    true
}

/// Lenient `request_device` matching: `accept_all_devices` short-circuits,
/// otherwise name prefix and service membership are honored (an empty service
/// list matches anything).
fn matches_request_options(advertisement: &AdvertisementData, opts: &RequestDeviceOptions) -> bool {
    if opts.accept_all_devices {
        return true;
    }
    if let Some(prefix) = &opts.name_prefix {
        if !advertisement
            .local_name
            .as_deref()
            .is_some_and(|name| name.starts_with(prefix))
        {
            return false;
        }
    }
    opts.services.is_empty()
        || opts
            .services
            .iter()
            .any(|uuid| advertisement.services.contains(uuid))
}

/// Resolves the first characteristic with `uuid` to its session id: the
/// containing service is the first one carrying the characteristic, its
/// `instance` is the count of same-UUID services before it.
fn first_characteristic_id(services: &[MockService], uuid: Uuid) -> Option<CharacteristicId> {
    for (position, service) in services.iter().enumerate() {
        if !service.characteristics.iter().any(|c| c.uuid == uuid) {
            continue;
        }
        let service_instance = services[..position]
            .iter()
            .filter(|s| s.uuid == service.uuid)
            .count() as u16;
        return Some(CharacteristicId {
            service: ServiceId {
                uuid: service.uuid,
                instance: service_instance,
            },
            uuid,
            instance: 0,
        });
    }
    None
}

/// Seeds the value store of `service`'s characteristics, keyed by the same
/// session ids discovery would hand out.
fn seed_values(inner: &mut PeriphInner, service: &MockService) {
    let service_instance = inner
        .services
        .iter()
        .filter(|s| s.uuid == service.uuid)
        .count() as u16;
    for (position, characteristic) in service.characteristics.iter().enumerate() {
        let char_instance = service.characteristics[..position]
            .iter()
            .filter(|c| c.uuid == characteristic.uuid)
            .count() as u16;
        let id = CharacteristicId {
            service: ServiceId {
                uuid: service.uuid,
                instance: service_instance,
            },
            uuid: characteristic.uuid,
            instance: char_instance,
        };
        inner
            .values
            .entry(id)
            .or_insert_with(|| characteristic.initial_value.clone());
        for (desc_position, descriptor) in characteristic.descriptors.iter().enumerate() {
            let desc_instance = characteristic.descriptors[..desc_position]
                .iter()
                .filter(|listed| listed.uuid == descriptor.uuid)
                .count() as u16;
            let descriptor_id = DescriptorId {
                characteristic: id,
                uuid: descriptor.uuid,
                instance: desc_instance,
            };
            inner
                .descriptor_values
                .entry(descriptor_id)
                .or_insert_with(|| descriptor.initial_value.clone());
        }
    }
}

/// Write bound of a resolved descriptor, inherited from its characteristic.
struct ResolvedDescriptor {
    max_write_len: usize,
}

fn resolve_descriptor(services: &[MockService], id: &DescriptorId) -> Option<ResolvedDescriptor> {
    let resolved = resolve_characteristic(services, &id.characteristic)?;
    let service = services
        .iter()
        .filter(|service| service.uuid == id.characteristic.service.uuid)
        .nth(id.characteristic.service.instance as usize)?;
    let characteristic = service
        .characteristics
        .iter()
        .filter(|characteristic| characteristic.uuid == id.characteristic.uuid)
        .nth(id.characteristic.instance as usize)?;
    let found = characteristic
        .descriptors
        .iter()
        .filter(|descriptor| descriptor.uuid == id.uuid)
        .nth(id.instance as usize);
    found.map(|_| ResolvedDescriptor {
        max_write_len: resolved.max_write_len,
    })
}

/// Property bits, echo flag, and write bound of a resolved characteristic.
struct ResolvedCharacteristic {
    properties: CharacteristicProperties,
    echo: bool,
    max_write_len: usize,
}

/// Resolves a session id against the tree, using the same instance counting
/// as discovery: `instance` fields are zero-based positions among same-UUID
/// entries in discovery order.
fn resolve_characteristic(
    services: &[MockService],
    ch: &CharacteristicId,
) -> Option<ResolvedCharacteristic> {
    let mut service_seen = 0u16;
    for service in services {
        if service.uuid != ch.service.uuid {
            continue;
        }
        if service_seen == ch.service.instance {
            let mut char_seen = 0u16;
            for characteristic in &service.characteristics {
                if characteristic.uuid != ch.uuid {
                    continue;
                }
                if char_seen == ch.instance {
                    return Some(ResolvedCharacteristic {
                        properties: characteristic.properties,
                        echo: characteristic.echo,
                        max_write_len: characteristic.max_write_len,
                    });
                }
                char_seen += 1;
            }
            return None;
        }
        service_seen += 1;
    }
    None
}

fn att_error(code: u8) -> Error {
    Error::Att(AttError { code, handle: None })
}

#[async_trait::async_trait]
impl Peripheral for MockPeripheral {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn name(&self) -> Option<String> {
        self.lock().name.clone()
    }

    async fn is_connected(&self) -> Result<bool, Error> {
        Ok(self.lock().connected)
    }

    async fn connect(&self) -> Result<(), Error> {
        let mut inner = self.lock();
        if inner.connected {
            return Ok(());
        }
        if let Some(failure) = Self::injected_failure(&inner, MockOp::Connect, Uuid::nil()) {
            return Err(failure.into_error());
        }
        inner.connected = true;
        self.shared
            .publish(AdapterEvent::DeviceConnected(self.id.clone()));
        Ok(())
    }

    async fn connect_with(&self, opts: &ConnectOptions) -> Result<(), Error> {
        if opts.auto_connect {
            return Err(Error::Unsupported {
                platform: "mock",
                feature: "auto_connect",
            });
        }
        if let Some(mtu) = opts.mtu {
            rblue_api::reject_invalid_att_mtu(mtu)?;
        }
        self.connect().await?;
        if let Some(mtu) = opts.mtu {
            self.request_mtu(mtu).await?;
        }
        Ok(())
    }
    async fn disconnect(&self) -> Result<(), Error> {
        self.drop_connection(DisconnectReason::Local);
        Ok(())
    }

    async fn discover_services(&self, filter: &[Uuid]) -> Result<ServiceSet, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        let mut services = Vec::new();
        for service in inner
            .services
            .iter()
            .filter(|service| filter.is_empty() || filter.contains(&service.uuid))
        {
            let service_id = ServiceId {
                uuid: service.uuid,
                instance: services
                    .iter()
                    .filter(|listed: &&Service| listed.uuid == service.uuid)
                    .count() as u16,
            };
            let mut characteristics = Vec::new();
            for characteristic in &service.characteristics {
                let instance = characteristics
                    .iter()
                    .filter(|listed: &&Characteristic| listed.uuid == characteristic.uuid)
                    .count() as u16;
                let id = CharacteristicId {
                    service: service_id,
                    uuid: characteristic.uuid,
                    instance,
                };
                characteristics.push(Characteristic {
                    uuid: characteristic.uuid,
                    properties: characteristic.properties,
                    descriptors: descriptors_from_uuids(
                        id,
                        characteristic
                            .descriptors
                            .iter()
                            .map(|descriptor| descriptor.uuid),
                    ),
                });
            }
            services.push(Service {
                uuid: service.uuid,
                primary: service.primary,
                characteristics,
            });
        }
        Ok(ServiceSet { services })
    }

    async fn read(&self, ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        if let Some(failure) = Self::injected_failure(&inner, MockOp::Read, ch.uuid) {
            return Err(failure.into_error());
        }
        let resolved = resolve_characteristic(&inner.services, ch)
            .ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        if !resolved.properties.contains(CharacteristicProperties::READ) {
            return Err(att_error(ATT_READ_NOT_PERMITTED));
        }
        Ok(inner.values.get(ch).cloned().unwrap_or_default())
    }

    async fn write(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        self.wait_if_writes_held().await;
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        if let Some(failure) = Self::injected_failure(&inner, MockOp::Write, ch.uuid) {
            return Err(failure.into_error());
        }
        let resolved = resolve_characteristic(&inner.services, ch)
            .ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        let required = match mode {
            WriteMode::WithResponse => CharacteristicProperties::WRITE,
            WriteMode::WithoutResponse => CharacteristicProperties::WRITE_WITHOUT_RESPONSE,
        };
        if !resolved.properties.contains(required) {
            return Err(att_error(ATT_WRITE_NOT_PERMITTED));
        }
        if data.len() > resolved.max_write_len {
            return Err(Error::InvalidLength {
                len: data.len(),
                max: resolved.max_write_len,
            });
        }
        let payload = data.to_vec();
        inner.values.insert(*ch, payload.clone());
        if resolved.echo && inner.subscriptions.contains(ch) {
            drop(inner);
            self.notifications.publish(Notification {
                id: *ch,
                data: payload,
            });
        }
        Ok(())
    }

    async fn max_write_len(&self, ch: &CharacteristicId, _mode: WriteMode) -> Result<usize, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        let resolved = resolve_characteristic(&inner.services, ch)
            .ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        Ok(resolved.max_write_len)
    }

    async fn read_descriptor(&self, id: &DescriptorId) -> Result<Vec<u8>, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        resolve_descriptor(&inner.services, id).ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        Ok(inner.descriptor_values.get(id).cloned().unwrap_or_default())
    }

    async fn write_descriptor(&self, id: &DescriptorId, data: &[u8]) -> Result<(), Error> {
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        let resolved =
            resolve_descriptor(&inner.services, id).ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        if data.len() > resolved.max_write_len {
            return Err(Error::InvalidLength {
                len: data.len(),
                max: resolved.max_write_len,
            });
        }
        inner.descriptor_values.insert(*id, data.to_vec());
        Ok(())
    }

    async fn subscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        if let Some(failure) = Self::injected_failure(&inner, MockOp::Subscribe, ch.uuid) {
            return Err(failure.into_error());
        }
        let resolved = resolve_characteristic(&inner.services, ch)
            .ok_or_else(|| att_error(ATT_INVALID_HANDLE))?;
        if !resolved
            .properties
            .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE)
        {
            return Err(Error::InvalidState(
                "characteristic supports neither notify nor indicate".to_owned(),
            ));
        }
        inner.subscriptions.insert(*ch);
        Ok(())
    }

    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        inner.subscriptions.remove(ch);
        Ok(())
    }

    fn notifications(&self) -> NotificationStream {
        self.notifications.subscribe().boxed()
    }

    async fn pair(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn unpair(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn rssi(&self) -> Result<i16, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        Ok(inner.rssi)
    }

    async fn mtu(&self) -> Result<u16, Error> {
        let inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        Ok(inner.mtu)
    }

    async fn request_mtu(&self, mtu: u16) -> Result<u16, Error> {
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        rblue_api::reject_invalid_att_mtu(mtu)?;
        inner.mtu = mtu;
        Ok(mtu)
    }

    async fn set_connection_priority(&self, priority: ConnectionPriority) -> Result<(), Error> {
        let mut inner = self.lock();
        if !inner.connected {
            return Err(Error::NotConnected);
        }
        inner.connection_priority = Some(priority);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use rblue_api::device::AddressType;
    use rblue_api::peripheral::WriteOptions;
    use rblue_api::Adapter;

    use crate::MockAdapter;

    const SERVICE_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a1);
    const ECHO_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a2);
    const PLAIN_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a3);
    const DESC_UUID: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a4);

    /// Echo characteristic (read/write/notify) plus a read-only characteristic.
    fn setup() -> (
        MockAdapter,
        Arc<MockPeripheral>,
        CharacteristicId,
        CharacteristicId,
    ) {
        let adapter = MockAdapter::new();
        let device = adapter.add_device(
            DeviceId::Mac([0x02, 0, 0, 0, 0, 1], AddressType::RandomStatic),
            "test-device",
        );
        device.add_service(
            MockService::new(SERVICE_UUID)
                .characteristic(
                    MockCharacteristic::new(
                        ECHO_UUID,
                        CharacteristicProperties::READ
                            | CharacteristicProperties::WRITE
                            | CharacteristicProperties::WRITE_WITHOUT_RESPONSE
                            | CharacteristicProperties::NOTIFY,
                    )
                    .with_initial_value(b"init".to_vec())
                    .with_echo()
                    .with_descriptor(
                        MockDescriptor::new(DESC_UUID).with_initial_value(b"desc-init".to_vec()),
                    ),
                )
                .characteristic(MockCharacteristic::new(
                    PLAIN_UUID,
                    CharacteristicProperties::READ,
                )),
        );
        let echo = device
            .find_characteristic(ECHO_UUID)
            .expect("echo characteristic registered");
        let plain = device
            .find_characteristic(PLAIN_UUID)
            .expect("plain characteristic registered");
        (adapter, device, echo, plain)
    }

    #[tokio::test]
    async fn forced_disconnect_emits_event_and_fails_operations() {
        let (adapter, device, echo, _) = setup();
        let mut events = adapter.events();
        device.connect().await.expect("connect");
        assert!(device.is_connected().await.expect("is_connected"));

        device.force_disconnect(DisconnectReason::Remote);

        assert!(!device.is_connected().await.expect("is_connected"));
        assert!(matches!(device.read(&echo).await, Err(Error::NotConnected)));
        assert!(matches!(
            device.subscribe(&echo).await,
            Err(Error::NotConnected)
        ));
        assert!(matches!(device.rssi().await, Err(Error::NotConnected)));

        // Reconnecting restores a working session.
        device.connect().await.expect("reconnect");
        assert!(device.read(&echo).await.is_ok());

        loop {
            match events.next().await {
                Some(AdapterEvent::DeviceConnected(_)) => continue,
                Some(AdapterEvent::DeviceDisconnected(id, reason)) => {
                    assert_eq!(&id, device.id());
                    assert_eq!(reason, Some(DisconnectReason::Remote));
                    break;
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn injected_failures_reach_read_write_and_subscribe() {
        let (_adapter, device, echo, plain) = setup();
        device.connect().await.expect("connect");

        device.set_failure(MockOp::Read, None, Some(MockFailure::att(0x05)));
        assert!(matches!(
            device.read(&echo).await,
            Err(Error::Att(att)) if att.code == 0x05
        ));

        device.set_failure(MockOp::Write, None, Some(MockFailure::platform(133)));
        assert!(matches!(
            device.write(&echo, b"x", WriteMode::WithResponse).await,
            Err(Error::Platform { code: 133, .. })
        ));

        device.set_failure(MockOp::Subscribe, None, Some(MockFailure::att(0x08)));
        assert!(matches!(
            device.subscribe(&echo).await,
            Err(Error::Att(att)) if att.code == 0x08
        ));

        // A characteristic-specific entry overrides the blanket injection.
        device.set_failure(MockOp::Read, Some(PLAIN_UUID), Some(MockFailure::att(0x07)));
        assert!(matches!(
            device.read(&plain).await,
            Err(Error::Att(att)) if att.code == 0x07
        ));
        assert!(matches!(
            device.read(&echo).await,
            Err(Error::Att(att)) if att.code == 0x05
        ));

        device.clear_failures();
        assert!(device.read(&echo).await.is_ok());
        assert!(device
            .write(&echo, b"x", WriteMode::WithResponse)
            .await
            .is_ok());
        assert!(device.subscribe(&echo).await.is_ok());
    }

    #[tokio::test]
    async fn pushed_notifications_reach_only_subscribers() {
        let (_adapter, device, echo, _) = setup();
        device.connect().await.expect("connect");
        device.subscribe(&echo).await.expect("subscribe");
        let mut notifications = device.notifications();

        let id = device
            .push_notify(ECHO_UUID, b"hello")
            .expect("echo characteristic exists");
        assert_eq!(id, echo);
        let notification = notifications.next().await.expect("notification");
        assert_eq!(notification.id, echo);
        assert_eq!(notification.data, b"hello");

        device.unsubscribe(&echo).await.expect("unsubscribe");
        device
            .push_notify(ECHO_UUID, b"dropped")
            .expect("characteristic still exists");
        let quiet = tokio::time::timeout(Duration::from_millis(50), notifications.next()).await;
        assert!(
            quiet.is_err(),
            "unsubscribed characteristics must not deliver values"
        );
    }

    #[tokio::test]
    async fn echo_write_notifies_and_semantics_are_enforced() {
        let (_adapter, device, echo, plain) = setup();
        device.connect().await.expect("connect");

        // Reads serve the configured initial value...
        assert_eq!(device.read(&echo).await.expect("read"), b"init");
        // ...a read-only characteristic rejects writes at ATT level...
        assert!(matches!(
            device.write(&plain, b"x", WriteMode::WithResponse).await,
            Err(Error::Att(att)) if att.code == ATT_WRITE_NOT_PERMITTED
        ));
        // ...and oversize payloads are rejected, never truncated.
        assert!(matches!(
            device
                .write(&echo, &[0u8; 515], WriteMode::WithResponse)
                .await,
            Err(Error::InvalidLength { len: 515, max: 514 })
        ));

        device.subscribe(&echo).await.expect("subscribe");
        let mut notifications = device.notifications();
        device
            .write(&echo, b"ping", WriteMode::WithoutResponse)
            .await
            .expect("echo write");
        let notification = notifications.next().await.expect("echo notification");
        assert_eq!(notification.id, echo);
        assert_eq!(notification.data, b"ping");
        // The write also updated the readable value.
        assert_eq!(device.read(&echo).await.expect("read"), b"ping");
    }

    #[tokio::test]
    async fn descriptor_read_write_and_invalid_length() {
        let (_adapter, device, echo, _) = setup();
        let descriptor = DescriptorId {
            characteristic: echo,
            uuid: DESC_UUID,
            instance: 0,
        };
        let missing = DescriptorId {
            characteristic: echo,
            uuid: Uuid::from_u128(0x00d1),
            instance: 0,
        };

        assert!(matches!(
            device.read_descriptor(&descriptor).await,
            Err(Error::NotConnected)
        ));
        device.connect().await.expect("connect");

        let services = device.discover_services(&[]).await.expect("discover");
        let discovered = &services.services[0].characteristics[0].descriptors;
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].id(), descriptor);
        assert_eq!(
            device.read_descriptor(&descriptor).await.expect("read"),
            b"desc-init"
        );
        device
            .write_descriptor(&descriptor, b"hello")
            .await
            .expect("write");
        assert_eq!(
            device.read_descriptor(&descriptor).await.expect("read"),
            b"hello"
        );
        assert!(matches!(
            device.read_descriptor(&missing).await,
            Err(Error::Att(att)) if att.code == ATT_INVALID_HANDLE
        ));
        assert!(matches!(
            device.write_descriptor(&missing, b"x").await,
            Err(Error::Att(att)) if att.code == ATT_INVALID_HANDLE
        ));
        assert!(matches!(
            device.write_descriptor(&descriptor, &[0u8; 515]).await,
            Err(Error::InvalidLength { len: 515, max: 514 })
        ));
    }

    #[tokio::test]
    async fn write_with_allow_long_write_chunks_without_truncating() {
        let adapter = MockAdapter::new();
        let device = adapter.add_device(
            DeviceId::Mac([0x02, 0, 0, 0, 0, 1], AddressType::RandomStatic),
            "long-write",
        );
        device.add_service(
            MockService::new(SERVICE_UUID).characteristic(
                MockCharacteristic::new(
                    ECHO_UUID,
                    CharacteristicProperties::READ
                        | CharacteristicProperties::WRITE
                        | CharacteristicProperties::NOTIFY,
                )
                .with_max_write_len(4)
                .with_echo(),
            ),
        );
        let echo = device
            .find_characteristic(ECHO_UUID)
            .expect("echo characteristic registered");
        device.connect().await.expect("connect");
        device.subscribe(&echo).await.expect("subscribe");
        let mut notifications = device.notifications();

        let data = vec![1, 2, 3, 4, 5, 6, 7, 8, 9];
        assert!(matches!(
            device.write(&echo, &data, WriteMode::WithResponse).await,
            Err(Error::InvalidLength { len: 9, max: 4 })
        ));

        device
            .write_with(
                &echo,
                &data,
                WriteMode::WithResponse,
                &WriteOptions {
                    allow_long_write: true,
                },
            )
            .await
            .expect("long write");

        let mut echoed = Vec::new();
        for _ in 0..3 {
            let notification = notifications.next().await.expect("chunk notification");
            echoed.extend_from_slice(&notification.data);
        }
        assert_eq!(echoed, data);
        // The stored value is the last chunk (each write is a single ATT write).
        assert_eq!(device.read(&echo).await.expect("read"), vec![9]);
    }

    #[tokio::test]
    async fn subscribe_requires_notify_or_indicate_property() {
        let (_adapter, device, _, plain) = setup();
        device.connect().await.expect("connect");
        assert!(matches!(
            device.subscribe(&plain).await,
            Err(Error::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn pair_and_unpair_succeed_without_a_connection() {
        let (_adapter, device, _, _) = setup();
        assert!(!device.is_connected().await.expect("is_connected"));
        device.pair().await.expect("pair");
        device.pair().await.expect("pair is idempotent");
        device.unpair().await.expect("unpair");
        device.unpair().await.expect("unpair is idempotent");
        assert!(!device.is_connected().await.expect("still disconnected"));
    }

    #[tokio::test]
    async fn request_mtu_requires_a_connection() {
        let (_adapter, device, _, _) = setup();
        assert!(matches!(
            device.request_mtu(512).await,
            Err(Error::NotConnected)
        ));
    }

    #[tokio::test]
    async fn request_mtu_updates_the_negotiated_value() {
        let (_adapter, device, _, _) = setup();
        device.connect().await.expect("connect");
        assert_eq!(device.mtu().await.expect("default mtu"), MOCK_ATT_MTU);
        assert_eq!(device.request_mtu(512).await.expect("request_mtu"), 512);
        assert_eq!(device.mtu().await.expect("mtu after request"), 512);
        assert!(matches!(
            device.request_mtu(22).await,
            Err(Error::InvalidState(_))
        ));
        assert_eq!(device.mtu().await.expect("unchanged after invalid"), 512);
    }

    #[tokio::test]
    async fn connect_with_mtu_requests_after_the_link_is_up() {
        let (_adapter, device, _, _) = setup();
        device
            .connect_with(&ConnectOptions {
                auto_connect: false,
                mtu: Some(185),
            })
            .await
            .expect("connect_with mtu");
        assert_eq!(device.mtu().await.expect("mtu"), 185);
        device.disconnect().await.expect("disconnect");
        assert!(matches!(device.mtu().await, Err(Error::NotConnected)));
        device.connect().await.expect("reconnect");
        assert_eq!(device.mtu().await.expect("reset default"), MOCK_ATT_MTU);
    }

    #[tokio::test]
    async fn connect_with_still_rejects_auto_connect() {
        let (_adapter, device, _, _) = setup();
        assert!(matches!(
            device
                .connect_with(&ConnectOptions {
                    auto_connect: true,
                    mtu: Some(512),
                })
                .await,
            Err(Error::Unsupported {
                platform: "mock",
                feature: "auto_connect",
            })
        ));
        assert!(!device.is_connected().await.expect("is_connected"));
    }

    #[tokio::test]
    async fn set_connection_priority_requires_a_local_session() {
        let (_adapter, device, _, _) = setup();
        assert!(matches!(
            device
                .set_connection_priority(ConnectionPriority::High)
                .await,
            Err(Error::NotConnected)
        ));
        assert_eq!(device.connection_priority(), None);

        device.connect().await.expect("connect");
        device
            .set_connection_priority(ConnectionPriority::Low)
            .await
            .expect("low");
        assert_eq!(device.connection_priority(), Some(ConnectionPriority::Low));
        device
            .set_connection_priority(ConnectionPriority::Balanced)
            .await
            .expect("balanced");
        device
            .set_connection_priority(ConnectionPriority::High)
            .await
            .expect("high");
        assert_eq!(device.connection_priority(), Some(ConnectionPriority::High));
    }

    #[tokio::test]
    async fn injected_connect_failure_is_platform_133() {
        let (_adapter, device, _, _) = setup();
        device.set_failure(MockOp::Connect, None, Some(MockFailure::platform(133)));
        assert!(matches!(
            device.connect().await,
            Err(Error::Platform { code: 133, .. })
        ));
        assert!(!device.is_connected().await.expect("is_connected"));

        device.clear_failures();
        device.connect().await.expect("connect after clearing 133");
        assert!(device.is_connected().await.expect("is_connected"));
    }

    #[tokio::test]
    async fn held_write_sees_force_disconnect_as_not_connected() {
        let (_adapter, device, echo, _) = setup();
        device.connect().await.expect("connect");
        device.hold_writes();

        let writer = Arc::clone(&device);
        let write = tokio::spawn(async move {
            writer
                .write(&echo, b"in-flight", WriteMode::WithResponse)
                .await
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while device.waiting_writes() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("write parked on the hold");

        device.force_disconnect(DisconnectReason::Remote);
        device.release_writes();
        assert!(matches!(
            write.await.expect("join"),
            Err(Error::NotConnected)
        ));
        assert!(!device.is_connected().await.expect("is_connected"));
    }
}
