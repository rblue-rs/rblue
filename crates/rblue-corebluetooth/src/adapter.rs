//! CoreBluetooth [`Adapter`]: the shell around the forked btleplug
//! `CBCentralManager` implementation.
//!
//! Owns the known-device registry, the btleplug event pump, and the
//! snapshot-then-live event hub. Device identity is [`DeviceId::Uuid`] end to
//! end: CoreBluetooth never exposes a MAC, and the fork's `PeripheralId`
//! wraps the CB identifier `NSUUID` directly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use btleplug::api::{
    Central as _, CentralEvent, Manager as BtleManager, Peripheral as BtlePeripheral,
    RetrievePeripheralsOptions,
};
use futures::stream::{BoxStream, StreamExt};
use rblue_api::adapter::{Adapter, AdapterEventStream};
use rblue_api::capability::{Capability, CapabilitySet};
use rblue_api::device::{AdapterId, DeviceId, Rssi};
use rblue_api::error::{Error, PermissionReason};
use rblue_api::event::{AdapterEvent, AdapterState, DisconnectReason};
use rblue_api::gatt::ServiceSet;
use rblue_api::peripheral::Peripheral;
use rblue_api::scan::{AdvertisementData, DiscoveredDevice, RequestDeviceOptions, ScanFilter};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;

use crate::hub::{broadcast_stream, EventHub, ADAPTER_EVENT_CAPACITY};
use crate::mapping::{
    advertisement_from_properties, map_btleplug_error, map_central_state, peripheral_uuid, PLATFORM,
};
use crate::peripheral::{CoreBluetoothPeripheral, NotificationHub};

/// Static capability table for the macOS/iOS column of the plan's matrix:
/// active scanning, GATT client, and connected-RSSI reads are available; MTU
/// is system-managed on both sides of the API; pairing is implicit;
/// `requestDevice` is web-only; background scanning is conservatively
/// reported as unavailable because iOS silently yields nothing without a
/// service-UUID filter and macOS needs the bluetooth-central background mode.
const CAPABILITIES: CapabilitySet = CapabilitySet::bit(Capability::ActiveScan)
    .union(CapabilitySet::bit(Capability::GattClient))
    .union(CapabilitySet::bit(Capability::RssiDuringConnection));

/// How a btleplug sighting mutates the registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SightingKind {
    /// Full sighting: the advertisement snapshot replaces the cached one.
    Discovered,
    /// Name-only update (GAP name cache refreshed).
    Renamed,
}

/// Registry entry for one device ever sighted by this adapter. Lives until
/// the adapter is dropped: `Error::NotFound` is reserved for devices that
/// were never seen, so disconnects must not evict anything.
pub(crate) struct DeviceEntry {
    id: DeviceId,
    uuid: Uuid,
    /// The creating adapter, so facades can run
    /// `retrievePeripheralsWithIdentifiers` when reconnecting after the fork
    /// dropped its internal state. `None` only in unit-test entries.
    adapter: Option<btleplug::platform::Adapter>,
    /// The latest btleplug peripheral object plus a generation counter the
    /// notification forwarder watches for handle swaps.
    slot: Mutex<HandleSlot>,
    /// Wakes the notification forwarder whenever the handle is replaced.
    generation_sender: watch::Sender<u64>,
    advertisement: Mutex<AdvertisementData>,
    connected: AtomicBool,
    /// Set when the link dropped (the fork removes its internal state) and
    /// cleared once a fresh handle was installed; `connect` uses it to decide
    /// whether retrieval by identifier is needed.
    needs_refresh: AtomicBool,
    /// GATT tree of the current connection, from the last `discover_services`
    /// on this device. Session-scoped by contract.
    services: Mutex<Option<ServiceSet>>,
    notifications: OnceLock<NotificationHub>,
}

struct HandleSlot {
    peripheral: Option<btleplug::platform::Peripheral>,
    generation: u64,
}

