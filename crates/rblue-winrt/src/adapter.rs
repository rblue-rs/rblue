//! WinRT [`Adapter`]: the shell around the forked btleplug `winrtble`
//! adapter.
//!
//! Owns the known-device registry, the btleplug event pump, and the
//! snapshot-then-live event hub. Device identity is [`DeviceId::Mac`] end to
//! end: the fork's `PeripheralId` wraps the 6-byte address directly, and the
//! shell keys its registry on the raw address bytes (the `AddressType`
//! annotation is metadata, not identity).
//!
//! Platform fact honored throughout: a Windows central can only connect to
//! addresses its watcher has observed in this boot session, so the registry
//! only ever contains watcher-sighted devices and `Error::NotFound` means
//! "never seen" (see `docs/platform-notes.md`).

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
use rblue_api::gatt::ServiceSet;
use rblue_api::peripheral::Peripheral;
use rblue_api::scan::{AdvertisementData, DiscoveredDevice, RequestDeviceOptions, ScanFilter};
use tokio::sync::{broadcast, watch};

use crate::hub::{broadcast_stream, EventHub, ADAPTER_EVENT_CAPACITY};
use crate::mapping::{
    advertisement_from_properties, device_id_from, map_btleplug_error, map_central_state,
    matches_filter, PLATFORM,
};
use crate::peripheral::{NotificationHub, WinRtPeripheral};

/// Static capability table for the plan's Windows column: active scanning
/// and GATT client are available, the negotiated MTU is queryable through
/// `GattSession.MaxPduSize`, and the OS permits background scanning under
/// its own restrictions. MTU is negotiated by the stack (no request API),
/// RSSI is scan-time only, pairing exists in WinRT (`PairAsync`) but the
/// fork exposes no API for it, and `requestDevice` is web-only.
const CAPABILITIES: CapabilitySet = CapabilitySet::bit(Capability::ActiveScan)
    .union(CapabilitySet::bit(Capability::GattClient))
    .union(CapabilitySet::bit(Capability::MtuQuery))
    .union(CapabilitySet::bit(Capability::BackgroundScan));

/// Registry entry for one device ever sighted by this adapter. Lives until
/// the adapter is dropped: `Error::NotFound` is reserved for devices that
/// were never seen, so disconnects must not evict anything.
pub(crate) struct DeviceEntry {
    id: DeviceId,
    address: [u8; 6],
    /// The latest btleplug peripheral object plus a generation counter the
    /// notification forwarder watches for handle swaps.
    slot: Mutex<HandleSlot>,
    /// Wakes the notification forwarder whenever the handle is replaced.
    generation_sender: watch::Sender<u64>,
    advertisement: Mutex<AdvertisementData>,
    connected: AtomicBool,
    /// GATT tree of the current connection, from the last `discover_services`
    /// on this device. Session-scoped by contract; cleared on disconnect
    /// together with the CCCD state.
    services: Mutex<Option<ServiceSet>>,
    notifications: OnceLock<NotificationHub>,
}

struct HandleSlot {
    peripheral: Option<btleplug::platform::Peripheral>,
    generation: u64,
}

impl DeviceEntry {
    fn new(
        id: DeviceId,
        address: [u8; 6],
        peripheral: btleplug::platform::Peripheral,
        advertisement: AdvertisementData,
    ) -> Self {
        let (generation_sender, _) = watch::channel(1_u64);
        Self {
            id,
            address,
            slot: Mutex::new(HandleSlot {
                peripheral: Some(peripheral),
                generation: 1,
            }),
            generation_sender,
            advertisement: Mutex::new(advertisement),
            connected: AtomicBool::new(false),
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
    /// the fork's unreachable-device errors.
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
            .find(|entry| entry.address == *address)
            .cloned()
    }
}

/// Shared state of the WinRT adapter.
struct AdapterShared {
    adapter_id: AdapterId,
    state: Mutex<AdapterInner>,
    events: EventHub,
    /// The btleplug adapter once `initialize` created it. A tokio mutex
    /// because initialization awaits adapter enumeration while holding it;
    /// the state lock above is never held across an await.
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
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                // Both sightings carry a full property refresh on Windows:
                // the fork mutates the peripheral's property cache from the
                // received advertisement before emitting either event, and
                // the advertisement sub-events below are emitted during that
                // same refresh, so a snapshot taken here already includes
                // manufacturer/service data and RSSI.
                self.record_sighting(adapter, id).await;
            }
            CentralEvent::DeviceConnected(id) => self.record_connection_state(id, true),
            CentralEvent::DeviceDisconnected(id) => self.record_connection_state(id, false),
            CentralEvent::DeviceServicesModified(id) => {
                // Only emitted by the CoreBluetooth subsystem; on Windows the
                // GATT tree changes are observed through rediscovery.
                let _ = id;
            }
            // Advertisement-field refinements folded into the full property
            // refresh of the discovery/update events above (the fork emits
            // them synchronously inside `update_properties`).
            CentralEvent::ManufacturerDataAdvertisement { .. }
            | CentralEvent::ServiceDataAdvertisement { .. }
            | CentralEvent::ServicesAdvertisement { .. }
            | CentralEvent::RssiUpdate { .. } => {}
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
        let device_id = device_id_from(address, properties.address_type);

