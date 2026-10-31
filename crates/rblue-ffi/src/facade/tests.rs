//! Facade unit tests against the mock backend (no Bluetooth hardware).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rblue::platform::mock::MockAdapter;
use rblue::prelude::{AddressType, AdvertisementData, DeviceId};
use rblue_api::contract::{CONTRACT_CHARACTERISTIC_UUID, CONTRACT_SERVICE_UUID};
use rblue_api::gatt::{CharacteristicId, ServiceId};
use rblue_api::Error as ApiError;

use super::{EventSink, Rblue, RblueError};

/// How long a test waits for the pump to deliver before failing.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(5);

/// A shared recording of what the pump delivered to the sink.
#[derive(Default)]
struct Recording {
    events: Mutex<Vec<String>>,
    notifications: Mutex<Vec<String>>,
}

impl Recording {
    fn events(&self) -> Vec<String> {
        self.events.lock().expect("recording").clone()
    }

    fn notifications(&self) -> Vec<String> {
        self.notifications.lock().expect("recording").clone()
    }
}

/// `Box<dyn EventSink>` is consumed by `set_event_sink`; the handle keeps a
/// cloneable view on what was delivered.
#[derive(Clone)]
struct SinkHandle(Arc<Recording>);

impl EventSink for SinkHandle {
    fn on_event(&self, event_json: String) {
        self.0.events.lock().expect("recording").push(event_json);
    }

    fn on_notification(&self, notification_json: String) {
        self.0
            .notifications
            .lock()
            .expect("recording")
            .push(notification_json);
    }
}

