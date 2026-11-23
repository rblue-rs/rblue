//! Mock [`Adapter`]: a simulated radio environment holding scripted devices.

use std::sync::{Arc, Mutex, MutexGuard};

use futures::channel::mpsc::unbounded;
use futures::stream::StreamExt;

use rblue_api::adapter::{Adapter, AdapterEventStream};
use rblue_api::capability::CapabilitySet;
use rblue_api::device::{AdapterId, AddressType, DeviceId, Rssi};
use rblue_api::error::Error;
use rblue_api::event::{AdapterEvent, AdapterState};
use rblue_api::gatt::CharacteristicProperties;
use rblue_api::peripheral::Peripheral;
use rblue_api::scan::{AdvertisementData, DiscoveredDevice, RequestDeviceOptions, ScanFilter};

use crate::hub::BroadcastHub;
use crate::peripheral::{matches_scan_filter, MockCharacteristic, MockPeripheral, MockService};

/// RSSI of a simulated device that has not advertised an explicit value.
pub(crate) const DEFAULT_RSSI: i16 = -55;

/// ATT MTU of the simulated link: the largest value the Bluetooth spec allows.
pub(crate) const MOCK_ATT_MTU: u16 = 517;

/// Default per-characteristic write bound (`MOCK_ATT_MTU` minus the 3-byte
/// ATT operation header).
pub(crate) const DEFAULT_MAX_WRITE_LEN: usize = (MOCK_ATT_MTU - 3) as usize; // 514

/// Mirrors `rblue_api::CONTRACT_SERVICE_UUID`, which the api crate exports
/// only behind its `contract` feature.
const CONTRACT_SERVICE_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0001);

/// Mirrors `rblue_api::CONTRACT_CHARACTERISTIC_UUID`; see
/// [`CONTRACT_SERVICE_UUID`].
const CONTRACT_CHARACTERISTIC_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0002);

/// Initial value of the contract echo characteristic; the contract requires
/// fixture reads to return non-empty data.
const CONTRACT_INITIAL_VALUE: &[u8] = b"rblue-mock-echo";

/// State shared between the adapter and every simulated peripheral: the event
/// hub plus the environment bookkeeping. Lock order is adapter state, then
/// peripheral state, then the event hub.
#[derive(Debug)]
pub(crate) struct AdapterShared {
    id: AdapterId,
    events: BroadcastHub<AdapterEvent>,
    state: Mutex<AdapterInner>,
}

impl AdapterShared {
    fn lock(&self) -> MutexGuard<'_, AdapterInner> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn publish(&self, event: AdapterEvent) {
        self.events.publish(event);
    }

    /// Records one sighting: applies the active scan filter, marks the device
    /// known on its first report, and publishes the discovery/update event
    /// while still holding the state lock, so the event-stream snapshot can
    /// never duplicate it. Filtered-out sightings are silently dropped.
    pub(crate) fn record_sighting(
        &self,
        id: &DeviceId,
        advertisement: AdvertisementData,
        rssi_fallback: i16,
    ) {
        let mut state = self.lock();
        let matched = match &state.filter {
            Some(filter) => matches_scan_filter(&advertisement, filter),
            None => true,
        };
        if !matched {
            return;
        }
        let rssi = advertisement.rssi.unwrap_or(rssi_fallback);
        let event = if state.mark_known(id) {
            AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                id: id.clone(),
                advertisement,
            })
        } else {
            AdapterEvent::DeviceUpdated(id.clone(), Rssi::new(rssi))
        };
        self.publish(event);
    }
}

/// Environment bookkeeping of one mock adapter.
#[derive(Debug)]
struct AdapterInner {
    initialized: bool,
    /// Current radio/permission state of the simulated adapter.
    radio: AdapterState,
    scanning: bool,
    filter: Option<ScanFilter>,
    /// Every registered device in registration order, visible or not.
    environment: Vec<Arc<MockPeripheral>>,
    /// Devices the adapter has reported at least once (scan history plus
    /// connected devices), in first-sighting order.
    known_order: Vec<DeviceId>,
}

impl AdapterInner {
    /// Marks `id` known on its first sighting; returns `true` when newly added.
    fn mark_known(&mut self, id: &DeviceId) -> bool {
        if self.known_order.iter().any(|known| known == id) {
            return false;
        }
        self.known_order.push(id.clone());
        true
    }

    fn find_in_environment(&self, id: &DeviceId) -> Option<Arc<MockPeripheral>> {
        self.environment
            .iter()
            .find(|device| device.device_id() == id)
            .cloned()
    }
}

/// In-memory mock adapter.
///
/// The mock is the reference backend: it reports every capability bit and
/// implements the full [`Adapter`] surface in memory, runtime-agnostic
/// (futures channels only, no executor dependency).
#[derive(Clone, Debug)]
pub struct MockAdapter {
    shared: Arc<AdapterShared>,
}

