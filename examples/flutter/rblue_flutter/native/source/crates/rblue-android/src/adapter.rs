//! Android [`Adapter`]: the shell around the forked btleplug `droidplug`
//! adapter (JNI to the `com.nonpolynomial.btleplug.android.impl` helper
//! classes).
//!
//! Owns the known-device registry, the btleplug event pump, and the
//! snapshot-then-live event hub. Device identity is [`DeviceId::Mac`] end to
//! end: the fork's `PeripheralId` wraps a `BDAddr`, so events are matched
//! back to registry entries by raw address bytes. The fork keeps one stable
//! `Peripheral` object per address for the adapter's lifetime, so unlike the
//! BlueZ shell there are no handle swaps to track.
//!
//! The fork tracks no radio state (`adapter_state` is a constant
//! `CentralState::Unknown`). The shell queries `BluetoothAdapter.getState()`
//! and listens for `ACTION_STATE_CHANGED` so `state()` and live
//! `StateChanged` events report the real radio. `initialize` still
//! succeeds when the radio is off (the first real operation can also report
//! it). `set_powered` is `Unsupported`: ordinary apps cannot reliably
//! enable or disable Bluetooth on modern Android.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use btleplug::api::{
    Central as _, CentralEvent, Manager as BtleManager, Peripheral as BtlePeripheral,
};
use futures::stream::{BoxStream, StreamExt};
use rblue_api::adapter::{Adapter, AdapterEventStream};
use rblue_api::capability::{Capability, CapabilitySet};
use rblue_api::device::{AdapterId, DeviceId};
use rblue_api::error::{Error, PermissionReason};
use rblue_api::event::{AdapterEvent, AdapterState, DisconnectReason};
use rblue_api::gatt::{CharacteristicId, ServiceSet};
use rblue_api::peripheral::Peripheral;
use rblue_api::scan::{AdvertisementData, DiscoveredDevice, RequestDeviceOptions, ScanFilter};
use tokio::sync::{broadcast, Mutex as AsyncMutex};

use crate::hub::{broadcast_stream, EventHub, ADAPTER_EVENT_CAPACITY};
use crate::mapping::{
    advertisement_from_properties, device_id_from, mac_bytes_from_id, map_btleplug_error,
    map_central_state, matches_filter, PLATFORM,
};
use crate::peripheral::{spawn_notification_forwarder, AndroidPeripheral, NotificationHub};

/// Static capability table for the plan's Android column: active scanning
/// and GATT client are available, the negotiated ATT MTU is queryable, an
/// explicit MTU request is available (`request_mtu` / connect-time
/// `ConnectOptions.mtu`), and RSSI is readable on the connected link
/// (`readRemoteRssi`). Absent on this platform: active pairing (`createBond`
/// is not exposed), background scanning (API 31+ requires a foreground
/// service - conservative), and the web-only picker. `ConnectionPriority`
/// is honored through the fork's `request_connection_parameters` mapping
/// onto `requestConnectionPriority`. `SystemConnectedPeripherals` uses a
/// shell-side `BluetoothManager.getConnectedDevices(GATT)` query filtered
/// by cached `BluetoothDevice.getUuids()`.
const CAPABILITIES: CapabilitySet = CapabilitySet::bit(Capability::ActiveScan)
    .union(CapabilitySet::bit(Capability::GattClient))
    .union(CapabilitySet::bit(Capability::MtuQuery))
    .union(CapabilitySet::bit(Capability::MtuNegotiation))
    .union(CapabilitySet::bit(Capability::RssiDuringConnection))
    .union(CapabilitySet::bit(Capability::Pairing))
    .union(CapabilitySet::bit(Capability::ConnectionPriority))
    .union(CapabilitySet::bit(Capability::SystemConnectedPeripherals));

/// Registry entry for one device ever sighted by this adapter. Lives until
/// the adapter is dropped: `Error::NotFound` is reserved for devices that
/// were never seen, so disconnects must not evict anything.
pub(crate) struct DeviceEntry {
    id: DeviceId,
    address: [u8; 6],
    /// The fork's peripheral object. One object exists per address for the
    /// fork's lifetime; the slot only records it (and lets the notification
    /// forwarder reach it).
    handle: Mutex<Option<btleplug::platform::Peripheral>>,
    advertisement: Mutex<AdvertisementData>,
    connected: AtomicBool,
    /// GATT tree of the current connection, from the last `discover_services`
    /// on this device. Session-scoped by contract; cleared on disconnect
    /// together with the CCCD state.
    services: Mutex<Option<ServiceSet>>,
    /// Characteristics subscribed on the current connection. Cleared on
    /// disconnect: the CCCD state dies with the link, and a bonded device's
    /// stack-side CCCD persistence must not leak values into a session that
    /// has not re-subscribed.
    subscriptions: Mutex<HashSet<CharacteristicId>>,
    notifications: OnceLock<NotificationHub>,
}