impl DeviceEntry {
    fn new(
        uuid: Uuid,
        adapter: btleplug::platform::Adapter,
        peripheral: btleplug::platform::Peripheral,
        advertisement: AdvertisementData,
    ) -> Self {
        let (generation_sender, _) = watch::channel(1_u64);
        Self {
            id: DeviceId::Uuid(uuid),
            uuid,
            adapter: Some(adapter),
            slot: Mutex::new(HandleSlot {
                peripheral: Some(peripheral),
                generation: 1,
            }),
            generation_sender,
            advertisement: Mutex::new(advertisement),
            connected: AtomicBool::new(false),
            needs_refresh: AtomicBool::new(false),
            services: Mutex::new(None),
            notifications: OnceLock::new(),
        }
    }

    /// Registry entry without platform handles, for unit tests.
    #[cfg(test)]
    fn for_tests(uuid: Uuid, advertisement: AdvertisementData) -> Self {
        let (generation_sender, _) = watch::channel(0_u64);
        Self {
            id: DeviceId::Uuid(uuid),
            uuid,
            adapter: None,
            slot: Mutex::new(HandleSlot {
                peripheral: None,
                generation: 0,
            }),
            generation_sender,
            advertisement: Mutex::new(advertisement),
            connected: AtomicBool::new(false),
            needs_refresh: AtomicBool::new(false),
            services: Mutex::new(None),
            notifications: OnceLock::new(),
        }
    }

    pub(crate) fn id(&self) -> &DeviceId {
        &self.id
    }

    pub(crate) fn advertisement(&self) -> AdvertisementData {
        self.lock_advertisement().clone()
    }

    fn set_advertisement(&self, advertisement: AdvertisementData) {
        *self.lock_advertisement() = advertisement;
    }

    fn patch_advertisement(&self, patch: impl FnOnce(&mut AdvertisementData)) {
        patch(&mut self.lock_advertisement());
    }

    pub(crate) fn name(&self) -> Option<String> {
        self.advertisement().local_name
    }

    pub(crate) fn current_handle(&self) -> Option<btleplug::platform::Peripheral> {
        self.lock_slot().peripheral.clone()
    }

    /// Installs a fresh btleplug handle and wakes the notification
    /// forwarder, which re-subscribes on the new object.
    fn set_handle(&self, peripheral: btleplug::platform::Peripheral) {
        let mut slot = self.lock_slot();
        slot.peripheral = Some(peripheral);
        slot.generation += 1;
        let _ = self.generation_sender.send_replace(slot.generation);
        self.needs_refresh.store(false, Ordering::Release);
    }

    pub(crate) fn handle_generation(&self) -> watch::Receiver<u64> {
        self.generation_sender.subscribe()
    }

    // Starts the notification hub on first use: the forwarder task needs a
    // weak reference, so construction goes through an `Arc<DeviceEntry>`.
    pub(crate) fn notifications_hub(entry: &Arc<Self>) -> &NotificationHub {
        entry.notifications.get_or_init(|| {
            let (sender, _) = broadcast::channel(crate::hub::NOTIFICATION_CAPACITY);
            crate::peripheral::spawn_notification_forwarder(entry, sender.clone());
            NotificationHub { sender }
        })
    }

    pub(crate) async fn is_connected(&self) -> Result<bool, Error> {
        match self.current_handle() {
            Some(peripheral) => peripheral.is_connected().await.map_err(map_btleplug_error),
            None => Ok(false),
        }
    }

    /// Returns the current handle after verifying the link is up, so
    /// disconnected GATT calls fail with [`Error::NotConnected`] instead of
    /// the fork's drained-future errors.
    pub(crate) async fn connected_handle(&self) -> Result<btleplug::platform::Peripheral, Error> {
        if !self.is_connected().await? {
            return Err(Error::NotConnected);
        }
        self.current_handle().ok_or(Error::NotConnected)
    }

    /// Refreshes the handle through `retrievePeripheralsWithIdentifiers`.
    ///
    /// The fork removes a peripheral's internal state when it disconnects, so
    /// a reconnect needs the CB object re-retrieved before `connectPeripheral`
    /// can work. Retrieval by identifier needs no scan; the stale handle is
    /// kept when CoreBluetooth no longer knows the identifier, and the
    /// subsequent connect attempt reports the device as gone.
    pub(crate) async fn refresh_handle(&self) {
        if !self.needs_refresh.swap(false, Ordering::AcqRel) {
            return;
        }
        let Some(adapter) = self.adapter.as_ref() else {
            return;
        };
        let options = RetrievePeripheralsOptions {
            identifiers: Some(vec![btleplug::platform::PeripheralId::from(self.uuid)]),
            services: None,
        };
        if let Ok(retrieved) = adapter.retrieve_peripherals(options).await {
            match retrieved.into_iter().next() {
                Some(peripheral) => self.set_handle(peripheral),
                None => self.needs_refresh.store(true, Ordering::Release),
            }
        } else {
            self.needs_refresh.store(true, Ordering::Release);
        }
    }