/// Waits until `done` holds, polling the pump's delivery from the test task.
async fn wait_until(done: impl Fn() -> bool, what: &str) {
    let wait = async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(DELIVERY_TIMEOUT, wait)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

/// Installs a recording sink and scans until the fixture device is known to
/// the adapter (the mock only resolves peripherals it has observed).
async fn scan_fixture_into_view(rblue: &Rblue, recording: &Arc<Recording>) {
    rblue.set_event_sink(Box::new(SinkHandle(Arc::clone(recording))));
    rblue.start_scan("{}".to_owned()).await.expect("start scan");
    wait_until(
        || {
            recording
                .events()
                .iter()
                .any(|event| event.contains("device_discovered"))
        },
        "device_discovered event",
    )
    .await;
    rblue.stop_scan().await.expect("stop scan");
}

/// Scans until the fixture device is known to the adapter, without any sink
/// installed (the mock only resolves peripherals it has observed).
async fn scan_fixture_into_view_sinkless(rblue: &Rblue) {
    rblue.start_scan("{}".to_owned()).await.expect("start scan");
    let device_json = fixture_device_json();
    let wait = async {
        while rblue.is_connected(device_json.clone()).await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(DELIVERY_TIMEOUT, wait)
        .await
        .expect("timed out waiting for the fixture device to become known");
    rblue.stop_scan().await.expect("stop scan");
}

/// The fixture device id, serialized the way facade arguments expect it.
fn fixture_device_json() -> String {
    serde_json::to_string(&DeviceId::Mac(
        [0x02, 0x00, 0x00, 0x00, 0x52, 0x01],
        AddressType::RandomStatic,
    ))
    .expect("device JSON")
}

/// The contract echo characteristic id, serialized the same way.
fn fixture_characteristic_json() -> String {
    serde_json::to_string(&CharacteristicId {
        service: ServiceId {
            uuid: CONTRACT_SERVICE_UUID,
            instance: 0,
        },
        uuid: CONTRACT_CHARACTERISTIC_UUID,
        instance: 0,
    })
    .expect("characteristic JSON")
}

#[test]
fn platform_codes_survive_the_error_mapping() {
    let mapped: RblueError = ApiError::Platform {
        code: 133,
        message: "GATT_ERROR".to_owned(),
    }
    .into();
    assert_eq!(
        mapped,
        RblueError::Platform {
            code: 133,
            message: "GATT_ERROR".to_owned()
        }
    );
}

#[test]
fn structured_nested_errors_survive_the_error_mapping() {
    let mapped: RblueError = ApiError::Pairing(rblue_api::PairingStatus::Other(0x0E)).into();
    assert_eq!(
        mapped,
        RblueError::Pairing {
            status: crate::PairingStatus::Other { code: 0x0E }
        }
    );
}

#[tokio::test]
async fn malformed_json_arguments_never_reach_the_backend() {
    let rblue = Rblue::with_adapter(Arc::new(MockAdapter::new()));
    assert!(matches!(
        rblue.start_scan("not json".to_owned()).await,
        Err(RblueError::InvalidArgument { .. })
    ));
    assert!(matches!(
        rblue.connect(fixture_device_json()).await,
        Err(RblueError::NotFound { .. })
    ));
}

#[tokio::test]
async fn discoveries_reach_the_installed_sink() {
    let mock = MockAdapter::new();
    let device = mock.add_device(
        DeviceId::Mac(
            [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
            AddressType::RandomStatic,
        ),
        "simulated-sensor",
    );
    device.set_advertisement_script(vec![AdvertisementData {
        local_name: Some("simulated-sensor".to_owned()),
        ..AdvertisementData::default()
    }]);

    let recording = Arc::new(Recording::default());
    let rblue = Rblue::with_adapter(Arc::new(mock));
    rblue.initialize().await.expect("initialize");
    scan_fixture_into_view(&rblue, &recording).await;
}

#[tokio::test]
async fn gatt_roundtrip_read_write_and_len_through_the_facade() {
    let rblue = Rblue::with_adapter(Arc::new(MockAdapter::with_contract_fixture()));
    rblue.initialize().await.expect("initialize");
    scan_fixture_into_view(&rblue, &Arc::new(Recording::default())).await;
    rblue.connect(fixture_device_json()).await.expect("connect");

    let device_json = fixture_device_json();
    let characteristic_json = fixture_characteristic_json();
    rblue
        .discover_services(device_json.clone(), None)
        .await
        .expect("discover");

    rblue
        .write(
            device_json.clone(),
            characteristic_json.clone(),
            b"ping".to_vec(),
            true,
        )
        .await
        .expect("write");
    let read = rblue
        .read(device_json.clone(), characteristic_json.clone())
        .await
        .expect("read");
    assert_eq!(read, b"ping".to_vec());

    let max_write_len = rblue
        .max_write_len(device_json.clone(), characteristic_json.clone(), true)
        .await
        .expect("max write len");
    assert!(max_write_len > 0);

    // Unified write semantics: oversize payloads are rejected, not truncated.
    let oversize = vec![0u8; (max_write_len + 1) as usize];
    assert!(matches!(
        rblue
            .write(device_json, characteristic_json, oversize, true)
            .await,
        Err(RblueError::InvalidLength { .. })
    ));
}

#[tokio::test]
async fn subscriptions_made_before_the_sink_are_forwarded() {
    let rblue = Rblue::with_adapter(Arc::new(MockAdapter::with_contract_fixture()));
    rblue.initialize().await.expect("initialize");

    // No sink yet: the adapter learns the device through a scan whose events
    // nobody receives.
    scan_fixture_into_view_sinkless(&rblue).await;
    rblue.connect(fixture_device_json()).await.expect("connect");
    rblue
        .subscribe(fixture_device_json(), fixture_characteristic_json())
        .await
        .expect("subscribe");

    // The sink arrives late; the forwarder must attach on install.
    let recording = Arc::new(Recording::default());
    rblue.set_event_sink(Box::new(SinkHandle(Arc::clone(&recording))));

    rblue
        .write(
            fixture_device_json(),
            fixture_characteristic_json(),
            b"ping".to_vec(),
            true,
        )
        .await
        .expect("write");

    wait_until(
        || {
            recording
                .notifications()
                .iter()
                .any(|notification| notification.contains("[112,105,110,103]"))
        },
        "echo notification",
    )
    .await;
}

#[tokio::test]
async fn disconnect_is_observed_through_the_sink() {
    let rblue = Rblue::with_adapter(Arc::new(MockAdapter::with_contract_fixture()));
    rblue.initialize().await.expect("initialize");

    let recording = Arc::new(Recording::default());
    scan_fixture_into_view(&rblue, &recording).await;
    rblue.connect(fixture_device_json()).await.expect("connect");
    rblue
        .subscribe(fixture_device_json(), fixture_characteristic_json())
        .await
        .expect("subscribe");
    rblue
        .disconnect(fixture_device_json())
        .await
        .expect("disconnect");

    wait_until(
        || {
            recording
                .events()
                .iter()
                .any(|event| event.contains("device_disconnected"))
        },
        "device_disconnected event",
    )
    .await;
}