        let mut inner = self.lock_state();
        if inner.scanning && !matches_filter(&advertisement, inner.filter.as_ref()) {
            return;
        }
        if let Some(entry) = inner.find(&address) {
            entry.set_advertisement(advertisement);
            // The fork rebuilt its internal state for this device (the
            // manager evicts peripherals on disconnect, and a re-sighting
            // creates a fresh object): install the new handle so reconnects
            // and notifications keep working. A connected entry keeps its
            // live handle.
            if !entry.connected.load(Ordering::Acquire) {
                entry.set_handle(peripheral);
            }
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

    fn record_connection_state(&self, id: btleplug::platform::PeripheralId, connected: bool) {
        let address = peripheral_address(&id);
        let inner = self.lock_state();
        let Some(entry) = inner.find(&address) else {
            return;
        };
        // The fork emits duplicate disconnect notifications (one from the
        // reply path, one from the connection-status handler); publish on
        // transitions only.
        if entry.connected.swap(connected, Ordering::AcqRel) != connected {
            let event = if connected {
                AdapterEvent::DeviceConnected(entry.id().clone())
            } else {
                // CCCD subscriptions die with the connection, and the
                // identifiers of the previous session must not resolve
                // against the next one.
                entry.clear_services();
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

/// Extracts the 6-byte address from the fork's `PeripheralId`. On Windows the
/// id wraps the address directly; the conversion runs through the fork's
/// `From<PeripheralId> for BDAddr`.
fn peripheral_address(id: &btleplug::platform::PeripheralId) -> [u8; 6] {
    btleplug::api::BDAddr::from(id.clone()).into_inner()
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

/// The Windows (WinRT) adapter backend. Obtain via
/// [`default_adapter`](crate::default_adapter); all trait methods are safe to
/// call concurrently.
pub(crate) struct WinRtAdapter {
    shared: Arc<AdapterShared>,
}

impl WinRtAdapter {
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
            // The fork collapses Windows radio states other than On/Off
            // (Disabled, NotAvailable) into CentralState::Unknown; the common
            // real-world cause is the user's Bluetooth toggle. A machine
            // without any radio fails earlier, at adapter enumeration.
            AdapterState::PoweredOff | AdapterState::Unknown => {
                Err(Error::Permission(PermissionReason::BluetoothOff))
            }
            _ => Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "adapter",
            }),
        }
    }
}

#[async_trait::async_trait]
impl Adapter for WinRtAdapter {
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
            let state = adapter.adapter_state().await.map_err(map_btleplug_error)?;
            return self.resolve_state(map_central_state(state));
        }
        let manager = btleplug::platform::Manager::new()
            .await
            .map_err(map_btleplug_error)?;
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
                // Applied in the fork's watcher handler, not by the OS radio
                // filter (see matches_filter for the reason).
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
            .map(|entry| Arc::new(WinRtPeripheral::new(entry.clone())) as Arc<dyn Peripheral>)
            .collect())
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        let DeviceId::Mac(address, _) = id else {
            // Windows devices are only ever identified by MAC.
            return Err(Error::NotFound(id.clone()));
        };
        let inner = self.shared.lock_state();
        inner
            .find(address)
            .map(|entry| Arc::new(WinRtPeripheral::new(entry)) as Arc<dyn Peripheral>)
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