    pub(crate) fn services(&self) -> Option<ServiceSet> {
        self.lock_services().clone()
    }

    pub(crate) fn store_services(&self, services: ServiceSet) {
        *self.lock_services() = Some(services);
    }

    pub(crate) fn clear_services(&self) {
        *self.lock_services() = None;
    }

    fn lock_slot(&self) -> MutexGuard<'_, HandleSlot> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_advertisement(&self) -> MutexGuard<'_, AdvertisementData> {
        self.advertisement
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_services(&self) -> MutexGuard<'_, Option<ServiceSet>> {
        self.services
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Mutable adapter state behind the state lock. The lock is held across the
/// event publications inside the pump, so the snapshot-then-live replay in
/// [`Adapter::events`] can never interleave with or duplicate a live event.
#[derive(Default)]
struct AdapterInner {
    scanning: bool,
    filter: Option<ScanFilter>,
    devices: Vec<Arc<DeviceEntry>>,
}

impl AdapterInner {
    fn find(&self, uuid: Uuid) -> Option<Arc<DeviceEntry>> {
        self.devices
            .iter()
            .find(|entry| entry.uuid == uuid)
            .cloned()
    }
}

/// Shared state of the CoreBluetooth adapter.
struct AdapterShared {
    adapter_id: AdapterId,
    state: Mutex<AdapterInner>,
    events: EventHub,
    /// The btleplug adapter once `initialize` created it. A tokio mutex
    /// because initialization awaits `centralManagerDidUpdateState` while
    /// holding it; the state lock above is never held across an await.
    initialized: tokio::sync::Mutex<Option<btleplug::platform::Adapter>>,
}

impl AdapterShared {
    fn lock_state(&self) -> MutexGuard<'_, AdapterInner> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Handles one btleplug central event. Awaits before taking the state
    /// lock; publications happen under the lock.
    async fn handle_event(&self, adapter: &btleplug::platform::Adapter, event: CentralEvent) {
        match event {
            CentralEvent::DeviceDiscovered(id) => {
                self.record_sighting(adapter, id, SightingKind::Discovered)
                    .await;
            }
            CentralEvent::DeviceUpdated(id) => {
                self.record_sighting(adapter, id, SightingKind::Renamed)
                    .await;
            }
            CentralEvent::DeviceConnected(id) => self.record_connection_state(id, true),
            CentralEvent::DeviceDisconnected(id) => self.record_connection_state(id, false),
            CentralEvent::DeviceServicesModified(id) => self.record_services_modified(id),
            CentralEvent::ManufacturerDataAdvertisement {
                id,
                manufacturer_data,
            } => {
                self.record_advertisement_patch(id, |adv| {
                    adv.manufacturer_data = manufacturer_data
                        .iter()
                        .min_by_key(|(company_id, _)| *company_id)
                        .map(|(company_id, data)| (*company_id, data.clone()));
                })
                .await;
            }
            CentralEvent::ServiceDataAdvertisement { id, service_data } => {
                self.record_advertisement_patch(id, |adv| {
                    adv.service_data = service_data
                        .iter()
                        .map(|(uuid, data)| (*uuid, data.clone()))
                        .collect();
                })
                .await;
            }
            CentralEvent::ServicesAdvertisement { id, services } => {
                self.record_advertisement_patch(id, |adv| {
                    adv.services = services;
                })
                .await;
            }
            CentralEvent::RssiUpdate { id, rssi } => {
                self.record_advertisement_patch(id, |adv| {
                    adv.rssi = Some(rssi);
                })
                .await;
            }
            CentralEvent::StateUpdate(state) => {
                self.events
                    .publish(AdapterEvent::StateChanged(map_central_state(state)));
            }
        }
    }

    /// Records a device sighting: refreshes the registry entry, applies the
    /// software part of the active scan filter, and publishes the discovery
    /// or update event while still holding the state lock. Filtered-out
    /// sightings are silently dropped, exactly like the mock backend.
    async fn record_sighting(
        &self,
        adapter: &btleplug::platform::Adapter,
        id: btleplug::platform::PeripheralId,
        kind: SightingKind,
    ) {
        let Ok(peripheral) = adapter.peripheral(&id).await else {
            return;
        };
        let Ok(Some(properties)) = peripheral.properties().await else {
            return;
        };
        let advertisement = advertisement_from_properties(&properties);
        let Some(uuid) = peripheral_uuid(&id) else {
            return;
        };

        let mut inner = self.lock_state();
        let filtering = inner.scanning;
        if filtering && !matches_filter(&advertisement, inner.filter.as_ref()) {
            return;
        }
        if let Some(entry) = inner.find(uuid) {
            match kind {
                SightingKind::Discovered => {
                    entry.set_advertisement(advertisement);
                    // The fork rebuilt its internal state for this device
                    // (re-sighting after a disconnect): install the new
                    // handle so reconnects and notifications keep working.
                    if !entry.connected.load(Ordering::Acquire) {
                        entry.set_handle(peripheral);
                    }
                }
                SightingKind::Renamed => {
                    if advertisement.local_name.is_some() {
                        let local_name = advertisement.local_name;
                        entry.patch_advertisement(|adv| adv.local_name = local_name);
                    }
                }
            }
            publish_sighting_update(&self.events, &entry);
        } else {
            let entry = Arc::new(DeviceEntry::new(
                uuid,
                adapter.clone(),
                peripheral,
                advertisement,
            ));
            inner.devices.push(entry.clone());
            self.events
                .publish(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                    id: entry.id().clone(),
                    advertisement: entry.advertisement(),
                }));
        }
    }