impl DeviceEntry {
    fn new(
        id: DeviceId,
        address: [u8; 6],
        peripheral: btleplug::platform::Peripheral,
        advertisement: AdvertisementData,
    ) -> Self {
        Self {
            id,
            address,
            handle: Mutex::new(Some(peripheral)),
            advertisement: Mutex::new(advertisement),
            connected: AtomicBool::new(false),
            services: Mutex::new(None),
            subscriptions: Mutex::new(HashSet::new()),
            notifications: OnceLock::new(),
        }
    }

    pub(crate) fn id(&self) -> &DeviceId {
        &self.id
    }

    pub(crate) fn address(&self) -> [u8; 6] {
        self.address
    }

    pub(crate) fn advertisement(&self) -> AdvertisementData {
        self.lock_advertisement().clone()
    }

    fn set_advertisement(&self, advertisement: AdvertisementData) {
        *self.lock_advertisement() = advertisement;
    }

    pub(crate) fn name(&self) -> Option<String> {
        self.advertisement().local_name
    }

    pub(crate) fn current_handle(&self) -> Option<btleplug::platform::Peripheral> {
        self.lock_handle().clone()
    }

    /// Installs the fork handle when the slot is empty. The fork keeps one
    /// object per address, so in practice the constructor already filled
    /// the slot; this only covers a defensive re-sighting.
    fn ensure_handle(&self, peripheral: btleplug::platform::Peripheral) {
        let mut handle = self.lock_handle();
        if handle.is_none() {
            *handle = Some(peripheral);
        }
    }

