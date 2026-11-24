//! Mock coverage for peripheral advertising and the local GATT server.

use std::time::Duration;

use futures::StreamExt;
use rblue_api::prelude::*;
use rblue_mock::{MockAdapter, LOCAL_ADVERTISEMENT_ID};

const LOCAL_SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_00a1);
const LOCAL_CHAR: uuid::Uuid = uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_00a2);

fn local_service(max_len: Option<usize>) -> LocalService {
    LocalService {
        uuid: LOCAL_SERVICE,
        primary: true,
        characteristics: vec![LocalCharacteristic {
            uuid: LOCAL_CHAR,
            properties: CharacteristicProperties::READ
                | CharacteristicProperties::WRITE
                | CharacteristicProperties::WRITE_WITHOUT_RESPONSE
                | CharacteristicProperties::NOTIFY,
            value: b"initial".to_vec(),
            max_len,
        }],
    }
}

#[tokio::test]
async fn advertising_starts_stops_and_replaces() {
    let adapter = MockAdapter::new();
    adapter.initialize().await.expect("init");
    assert!(!adapter.is_advertising().await.expect("idle"));
    adapter
        .start_advertising(&AdvertiseParams {
            local_name: Some("one".into()),
            services: vec![LOCAL_SERVICE],
            ..AdvertiseParams::default()
        })
        .await
        .expect("start");
    assert!(adapter.is_advertising().await.expect("active"));
    adapter
        .start_advertising(&AdvertiseParams {
            local_name: Some("two".into()),
            services: vec![LOCAL_SERVICE],
            ..AdvertiseParams::default()
        })
        .await
        .expect("replace");
    assert!(adapter.is_advertising().await.expect("still active"));
    adapter.stop_advertising().await.expect("stop");
    assert!(!adapter.is_advertising().await.expect("stopped"));
    assert!(matches!(
        adapter.stop_advertising().await,
        Err(Error::InvalidState(_))
    ));
}

#[tokio::test]
async fn advertising_loopback_is_visible_to_a_running_scan() {
    let adapter = MockAdapter::new();
    adapter.initialize().await.expect("init");
    let mut events = adapter.events();
    adapter
        .start_scan(&ScanFilter::default())
        .await
        .expect("scan");
    adapter
        .start_advertising(&AdvertiseParams {
            local_name: Some("loopback".into()),
            services: vec![LOCAL_SERVICE],
            include_tx_power: true,
            ..AdvertiseParams::default()
        })
        .await
        .expect("advertise");

    let discovered = loop {
        let event = events.next().await.expect("event");
        if let AdapterEvent::DeviceDiscovered(device) = event {
            if device.id == LOCAL_ADVERTISEMENT_ID {
                break device;
            }
        }
    };
    assert_eq!(
        discovered.advertisement.local_name.as_deref(),
        Some("loopback")
    );
    assert_eq!(discovered.advertisement.services, vec![LOCAL_SERVICE]);
    assert_eq!(discovered.advertisement.tx_power, Some(0));
    assert_eq!(discovered.advertisement.is_connectable, Some(true));
    adapter.stop_scan().await.expect("stop scan");
    adapter.stop_advertising().await.expect("stop adv");
}

#[tokio::test]
async fn advertising_timeout_expires_without_an_executor_task() {
    let adapter = MockAdapter::new();
    adapter.initialize().await.expect("init");
    adapter
        .start_advertising(&AdvertiseParams {
            timeout: Some(Duration::from_millis(1)),
            ..AdvertiseParams::default()
        })
        .await
        .expect("start");
    tokio::time::sleep(Duration::from_millis(15)).await;
    assert!(
        !adapter.is_advertising().await.expect("expired"),
        "timeout must be honored the next time advertising state is inspected"
    );
}

#[tokio::test]
async fn local_service_round_trip_and_invalid_length() {
    let adapter = MockAdapter::new();
    let service_id = adapter
        .add_local_service(&local_service(Some(8)))
        .await
        .expect("add");
    let snapshot = adapter.local_services().await.expect("snapshot");
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id, service_id);
    let char_id = snapshot[0].characteristics[0].id;
    adapter
        .write_local_characteristic(&char_id, b"abcd")
        .await
        .expect("write");
    let stored = adapter.local_services().await.expect("after write");
    assert_eq!(stored[0].characteristics[0].value, b"abcd");
    assert!(matches!(
        adapter
            .write_local_characteristic(&char_id, b"012345678")
            .await,
        Err(Error::InvalidLength { len: 9, max: 8 })
    ));
    adapter
        .notify_local_characteristic(&char_id, b"ping")
        .await
        .expect("notify with no subscribers is ok");
    adapter
        .remove_local_service(&service_id)
        .await
        .expect("remove");
    assert!(adapter.local_services().await.expect("empty").is_empty());
}