    /// Records advertisement-field refinements (manufacturer/service data,
    /// service list, RSSI) as updates. Sightings for unknown devices are
    /// ignored: the fork always emits `DeviceDiscovered` first, so this only
    /// happens when events predate the shell's subscription.
    async fn record_advertisement_patch(
        &self,
        id: btleplug::platform::PeripheralId,
        patch: impl FnOnce(&mut AdvertisementData),
    ) {
        let Some(uuid) = peripheral_uuid(&id) else {
            return;
        };
        let inner = self.lock_state();
        let Some(entry) = inner.find(uuid) else {
            return;
        };
        // Apply to a copy first: a patch that fails the active scan filter
        // must leave the cached advertisement untouched, like any other
        // filtered-out sighting.
        let mut patched = entry.advertisement();
        patch(&mut patched);
        if inner.scanning && !matches_filter(&patched, inner.filter.as_ref()) {
            return;
        }
        entry.set_advertisement(patched);
        publish_sighting_update(&self.events, &entry);
    }

    fn record_connection_state(&self, id: btleplug::platform::PeripheralId, connected: bool) {
        let Some(uuid) = peripheral_uuid(&id) else {
            return;
        };
        let inner = self.lock_state();
        let Some(entry) = inner.find(uuid) else {
            return;
        };
        // The fork emits duplicate disconnect notifications (one from the
        // reply path, one from the delegate); publish on transitions only.
        if entry.connected.swap(connected, Ordering::AcqRel) != connected {
            let event = if connected {
                AdapterEvent::DeviceConnected(entry.id().clone())
            } else {
                entry.needs_refresh.store(true, Ordering::Release);
                AdapterEvent::DeviceDisconnected(
                    entry.id().clone(),
                    Some(DisconnectReason::Unknown),
                )
            };
            self.events.publish(event);
        }
    }

    fn record_services_modified(&self, id: btleplug::platform::PeripheralId) {
        let Some(uuid) = peripheral_uuid(&id) else {
            return;
        };
        let inner = self.lock_state();
        if let Some(entry) = inner.find(uuid) {
            // The GATT tree changed under us: cached identifiers and the
            // notification-id mapping are stale until rediscovery.
            entry.clear_services();
        }
    }
}

/// Publishes a `DeviceUpdated` for a known entry. Without an RSSI there is
/// nothing the unified update event can carry, so the sighting is dropped.
fn publish_sighting_update(events: &EventHub, entry: &DeviceEntry) {
    if let Some(rssi) = entry.advertisement().rssi {
        events.publish(AdapterEvent::DeviceUpdated(
            entry.id().clone(),
            Rssi::new(rssi),
        ));
    }
}