    // Starts the notification hub on first use: the forwarder task needs a
    // weak reference, so construction goes through an `Arc<DeviceEntry>`.
    pub(crate) fn notifications_hub(entry: &Arc<Self>) -> &NotificationHub {
        entry.notifications.get_or_init(|| {
            let (sender, _) = broadcast::channel(crate::hub::NOTIFICATION_CAPACITY);
            spawn_notification_forwarder(Arc::downgrade(entry), sender.clone());
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
    /// the fork's `NotConnectedException` (which would be indistinguishable
    /// anyway, but keeps the failure at the shell's semantics).
    pub(crate) async fn connected_handle(&self) -> Result<btleplug::platform::Peripheral, Error> {
        if !self.is_connected().await? {
            return Err(Error::NotConnected);
        }
        self.current_handle().ok_or(Error::NotConnected)
    }

    pub(crate) fn services(&self) -> Option<ServiceSet> {
        self.lock_services().clone()
    }

    pub(crate) fn store_services(&self, services: ServiceSet) {
        *self.lock_services() = Some(services);
    }

    pub(crate) fn is_subscribed(&self, id: &CharacteristicId) -> bool {
        self.lock_subscriptions().contains(id)
    }

    pub(crate) fn add_subscription(&self, id: CharacteristicId) {
        self.lock_subscriptions().insert(id);
    }

    pub(crate) fn remove_subscription(&self, id: &CharacteristicId) {
        self.lock_subscriptions().remove(id);
    }

    /// CCCD semantics: subscriptions die with the connection, and the
    /// identifiers of the previous session must not resolve against the
    /// next one.
    fn clear_session(&self) {
        *self.lock_services() = None;
        self.lock_subscriptions().clear();
    }

    /// Refreshes the cached name from the fork's properties snapshot. The
    /// fork updates its snapshot with the system-cached GAP name after a
    /// successful connect - more reliable than the advertised name for the
    /// supervisor's reconnect-by-filter matching.
    pub(crate) async fn refresh_name(&self, peripheral: &btleplug::platform::Peripheral) {
        if let Ok(Some(properties)) = peripheral.properties().await {
            if let Some(name) = properties.local_name {
                self.lock_advertisement().local_name = Some(name);
            }
        }
    }

    fn lock_handle(&self) -> MutexGuard<'_, Option<btleplug::platform::Peripheral>> {
        self.handle
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

    fn lock_subscriptions(&self) -> MutexGuard<'_, HashSet<CharacteristicId>> {
        self.subscriptions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Mutable adapter state behind the state lock. The lock is held across the
/// event publications inside the pump, so the snapshot-then-live replay in
/// [`Adapter::events`] can never interleave with a live event.
#[derive(Default)]
struct AdapterInner {
    scanning: bool,
    filter: Option<ScanFilter>,
    devices: Vec<Arc<DeviceEntry>>,
}

impl AdapterInner {
    fn find(&self, address: &[u8; 6]) -> Option<Arc<DeviceEntry>> {
        self.devices
            .iter()
            .find(|entry| entry.address() == *address)
            .cloned()
    }
}

/// Shared state of the Android adapter.
struct AdapterShared {
    adapter_id: AdapterId,
    state: Mutex<AdapterInner>,
    events: EventHub,
    /// The btleplug adapter once `initialize` fetched it. A tokio mutex
    /// because initialization awaits the fork's manager while holding it;
    /// the state lock above is never held across an await.
    initialized: AsyncMutex<Option<btleplug::platform::Adapter>>,
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
            // The fork emits `DeviceDiscovered` on a first sighting and
            // `DeviceUpdated` on every re-sighting; both carry the complete
            // properties snapshot (already cached on the peripheral object
            // by the fork's JNI scan callback), so a full refresh covers
            // every per-advertisement-field change.
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                self.record_sighting(adapter, id).await;
            }
            // Never emitted by droidplug today; handled for completeness
            // with the same merge-then-record fallback as the full sighting.
            CentralEvent::RssiUpdate { id, rssi } => {
                let Ok(peripheral) = adapter.peripheral(&id).await else {
                    return;
                };
                if !self.update_known_device(peripheral.address().into_inner(), |advertisement| {
                    advertisement.rssi = Some(rssi);
                }) {
                    self.record_sighting(adapter, id).await;
                }
            }
            // The fork emits the manufacturer-data, service-data, and
            // services events for every sighting together with the
            // full-properties event above; the snapshot they mirror is
            // already merged by `record_sighting`, so they are no-ops here.
            CentralEvent::ManufacturerDataAdvertisement { .. }
            | CentralEvent::ServiceDataAdvertisement { .. }
            | CentralEvent::ServicesAdvertisement { .. }
            | CentralEvent::DeviceServicesModified(_) => {}
            CentralEvent::DeviceConnected(id) => {
                if let Ok(peripheral) = adapter.peripheral(&id).await {
                    self.record_connection_state(peripheral.address().into_inner(), true);
                }
            }
            CentralEvent::DeviceDisconnected(id) => {
                if let Ok(peripheral) = adapter.peripheral(&id).await {
                    self.record_connection_state(peripheral.address().into_inner(), false);
                }
            }
            CentralEvent::StateUpdate(state) => {
                self.events
                    .publish(AdapterEvent::StateChanged(map_central_state(state)));
            }
        }
    }

    /// Records a device sighting: refreshes the registry entry and publishes
    /// the discovery or update event while still holding the state lock.
    /// Filtered-out sightings are silently dropped, exactly like the mock
    /// backend.
    async fn record_sighting(
        &self,
        adapter: &btleplug::platform::Adapter,
        id: btleplug::platform::PeripheralId,
    ) {
        let Ok(peripheral) = adapter.peripheral(&id).await else {
            return;
        };
        let Ok(Some(properties)) = peripheral.properties().await else {
            return;
        };
        let advertisement = advertisement_from_properties(&properties);
        let address = properties.address.into_inner();
        let device_id = device_id_from(properties.address);

        let mut inner = self.lock_state();
        if inner.scanning && !matches_filter(&advertisement, inner.filter.as_ref()) {
            return;
        }
        if let Some(entry) = inner.find(&address) {
            entry.set_advertisement(advertisement);
            // A re-sighting hands back the same fork object; the connected
            // entry keeps using it either way.
            entry.ensure_handle(peripheral);
            publish_sighting_update(&self.events, &entry);
        } else {
            let entry = Arc::new(DeviceEntry::new(
                device_id,
                address,
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

    /// Applies a property merge to a known entry and publishes the update,
    /// under the state lock. Returns `false` when the device is unknown to
    /// the registry so the caller can record a full sighting instead. The
    /// merge is evaluated against the active scan filter on the merged
    /// snapshot first; filtered-out devices are dropped during a scan, like
    /// `record_sighting`, and the entry keeps its cached data.
    fn update_known_device(
        &self,
        address: [u8; 6],
        update: impl FnOnce(&mut AdvertisementData),
    ) -> bool {
        let inner = self.lock_state();
        let Some(entry) = inner.find(&address) else {
            return false;
        };
        let mut merged = entry.advertisement();
        update(&mut merged);
        if inner.scanning && !matches_filter(&merged, inner.filter.as_ref()) {
            return true;
        }
        entry.set_advertisement(merged);
        publish_sighting_update(&self.events, &entry);
        true
    }

    fn record_connection_state(&self, address: [u8; 6], connected: bool) {
        let inner = self.lock_state();
        let Some(entry) = inner.find(&address) else {
            return;
        };
        // Publish on transitions only: a connect call racing a sighting can
        // repeat the same state.
        if entry.connected.swap(connected, Ordering::AcqRel) != connected {
            let event = if connected {
                AdapterEvent::DeviceConnected(entry.id().clone())
            } else {
                entry.clear_session();
                // The fork's disconnect notification carries no status
                // code, so the raw-code reason cannot be populated here
                // (fork limitation - see docs/error-mapping.md).
                AdapterEvent::DeviceDisconnected(
                    entry.id().clone(),
                    Some(DisconnectReason::Unknown),
                )
            };
            self.events.publish(event);
        }
    }
}

/// Publishes a `DeviceUpdated` for a known entry. Without an RSSI there is
/// nothing the unified update event can carry, so the sighting is dropped.
fn publish_sighting_update(events: &EventHub, entry: &DeviceEntry) {
    if let Some(rssi) = entry.advertisement().rssi {
        events.publish(AdapterEvent::DeviceUpdated(
            entry.id().clone(),
            rblue_api::device::Rssi::new(rssi),
        ));
    }
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

/// The Android adapter backend. Obtain via
/// [`default_adapter`](crate::default_adapter) after
/// [`init`](crate::init); all trait methods are safe to call concurrently.
pub(crate) struct AndroidAdapter {
    shared: Arc<AdapterShared>,
}

impl AndroidAdapter {
    pub(crate) fn new() -> Self {
        let shared = Arc::new(AdapterShared {
            adapter_id: AdapterId::default_adapter(),
            state: Mutex::new(AdapterInner::default()),
            events: EventHub::default(),
            initialized: AsyncMutex::new(None),
        });
        let weak = Arc::downgrade(&shared);
        crate::radio::register_publisher(move |event| {
            if let Some(shared) = weak.upgrade() {
                shared.events.publish(event);
            }
        });
        Self { shared }
    }

    /// The btleplug adapter handle, once [`Adapter::initialize`] fetched it
    /// from the fork's global-adapter singleton.
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
            // Initialize stays successful when the radio is off: Android
            // apps commonly open the adapter, then prompt the user to turn
            // Bluetooth on. Scan/connect still report the radio condition.
            AdapterState::PoweredOn | AdapterState::Unknown | AdapterState::PoweredOff => Ok(()),
            AdapterState::Unauthorized => Err(Error::Permission(PermissionReason::Unauthorized)),
            AdapterState::Unavailable => Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "adapter",
            }),
        }
    }

    /// Prefers a JNI / broadcast reading over the fork's constant `Unknown`.
    fn resolved_radio_state(&self, fork_state: AdapterState) -> AdapterState {
        match crate::radio::current_state() {
            AdapterState::Unknown => fork_state,
            state => state,
        }
    }

    /// Fetches the fork's global adapter. Requires
    /// [`init`](crate::init) to have run: the fork's singleton panics on a
    /// missing `platform::init`, and the shell's flag check is what keeps
    /// that panic unreachable.
    async fn platform_adapter() -> Result<btleplug::platform::Adapter, Error> {
        if !crate::jni_initialized() {
            return Err(Error::InvalidState(
                "backend not initialized; call rblue_android::init() with the \
                 application JNIEnv first"
                    .to_owned(),
            ));
        }
        let manager = btleplug::platform::Manager::new()
            .await
            .map_err(map_btleplug_error)?;
        manager
            .adapters()
            .await
            .map_err(map_btleplug_error)?
            .into_iter()
            .next()
            .ok_or(Error::Unsupported {
                platform: PLATFORM,
                feature: "adapter",
            })
    }
}

/// Constructs a fork peripheral for `address`. droidplug's `PeripheralId`
/// field is `pub(super)`, so the shell rebuilds the id through the type's
/// serde representation (the colon-delimited MAC string).
async fn fork_peripheral_from_address(
    adapter: &btleplug::platform::Adapter,
    address: [u8; 6],
) -> Result<btleplug::platform::Peripheral, Error> {
    let id = peripheral_id_from_mac(address)?;
    match adapter.peripheral(&id).await {
        Ok(existing) => Ok(existing),
        Err(_) => adapter
            .add_peripheral(&id)
            .await
            .map_err(map_btleplug_error),
    }
}

fn peripheral_id_from_mac(address: [u8; 6]) -> Result<btleplug::platform::PeripheralId, Error> {
    let formatted = btleplug::api::BDAddr::from(address).to_string();
    serde_json::from_str(&format!("\"{formatted}\""))
        .map_err(|err| Error::Backend(format!("android peripheral id: {err}")))
}

#[async_trait::async_trait]
impl Adapter for AndroidAdapter {
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
            // current radio state (JNI / last broadcast, not the fork
            // constant).
            let state = adapter.adapter_state().await.map_err(map_btleplug_error)?;
            return self.resolve_state(self.resolved_radio_state(map_central_state(state)));
        }
        // Subscribe to the event stream before anything can publish into
        // it. No Bluetooth operation happens here: the fork's adapter
        // object was created during `init` (from the application thread
        // with the visible ClassLoader). Radio state is read from
        // BluetoothAdapter, not the fork's constant `Unknown`.
        let adapter = Self::platform_adapter().await?;
        let events = adapter.events().await.map_err(map_btleplug_error)?;
        let state = adapter.adapter_state().await.map_err(map_btleplug_error)?;
        tokio::spawn(event_pump(
            Arc::downgrade(&self.shared),
            adapter.clone(),
            events,
        ));
        *initialized = Some(adapter);
        self.resolve_state(self.resolved_radio_state(map_central_state(state)))
    }