impl MockAdapter {
    /// Creates an empty simulated radio environment.
    ///
    /// Devices are added with [`MockAdapter::add_device`]; the fixture for the
    /// `rblue_api::contract` suite is available as
    /// [`MockAdapter::with_contract_fixture`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            shared: Arc::new(AdapterShared {
                id: AdapterId::default_adapter(),
                events: BroadcastHub::default(),
                state: Mutex::new(AdapterInner {
                    initialized: false,
                    radio: AdapterState::Unknown,
                    scanning: false,
                    filter: None,
                    environment: Vec::new(),
                    known_order: Vec::new(),
                }),
            }),
        }
    }

    /// Creates an adapter preloaded with the fixture the
    /// `rblue_api::contract` suite requires: one connectable peripheral
    /// advertising the contract service with a `READ | WRITE | NOTIFY` echo
    /// characteristic whose writes come back as notifications.
    #[must_use]
    pub fn with_contract_fixture() -> Self {
        let adapter = Self::new();
        let device = adapter.add_device(
            DeviceId::Mac(
                [0x02, 0x00, 0x00, 0x00, 0x52, 0x01],
                AddressType::RandomStatic,
            ),
            "rblue-mock-contract",
        );
        device.add_service(
            MockService::new(CONTRACT_SERVICE_UUID).characteristic(
                MockCharacteristic::new(
                    CONTRACT_CHARACTERISTIC_UUID,
                    CharacteristicProperties::READ
                        | CharacteristicProperties::WRITE
                        | CharacteristicProperties::WRITE_WITHOUT_RESPONSE
                        | CharacteristicProperties::NOTIFY,
                )
                .with_initial_value(CONTRACT_INITIAL_VALUE.to_vec())
                .with_echo(),
            ),
        );
        device.set_advertisement(AdvertisementData {
            local_name: Some("rblue-mock-contract".to_owned()),
            services: vec![CONTRACT_SERVICE_UUID],
            rssi: Some(DEFAULT_RSSI),
            is_connectable: Some(true),
            ..AdvertisementData::default()
        });
        adapter
    }

    /// Registers a simulated device with the given identity and advertised
    /// name. The returned handle is the test control plane
    /// ([`MockPeripheral`]).
    pub fn add_device(&self, id: DeviceId, name: &str) -> Arc<MockPeripheral> {
        let peripheral = Arc::new(MockPeripheral::new(id, name, Arc::clone(&self.shared)));
        self.shared.lock().environment.push(Arc::clone(&peripheral));
        peripheral
    }
}

impl Default for MockAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Adapter for MockAdapter {
    fn id(&self) -> &AdapterId {
        &self.shared.id
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::all()
    }

    async fn initialize(&self) -> Result<(), Error> {
        let mut state = self.shared.lock();
        if !state.initialized {
            state.initialized = true;
            state.radio = AdapterState::PoweredOn;
            self.shared
                .publish(AdapterEvent::StateChanged(AdapterState::PoweredOn));
        }
        Ok(())
    }

    async fn state(&self) -> Result<AdapterState, Error> {
        Ok(self.shared.lock().radio)
    }

    async fn set_powered(&self, powered: bool) -> Result<(), Error> {
        let next = if powered {
            AdapterState::PoweredOn
        } else {
            AdapterState::PoweredOff
        };
        let previous = {
            let mut state = self.shared.lock();
            let previous = state.radio;
            state.radio = next;
            previous
        };
        if previous != next {
            self.shared.publish(AdapterEvent::StateChanged(next));
        }
        Ok(())
    }

    async fn start_scan(&self, filter: &ScanFilter) -> Result<(), Error> {
        let visible = {
            let mut state = self.shared.lock();
            state.filter = Some(filter.clone());
            state.scanning = true;
            state
                .environment
                .iter()
                .filter(|device| device.is_visible())
                .cloned()
                .collect::<Vec<_>>()
        };
        for device in &visible {
            device.sight_current_advertisement();
        }
        Ok(())
    }

    async fn stop_scan(&self) -> Result<(), Error> {
        let mut state = self.shared.lock();
        if !state.scanning {
            return Err(Error::InvalidState("no scan is running".to_owned()));
        }
        state.scanning = false;
        Ok(())
    }