/// Software part of the scan filter: `name_prefix`, `manufacturer_data`, and
/// `rssi_threshold` can never be pushed down to CoreBluetooth hardware, so
/// the full advertisement stream is received and filtered here. Semantics
/// match the mock backend's `matches_scan_filter`.
pub(crate) fn matches_filter(
    advertisement: &AdvertisementData,
    filter: Option<&ScanFilter>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
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

/// The btleplug event pump: subscribes to the fork's central-event stream at
/// initialization time (so nothing is lost between adapter creation and the
/// first user subscription) and mirrors it into the registry and the rblue
/// event hub.
async fn event_pump(
    shared: Weak<AdapterShared>,
    adapter: btleplug::platform::Adapter,
    events: BoxStream<'static, CentralEvent>,
) {
    let mut events = events;
    while let Some(event) = events.next().await {
        let Some(shared) = shared.upgrade() else {
            break;
        };
        shared.handle_event(&adapter, event).await;
    }
}

/// The macOS/iOS adapter backend. Obtain via
/// [`default_adapter`](crate::default_adapter); all trait methods are safe to
/// call concurrently.
#[derive(Clone)]
pub(crate) struct CoreBluetoothAdapter {
    shared: Arc<AdapterShared>,
}

impl CoreBluetoothAdapter {
    pub(crate) fn new() -> Self {
        Self {
            shared: Arc::new(AdapterShared {
                adapter_id: AdapterId::default_adapter(),
                state: Mutex::new(AdapterInner::default()),
                events: EventHub::default(),
                initialized: tokio::sync::Mutex::new(None),
            }),
        }
    }

    /// The btleplug adapter handle, once [`Adapter::initialize`] created it.
    async fn initialized_adapter(&self) -> Result<btleplug::platform::Adapter, Error> {
        self.shared.initialized.lock().await.clone().ok_or_else(|| {
            Error::InvalidState("adapter not initialized; call initialize() first".to_owned())
        })
    }

    /// Publishes the resolved adapter state and translates it into the
    /// `initialize` result.
    fn resolve_state(&self, state: AdapterState) -> Result<(), Error> {
        self.shared
            .events
            .publish(AdapterEvent::StateChanged(state));
        match state {
            AdapterState::PoweredOn => Ok(()),
            AdapterState::PoweredOff => Err(Error::Permission(PermissionReason::BluetoothOff)),
            _ => Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "adapter",
            }),
        }
    }

    /// Re-reports the current radio state for a repeated `initialize`.
    async fn reinitialize(&self, adapter: &btleplug::platform::Adapter) -> Result<(), Error> {
        let state = adapter.adapter_state().await.map_err(map_btleplug_error)?;
        self.resolve_state(map_central_state(state))
    }
}

impl Default for CoreBluetoothAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Adapter for CoreBluetoothAdapter {
    fn id(&self) -> &AdapterId {
        &self.shared.adapter_id
    }

    fn capabilities(&self) -> CapabilitySet {
        CAPABILITIES
    }

    async fn initialize(&self) -> Result<(), Error> {
        let mut initialized = self.shared.initialized.lock().await;
        if let Some(adapter) = initialized.as_ref() {
            // Opens or wakes the adapter: a repeated call re-reports the
            // current radio state.
            return self.reinitialize(adapter).await;
        }
        let manager = btleplug::platform::Manager::new()
            .await
            .map_err(map_btleplug_error)?;
        // Adapter creation blocks until centralManagerDidUpdateState, so
        // every later platform call is valid once this returns.
        let adapter = manager
            .adapters()
            .await
            .map_err(map_btleplug_error)?
            .into_iter()
            .next()
            .ok_or(Error::Unsupported {
                platform: PLATFORM,
                feature: "adapter",
            })?;
        // Subscribe to the event stream before anything can publish into it.
        let events = adapter.events().await.map_err(map_btleplug_error)?;
        let state = adapter.adapter_state().await.map_err(map_btleplug_error)?;
        tokio::spawn(event_pump(
            Arc::downgrade(&self.shared),
            adapter.clone(),
            events,
        ));
        *initialized = Some(adapter);
        self.resolve_state(map_central_state(state))
    }