    async fn state(&self) -> Result<AdapterState, Error> {
        Ok(crate::radio::current_state())
    }

    async fn set_powered(&self, _powered: bool) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "set_powered",
        })
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
                // Pushed down through the ScanFilter parcel; the remaining
                // filter fields are applied in software (see matches_filter).
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
            .map(|entry| Arc::new(AndroidPeripheral::new(entry.clone())) as Arc<dyn Peripheral>)
            .collect())
    }

    async fn system_connected_peripherals(
        &self,
        services: &[uuid::Uuid],
    ) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        if services.is_empty() {
            return Err(Error::InvalidState(
                "at least one service UUID is required".into(),
            ));
        }
        let adapter = self.initialized_adapter().await?;
        let reported = crate::system_connected::query_gatt_connected()?;
        let mut results: Vec<Arc<dyn Peripheral>> = Vec::new();
        for device in reported {
            if !crate::system_connected::matches_requested_services(&device.services, services) {
                continue;
            }
            let peripheral = fork_peripheral_from_address(&adapter, device.address).await?;
            let advertisement = AdvertisementData {
                local_name: device.name.clone(),
                services: device.services.clone(),
                ..AdvertisementData::default()
            };
            let device_id = device_id_from(btleplug::api::BDAddr::from(device.address));
            let mut inner = self.shared.lock_state();
            let entry = if let Some(entry) = inner.find(&device.address) {
                entry.ensure_handle(peripheral);
                entry
            } else {
                let entry = Arc::new(DeviceEntry::new(
                    device_id,
                    device.address,
                    peripheral,
                    advertisement,
                ));
                inner.devices.push(entry.clone());
                entry
            };
            results.push(Arc::new(AndroidPeripheral::new(entry)) as Arc<dyn Peripheral>);
        }
        Ok(results)
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        let Some(address) = mac_bytes_from_id(id) else {
            // Android devices are only ever identified by MAC.
            return Err(Error::NotFound(id.clone()));
        };
        let inner = self.shared.lock_state();
        inner
            .find(&address)
            .map(|entry| Arc::new(AndroidPeripheral::new(entry)) as Arc<dyn Peripheral>)
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