    fn events(&self) -> AdapterEventStream {
        let (tx, rx) = unbounded();
        {
            let state = self.shared.lock();
            // Register first, then replay the known-device snapshot: with the
            // shared lock held, no sighting can slip between the two steps.
            self.shared.events.register(tx.clone());
            for id in &state.known_order {
                let Some(device) = state.find_in_environment(id) else {
                    continue;
                };
                let advertisement = device.current_advertisement();
                let _ = tx.unbounded_send(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                    id: id.clone(),
                    advertisement,
                }));
            }
        }
        rx.boxed()
    }

    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        let state = self.shared.lock();
        let mut snapshot: Vec<Arc<dyn Peripheral>> = Vec::with_capacity(state.known_order.len());
        for id in &state.known_order {
            if let Some(device) = state.find_in_environment(id) {
                snapshot.push(device);
            }
        }
        for device in &state.environment {
            let known = state.known_order.iter().any(|id| id == device.device_id());
            if !known && device.is_connected_now() {
                let device = Arc::clone(device);
                snapshot.push(device);
            }
        }
        Ok(snapshot)
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        let state = self.shared.lock();
        let known = state.known_order.iter().any(|known_id| known_id == id);
        if let Some(device) = state.find_in_environment(id) {
            if known || device.is_connected_now() {
                return Ok(device);
            }
        }
        Err(Error::NotFound(id.clone()))
    }

    async fn request_device(
        &self,
        opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error> {
        let picked = {
            let state = self.shared.lock();
            state
                .environment
                .iter()
                .filter(|device| device.is_visible())
                .find(|device| device.matches_request_options(opts))
                .cloned()
        };
        match picked {
            // The simulated picker "sees" the device: emit its sighting so it
            // becomes known and observable on the event stream.
            Some(device) => {
                device.sight_current_advertisement();
                Ok(device)
            }
            None => Err(Error::NotFound(DeviceId::Web(
                "rblue-mock: no visible device matches the request options".to_owned(),
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const SENSOR_SERVICE: uuid::Uuid =
        uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00b1);
    const BEACON_SERVICE: uuid::Uuid =
        uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00b2);
    const UNADVERTISED_SERVICE: uuid::Uuid =
        uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00b3);

    fn mac(id: u8) -> DeviceId {
        DeviceId::Mac(
            [0x02, 0x00, 0x00, 0x00, 0x00, id],
            AddressType::RandomStatic,
        )
    }

    fn adv(rssi: i16) -> AdvertisementData {
        AdvertisementData {
            local_name: Some("scripted".to_owned()),
            services: vec![SENSOR_SERVICE],
            rssi: Some(rssi),
            ..AdvertisementData::default()
        }
    }

    #[test]
    fn capabilities_report_every_bit() {
        let adapter = MockAdapter::new();
        assert_eq!(adapter.capabilities(), CapabilitySet::all());
        assert!(adapter
            .capabilities()
            .has(rblue_api::Capability::AdapterPowerControl));
    }

    #[tokio::test]
    async fn state_is_unknown_until_initialize_powers_on() {
        let adapter = MockAdapter::new();
        assert_eq!(adapter.state().await.expect("state"), AdapterState::Unknown);
        adapter.initialize().await.expect("initialize");
        assert_eq!(
            adapter.state().await.expect("state"),
            AdapterState::PoweredOn
        );
    }

    #[tokio::test]
    async fn set_powered_toggles_state_and_emits_state_changed() {
        let adapter = MockAdapter::new();
        let mut events = adapter.events();
        adapter.initialize().await.expect("initialize");
        match events.next().await {
            Some(AdapterEvent::StateChanged(AdapterState::PoweredOn)) => {}
            other => panic!("expected initialize StateChanged(PoweredOn), got {other:?}"),
        }

        adapter.set_powered(false).await.expect("power off");
        assert_eq!(
            adapter.state().await.expect("state"),
            AdapterState::PoweredOff
        );
        match events.next().await {
            Some(AdapterEvent::StateChanged(AdapterState::PoweredOff)) => {}
            other => panic!("expected StateChanged(PoweredOff), got {other:?}"),
        }

        adapter.set_powered(true).await.expect("power on");
        assert_eq!(
            adapter.state().await.expect("state"),
            AdapterState::PoweredOn
        );
        match events.next().await {
            Some(AdapterEvent::StateChanged(AdapterState::PoweredOn)) => {}
            other => panic!("expected StateChanged(PoweredOn), got {other:?}"),
        }

        // Already in the requested state: success, no extra event.
        adapter.set_powered(true).await.expect("idempotent");
        assert_eq!(
            adapter.state().await.expect("state"),
            AdapterState::PoweredOn
        );
    }

    #[tokio::test]
    async fn stop_scan_without_scan_is_invalid_state() {
        let adapter = MockAdapter::new();
        assert!(matches!(
            adapter.stop_scan().await,
            Err(Error::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn script_publishes_discovered_then_updated_with_rssi_curve() {
        let adapter = MockAdapter::new();
        let device = adapter.add_device(mac(1), "scripted");
        device.set_advertisement_script(vec![adv(-40), adv(-55), adv(-70)]);
        let mut events = adapter.events();

        adapter
            .start_scan(&ScanFilter::default())
            .await
            .expect("scan");
        // The scan snapshot discovers the visible device with its default
        // advertisement...
        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => {
                assert_eq!(discovered.id, mac(1));
                assert_eq!(discovered.advertisement.rssi, Some(DEFAULT_RSSI));
            }
            other => panic!("expected DeviceDiscovered, got {other:?}"),
        }
        // ...and the script yields the RSSI curve as updates.
        for expected in [-40, -55, -70] {
            assert!(device.advance_script());
            match events.next().await {
                Some(AdapterEvent::DeviceUpdated(id, rssi)) => {
                    assert_eq!(id, mac(1));
                    assert_eq!(rssi.as_i16(), expected);
                }
                other => panic!("expected DeviceUpdated, got {other:?}"),
            }
        }
        assert!(!device.advance_script(), "script must be exhausted");

        adapter.stop_scan().await.expect("stop scan");
    }

    #[tokio::test]
    async fn disappear_hides_device_until_it_advertises_again() {
        let adapter = MockAdapter::new();
        let hidden = adapter.add_device(mac(1), "hidden");
        let _visible = adapter.add_device(mac(2), "visible");
        hidden.disappear();

        let mut events = adapter.events();
        adapter
            .start_scan(&ScanFilter::default())
            .await
            .expect("scan");

        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => assert_eq!(discovered.id, mac(2)),
            other => panic!("only the visible device may be discovered, got {other:?}"),
        }
        let snapshot = adapter.peripherals().await.expect("peripherals snapshot");
        assert!(snapshot.iter().any(|p| p.id() == &mac(2)));
        assert!(!snapshot.iter().any(|p| p.id() == &mac(1)));
        assert!(matches!(
            adapter.peripheral(&mac(1)).await,
            Err(Error::NotFound(_))
        ));

        hidden.advertise();
        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => assert_eq!(discovered.id, mac(1)),
            other => panic!("expected re-discovery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn events_snapshot_reports_known_devices() {
        let adapter = MockAdapter::new();
        let device = adapter.add_device(mac(1), "known");
        device.advertise();

        // The device became known before the subscription; the snapshot must
        // still deliver it as a fresh discovery.
        let mut events = adapter.events();
        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => {
                assert_eq!(discovered.id, mac(1))
            }
            other => panic!("expected snapshot discovery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn request_device_picks_matching_device() {
        let adapter = MockAdapter::new();
        let sensor = adapter.add_device(mac(1), "sensor");
        let beacon = adapter.add_device(mac(2), "beacon");
        sensor.set_advertisement(adv(-40));
        beacon.set_advertisement(AdvertisementData {
            services: vec![BEACON_SERVICE],
            ..adv(-40)
        });

        let opts = RequestDeviceOptions {
            services: vec![SENSOR_SERVICE],
            ..RequestDeviceOptions::default()
        };
        let picked = adapter
            .request_device(&opts)
            .await
            .expect("a matching device");
        assert_eq!(picked.id(), &mac(1));

        let other_opts = RequestDeviceOptions {
            services: vec![UNADVERTISED_SERVICE],
            ..RequestDeviceOptions::default()
        };
        assert!(matches!(
            adapter.request_device(&other_opts).await,
            Err(Error::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn scan_filter_is_applied_to_sightings() {
        let adapter = MockAdapter::new();
        let quiet = adapter.add_device(mac(1), "quiet-device");
        let loud = adapter.add_device(mac(2), "loud-device");
        let filter = ScanFilter {
            name_prefix: Some("loud".to_owned()),
            ..ScanFilter::default()
        };

        let mut events = adapter.events();
        adapter.start_scan(&filter).await.expect("scan");
        match events.next().await {
            Some(AdapterEvent::DeviceDiscovered(discovered)) => assert_eq!(discovered.id, mac(2)),
            other => panic!("only the matching device may be discovered, got {other:?}"),
        }
        // Filtered-out sightings leave no scan history.
        assert!(matches!(
            adapter.peripheral(&mac(1)).await,
            Err(Error::NotFound(_))
        ));

        // Sightings of non-matching devices are suppressed while the filter
        // is active.
        quiet.set_rssi(-40);
        let quiet_update = tokio::time::timeout(Duration::from_millis(50), events.next()).await;
        assert!(
            quiet_update.is_err(),
            "filtered-out devices must not emit sightings"
        );

        // A matching sighting flows through as an update.
        loud.set_rssi(-60);
        match events.next().await {
            Some(AdapterEvent::DeviceUpdated(id, rssi)) => {
                assert_eq!(id, mac(2));
                assert_eq!(rssi.as_i16(), -60);
            }
            other => panic!("expected DeviceUpdated, got {other:?}"),
        }
    }
}