    async fn start_scan(&self, filter: &ScanFilter) -> Result<(), Error> {
        let adapter = self.initialized_adapter().await?;
        // Record the filter before the platform call so the event pump never
        // publishes an unfiltered sighting; revert when the stack rejects
        // the scan.
        {
            let mut inner = self.shared.lock_state();
            inner.scanning = true;
            inner.filter = Some(filter.clone());
        }
        if let Err(error) = adapter
            .start_scan(btleplug::api::ScanFilter {
                // Hardware filtering on CoreBluetooth supports service UUIDs
                // only; the remaining filter fields are applied in software.
                services: filter.services.clone(),
            })
            .await
        {
            let mut inner = self.shared.lock_state();
            inner.scanning = false;
            inner.filter = None;
            return Err(map_btleplug_error(error));
        }
        Ok(())
    }

    async fn stop_scan(&self) -> Result<(), Error> {
        if !self.shared.lock_state().scanning {
            return Err(Error::InvalidState("no scan is running".to_owned()));
        }
        let adapter = self.initialized_adapter().await?;
        adapter.stop_scan().await.map_err(map_btleplug_error)?;
        self.shared.lock_state().scanning = false;
        Ok(())
    }

    fn events(&self) -> AdapterEventStream {
        let (sender, receiver) = broadcast::channel(ADAPTER_EVENT_CAPACITY);
        {
            // Register, then replay the known-device snapshot, all under the
            // state lock: no sighting can slip between the two steps, and
            // devices known before the subscription arrive as fresh
            // discoveries.
            let inner = self.shared.lock_state();
            self.shared.events.register(sender.clone());
            for entry in &inner.devices {
                let _ = sender.send(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                    id: entry.id().clone(),
                    advertisement: entry.advertisement(),
                }));
            }
        }
        broadcast_stream(receiver).boxed()
    }

    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        let inner = self.shared.lock_state();
        Ok(inner
            .devices
            .iter()
            .map(|entry| {
                Arc::new(CoreBluetoothPeripheral::new(entry.clone())) as Arc<dyn Peripheral>
            })
            .collect())
    }

    async fn system_connected_peripherals(
        &self,
        services: &[Uuid],
    ) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        if services.is_empty() {
            return Err(Error::InvalidState(
                "at least one service UUID is required".into(),
            ));
        }
        let adapter = self.initialized_adapter().await?;
        let peripherals = adapter
            .retrieve_peripherals(RetrievePeripheralsOptions {
                identifiers: None,
                services: Some(services.to_vec()),
            })
            .await
            .map_err(map_btleplug_error)?;
        let mut results: Vec<Arc<dyn Peripheral>> = Vec::with_capacity(peripherals.len());
        for peripheral in peripherals {
            let Some(uuid) = peripheral_uuid(&peripheral.id()) else {
                continue;
            };
            let advertisement = peripheral
                .properties()
                .await
                .map_err(map_btleplug_error)?
                .as_ref()
                .map(advertisement_from_properties)
                .unwrap_or_default();
            let mut inner = self.shared.lock_state();
            let entry = if let Some(entry) = inner.find(uuid) {
                if !entry.connected.load(Ordering::Acquire) {
                    entry.set_handle(peripheral);
                }
                entry
            } else {
                let entry = Arc::new(DeviceEntry::new(
                    uuid,
                    adapter.clone(),
                    peripheral,
                    advertisement,
                ));
                inner.devices.push(entry.clone());
                entry
            };
            results.push(Arc::new(CoreBluetoothPeripheral::new(entry)));
        }
        Ok(results)
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        let DeviceId::Uuid(uuid) = id else {
            // CoreBluetooth devices are only ever identified by UUID.
            return Err(Error::NotFound(id.clone()));
        };
        let inner = self.shared.lock_state();
        inner
            .find(*uuid)
            .map(|entry| Arc::new(CoreBluetoothPeripheral::new(entry)) as Arc<dyn Peripheral>)
            .ok_or_else(|| Error::NotFound(id.clone()))
    }

    async fn request_device(
        &self,
        _opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "request_device",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::uuid;

    fn advertisement(rssi: i16, name: &str) -> AdvertisementData {
        AdvertisementData {
            local_name: Some(name.to_owned()),
            rssi: Some(rssi),
            ..AdvertisementData::default()
        }
    }

    fn entry(uuid_value: Uuid, name: &str) -> Arc<DeviceEntry> {
        Arc::new(DeviceEntry::for_tests(uuid_value, advertisement(-55, name)))
    }

    #[test]
    fn capabilities_match_platform_matrix() {
        let caps = CoreBluetoothAdapter::new().capabilities();
        assert!(caps.has(Capability::ActiveScan));
        assert!(caps.has(Capability::GattClient));
        assert!(caps.has(Capability::RssiDuringConnection));
        assert!(!caps.has(Capability::MtuNegotiation));
        assert!(!caps.has(Capability::MtuQuery));
        assert!(!caps.has(Capability::Pairing));
        assert!(!caps.has(Capability::RequestDevicePicker));
        assert!(!caps.has(Capability::BackgroundScan));
        assert!(!caps.has(Capability::ConnectionPriority));
    }

    #[test]
    fn name_prefix_filter_is_applied_in_software() {
        let filter = ScanFilter {
            name_prefix: Some("loud".to_owned()),
            ..ScanFilter::default()
        };
        assert!(matches_filter(
            &advertisement(-40, "loud-device"),
            Some(&filter)
        ));
        assert!(!matches_filter(
            &advertisement(-40, "quiet-device"),
            Some(&filter)
        ));
    }

    #[test]
    fn rssi_threshold_filter_is_applied_in_software() {
        let filter = ScanFilter {
            rssi_threshold: Some(-50),
            ..ScanFilter::default()
        };
        assert!(matches_filter(&advertisement(-40, "any"), Some(&filter)));
        assert!(!matches_filter(&advertisement(-60, "any"), Some(&filter)));
        // A sighting without RSSI passes the threshold check: the platform
        // did not report a value to compare against.
        let mut no_rssi = advertisement(-40, "any");
        no_rssi.rssi = None;
        assert!(matches_filter(&no_rssi, Some(&filter)));
    }

    #[test]
    fn service_and_manufacturer_filters_match_mock_semantics() {
        let battery = uuid!("0000180f-0000-1000-8000-00805f9b34fb");
        let mut adv = advertisement(-40, "any");
        adv.services = vec![battery];
        adv.manufacturer_data = Some((0x004C, vec![1, 2, 3]));

        let other_service = ScanFilter {
            services: vec![uuid!("0000fd6f-0000-1000-8000-00805f9b34fb")],
            ..ScanFilter::default()
        };
        assert!(!matches_filter(&adv, Some(&other_service)));
        let same_service = ScanFilter {
            services: vec![battery],
            ..ScanFilter::default()
        };
        assert!(matches_filter(&adv, Some(&same_service)));

        let matching_manufacturer = ScanFilter {
            manufacturer_data: Some(rblue_api::ManufacturerFilter {
                company_id: 0x004C,
                data_prefix: vec![1, 2],
            }),
            ..ScanFilter::default()
        };
        assert!(matches_filter(&adv, Some(&matching_manufacturer)));
        let mismatching_manufacturer = ScanFilter {
            manufacturer_data: Some(rblue_api::ManufacturerFilter {
                company_id: 0x004C,
                data_prefix: vec![9],
            }),
            ..ScanFilter::default()
        };
        assert!(!matches_filter(&adv, Some(&mismatching_manufacturer)));
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(matches_filter(&advertisement(-100, "any"), None));
        assert!(matches_filter(
            &advertisement(-100, "any"),
            Some(&ScanFilter::default())
        ));
    }

    #[tokio::test]
    async fn events_replay_known_devices_then_stay_live() {
        let adapter = CoreBluetoothAdapter::new();
        let known_uuid = uuid!("9fb0e9e1-7a34-4e2a-a4c5-9c34a2213f81");
        lock_devices(&adapter)
            .devices
            .push(entry(known_uuid, "known"));

        let mut events = adapter.events();
        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => {
                assert_eq!(discovered.id, DeviceId::Uuid(known_uuid));
                assert_eq!(
                    discovered.advertisement.local_name.as_deref(),
                    Some("known")
                );
                assert_eq!(discovered.advertisement.rssi, Some(-55));
            }
            other => panic!("expected snapshot discovery, got {other:?}"),
        }
        // Live publishes still reach the subscriber after the snapshot.
        adapter.shared.events.publish(AdapterEvent::DeviceUpdated(
            DeviceId::Uuid(known_uuid),
            Rssi::new(-60),
        ));
        assert!(matches!(
            events.next().await,
            Some(AdapterEvent::DeviceUpdated(..))
        ));
    }

    #[tokio::test]
    async fn peripheral_lookup_rules() {
        let adapter = CoreBluetoothAdapter::new();
        let known_uuid = uuid!("9fb0e9e1-7a34-4e2a-a4c5-9c34a2213f82");
        lock_devices(&adapter)
            .devices
            .push(entry(known_uuid, "known"));

        assert!(adapter
            .peripheral(&DeviceId::Uuid(known_uuid))
            .await
            .is_ok());
        // Never-seen devices of any kind report NotFound with the queried id.
        let mac = DeviceId::Mac([1, 2, 3, 4, 5, 6], rblue_api::AddressType::RandomStatic);
        assert!(matches!(
            adapter.peripheral(&mac).await,
            Err(Error::NotFound(returned)) if returned == mac
        ));
        let web = DeviceId::Web("web-id".to_owned());
        assert!(matches!(
            adapter.peripheral(&web).await,
            Err(Error::NotFound(returned)) if returned == web
        ));
    }

    #[tokio::test]
    async fn scans_before_initialize_are_invalid_state() {
        let adapter = CoreBluetoothAdapter::new();
        assert!(matches!(
            adapter.start_scan(&ScanFilter::default()).await,
            Err(Error::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn repeated_stop_scan_is_invalid_state() {
        let adapter = CoreBluetoothAdapter::new();
        assert!(matches!(
            adapter.stop_scan().await,
            Err(Error::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn connect_on_unlinked_entry_reports_not_connected() {
        let peripheral = CoreBluetoothPeripheral::new(entry(
            uuid!("9fb0e9e1-7a34-4e2a-a4c5-9c34a2213f83"),
            "unlinked",
        ));
        assert!(matches!(peripheral.is_connected().await, Ok(false)));
        assert!(matches!(
            peripheral
                .read(&rblue_api::CharacteristicId {
                    service: rblue_api::ServiceId {
                        uuid: uuid!("0000180f-0000-1000-8000-00805f9b34fb"),
                        instance: 0,
                    },
                    uuid: uuid!("00002a19-0000-1000-8000-00805f9b34fb"),
                    instance: 0,
                })
                .await,
            Err(Error::NotConnected)
        ));
    }

    #[tokio::test]
    async fn mtu_pair_and_request_device_are_unsupported() {
        let peripheral = CoreBluetoothPeripheral::new(entry(
            uuid!("9fb0e9e1-7a34-4e2a-a4c5-9c34a2213f84"),
            "unsupported",
        ));
        let any_id = rblue_api::CharacteristicId {
            service: rblue_api::ServiceId {
                uuid: uuid!("0000180f-0000-1000-8000-00805f9b34fb"),
                instance: 0,
            },
            uuid: uuid!("00002a19-0000-1000-8000-00805f9b34fb"),
            instance: 0,
        };
        assert!(matches!(
            peripheral.mtu().await,
            Err(Error::Unsupported {
                platform: "corebluetooth",
                feature: "mtu"
            })
        ));
        assert!(matches!(
            peripheral.pair().await,
            Err(Error::Unsupported {
                platform: "corebluetooth",
                feature: "pairing"
            })
        ));
        assert!(matches!(
            peripheral
                .max_write_len(&any_id, rblue_api::WriteMode::WithResponse)
                .await,
            Err(Error::NotConnected)
        ));
        let adapter = CoreBluetoothAdapter::new();
        assert!(matches!(
            adapter
                .request_device(&RequestDeviceOptions::default())
                .await,
            Err(Error::Unsupported {
                feature: "request_device",
                ..
            })
        ));
    }

    /// Test helper: mutable access to the device registry.
    fn lock_devices(adapter: &CoreBluetoothAdapter) -> MutexGuard<'_, AdapterInner> {
        adapter.shared.lock_state()
    }
}