#[tokio::test]
async fn virtual_central_write_subscribe_and_notify() {
    let adapter = MockAdapter::new();
    let service_id = adapter
        .add_local_service(&local_service(None))
        .await
        .expect("add");
    let char_id = adapter.local_services().await.expect("snapshot")[0].characteristics[0].id;
    let mut server_events = adapter.server_events().expect("server events");

    let central_id = DeviceId::Mac([0x02, 0, 0, 0, 0, 0xAA], AddressType::RandomStatic);
    let central = adapter
        .connect_virtual_central(central_id.clone())
        .expect("connect");
    match server_events.next().await {
        Some(GattServerEvent::CentralConnected { central }) => {
            assert_eq!(central, central_id)
        }
        other => panic!("expected CentralConnected, got {other:?}"),
    }

    assert_eq!(central.read(&char_id).expect("read"), b"initial");
    match server_events.next().await {
        Some(GattServerEvent::CharacteristicRead { characteristic, .. }) => {
            assert_eq!(characteristic, char_id)
        }
        other => panic!("expected CharacteristicRead, got {other:?}"),
    }

    central
        .write(&char_id, b"from-central", WriteMode::WithResponse)
        .expect("write");
    match server_events.next().await {
        Some(GattServerEvent::CharacteristicWrite { value, mode, .. }) => {
            assert_eq!(value, b"from-central");
            assert_eq!(mode, WriteMode::WithResponse);
        }
        other => panic!("expected CharacteristicWrite, got {other:?}"),
    }

    let mut notifications = central.notifications().expect("notification stream");
    central.subscribe(&char_id).expect("subscribe");
    match server_events.next().await {
        Some(GattServerEvent::SubscriptionChanged { subscribed, .. }) => {
            assert!(subscribed)
        }
        other => panic!("expected SubscriptionChanged, got {other:?}"),
    }
    adapter
        .notify_local_characteristic(&char_id, b"hello")
        .await
        .expect("notify");
    let (notified_id, payload) = notifications.next().await.expect("payload");
    assert_eq!(notified_id, char_id);
    assert_eq!(payload, b"hello");

    adapter
        .disconnect_virtual_central(&central_id, Some(DisconnectReason::Local))
        .expect("disconnect");
    match server_events.next().await {
        Some(GattServerEvent::SubscriptionChanged { subscribed, .. }) => {
            assert!(!subscribed)
        }
        other => panic!("expected unsubscribe on disconnect, got {other:?}"),
    }
    match server_events.next().await {
        Some(GattServerEvent::CentralDisconnected { central, reason }) => {
            assert_eq!(central, central_id);
            assert_eq!(reason, Some(DisconnectReason::Local));
        }
        other => panic!("expected CentralDisconnected, got {other:?}"),
    }
    let _ = service_id;
}

#[tokio::test]
async fn notify_without_notify_property_is_invalid_state() {
    let adapter = MockAdapter::new();
    let service = LocalService {
        uuid: LOCAL_SERVICE,
        primary: true,
        characteristics: vec![LocalCharacteristic::new(
            LOCAL_CHAR,
            CharacteristicProperties::READ,
        )],
    };
    let service_id = adapter.add_local_service(&service).await.expect("add");
    let char_id = adapter.local_services().await.expect("snapshot")[0].characteristics[0].id;
    assert!(matches!(
        adapter.notify_local_characteristic(&char_id, b"x").await,
        Err(Error::InvalidState(_))
    ));
    adapter
        .remove_local_service(&service_id)
        .await
        .expect("remove");
}

#[tokio::test]
async fn powering_off_stops_advertising_and_disconnects_centrals() {
    let adapter = MockAdapter::new();
    adapter.initialize().await.expect("init");
    adapter
        .start_advertising(&AdvertiseParams::default())
        .await
        .expect("advertise");
    let _ = adapter
        .add_local_service(&local_service(None))
        .await
        .expect("service");
    let central_id = DeviceId::Mac([0x02, 0, 0, 0, 0, 0xBB], AddressType::RandomStatic);
    adapter
        .connect_virtual_central(central_id.clone())
        .expect("connect");
    adapter.set_powered(false).await.expect("off");
    assert!(!adapter.is_advertising().await.expect("stopped"));
    assert!(matches!(
        adapter.disconnect_virtual_central(&central_id, None),
        Err(Error::InvalidState(_))
    ));
}
