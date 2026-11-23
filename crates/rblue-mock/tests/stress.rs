//! Offline mock stress tests for reconnect storms, notification floods,
//! concurrent GATT writes against [`GattOpQueue`], mid-write
//! [`MockPeripheral::force_disconnect`], and supervisor recovery after
//! [`MockFailure::platform`]` (133)`.
//!
//! These run under `cargo test` with no Bluetooth hardware.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rblue_api::prelude::*;
use rblue_api::queue::{GattOpQueue, GattOperation, QueueConfig};
use rblue_api::supervisor::{ConnectionSupervisor, ReconnectPolicy, SupervisorEvent};
use rblue_mock::{
    MockAdapter, MockCharacteristic, MockFailure, MockOp, MockPeripheral, MockService,
};

const SERVICE_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00c1);
const ECHO_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00c2);
const OTHER_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00c3);

const RECONNECT_ROUNDS: usize = 32;
const NOTIFY_FLOOD: usize = 512;
const CONCURRENT_WRITERS: usize = 32;
const EVENT_TIMEOUT: Duration = Duration::from_secs(2);

fn device_id() -> DeviceId {
    DeviceId::Mac(
        [0x02, 0x00, 0x00, 0x00, 0x28, 0x01],
        AddressType::RandomStatic,
    )
}

fn fixture() -> (Arc<MockAdapter>, Arc<MockPeripheral>, CharacteristicId) {
    let adapter = Arc::new(MockAdapter::new());
    let device = adapter.add_device(device_id(), "stress-device");
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
                .with_max_write_len(512),
            )
            .characteristic(
                MockCharacteristic::new(
                    OTHER_UUID,
                    CharacteristicProperties::READ
                        | CharacteristicProperties::WRITE
                        | CharacteristicProperties::NOTIFY,
                )
                .with_initial_value(b"other".to_vec()),
            ),
    );
    device.set_advertisement(AdvertisementData {
        local_name: Some("stress-device".to_owned()),
        services: vec![SERVICE_UUID],
        rssi: Some(-40),
        is_connectable: Some(true),
        ..AdvertisementData::default()
    });
    let echo = device
        .find_characteristic(ECHO_UUID)
        .expect("echo characteristic");
    (adapter, device, echo)
}

fn fast_policy() -> ReconnectPolicy {
    ReconnectPolicy {
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
        multiplier: 1.0,
        jitter_fraction: 0.0,
        gatt_133_backoff: Duration::from_millis(5),
        breaker_threshold: Some(8),
        max_attempts: None,
        reconnect_on_local_disconnect: false,
        jitter_seed: Some(1),
    }
}

async fn next_supervisor_event(
    events: &mut rblue_api::supervisor::SupervisorEventStream,
) -> SupervisorEvent {
    tokio::time::timeout(EVENT_TIMEOUT, events.next())
        .await
        .expect("supervisor event timed out")
        .expect("supervisor event stream ended")
}

async fn wait_until_write_parked(device: &MockPeripheral) {
    tokio::time::timeout(EVENT_TIMEOUT, async {
        while device.waiting_writes() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("write did not park on the hold");
}

#[tokio::test]
async fn reconnect_storm_restores_gatt_after_repeated_drops() {
    let (_adapter, device, echo) = fixture();
    for _ in 0..RECONNECT_ROUNDS {
        device.connect().await.expect("connect");
        assert!(device.is_connected().await.expect("is_connected"));
        device.force_disconnect(DisconnectReason::Remote);
        assert!(!device.is_connected().await.expect("is_connected"));
        assert!(matches!(
            device
                .write(&echo, b"during-drop", WriteMode::WithResponse)
                .await,
            Err(Error::NotConnected)
        ));
    }

    device.connect().await.expect("final connect");
    device
        .write(&echo, b"after-storm", WriteMode::WithResponse)
        .await
        .expect("write after storm");
    assert_eq!(device.read(&echo).await.expect("read"), b"after-storm");
}

#[tokio::test]
async fn supervisor_reconnect_storm_recovers_each_drop() {
    let (adapter, device, echo) = fixture();
    adapter.initialize().await.expect("initialize");
    device.advertise();

    let (supervisor, handle, mut events) =
        ConnectionSupervisor::new(Arc::clone(&adapter) as Arc<dyn Adapter>, fast_policy());
    let loop_handle = tokio::spawn(supervisor.run());

    handle
        .auto_connect(device_id())
        .await
        .expect("arm supervisor");
    match next_supervisor_event(&mut events).await {
        SupervisorEvent::Connected(id) => assert_eq!(id, device_id()),
        other => panic!("expected Connected, got {other:?}"),
    }

    for _ in 0..RECONNECT_ROUNDS {
        device.force_disconnect(DisconnectReason::Remote);
        match next_supervisor_event(&mut events).await {
            SupervisorEvent::ReconnectScheduled { id, attempt, .. } => {
                assert_eq!(id, device_id());
                assert_eq!(attempt, 1);
            }
            other => panic!("expected ReconnectScheduled, got {other:?}"),
        }
        match next_supervisor_event(&mut events).await {
            SupervisorEvent::Connected(id) => assert_eq!(id, device_id()),
            other => panic!("expected Connected, got {other:?}"),
        }
        assert!(device.is_connected().await.expect("is_connected"));
    }

    device
        .write(&echo, b"supervised", WriteMode::WithResponse)
        .await
        .expect("write after supervised storm");

    handle.shutdown().await;
    let _ = loop_handle.await;
}

#[tokio::test]
async fn notify_flood_is_delivered_in_order_to_every_subscriber() {
    let (_adapter, device, echo) = fixture();
    device.connect().await.expect("connect");
    device.subscribe(&echo).await.expect("subscribe");

    let mut first = device.notifications();
    let mut second = device.notifications();

    for index in 0..NOTIFY_FLOOD {
        let payload = (index as u32).to_be_bytes();
        device
            .push_notify(ECHO_UUID, &payload)
            .expect("echo characteristic exists");
    }

    for index in 0..NOTIFY_FLOOD {
        let expected = (index as u32).to_be_bytes().to_vec();
        let left = first.next().await.expect("first subscriber lost a notify");
        let right = second
            .next()
            .await
            .expect("second subscriber lost a notify");
        assert_eq!(left.id, echo);
        assert_eq!(right.id, echo);
        assert_eq!(left.data, expected);
        assert_eq!(right.data, expected);
    }

    // Unsubscribed characteristics must not leak into these streams.
    device
        .push_notify(OTHER_UUID, b"no-cross-talk")
        .expect("other characteristic exists");
    let leftover = tokio::time::timeout(Duration::from_millis(50), first.next()).await;
    assert!(
        leftover.is_err(),
        "notification for an unsubscribed characteristic leaked to a subscriber"
    );
}

#[tokio::test]
async fn concurrent_direct_writes_store_complete_payloads() {
    let (_adapter, device, echo) = fixture();
    device.connect().await.expect("connect");

    let mut tasks = Vec::with_capacity(CONCURRENT_WRITERS);
    for index in 0..CONCURRENT_WRITERS {
        let writer = Arc::clone(&device);
        let payload = vec![index as u8; 8];
        tasks.push(tokio::spawn(async move {
            writer
                .write(&echo, &payload, WriteMode::WithResponse)
                .await
                .map(|()| payload)
        }));
    }

    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.expect("join").expect("write"));
    }
    let value = device.read(&echo).await.expect("read");
    assert_eq!(value.len(), 8, "payload must not be torn");
    assert!(
        outcomes.iter().any(|payload| payload == &value),
        "final value {value:?} was not one of the concurrent writes"
    );
}

#[tokio::test]
async fn concurrent_queued_writes_run_serially_and_keep_payloads_intact() {
    let (_adapter, device, echo) = fixture();
    device.connect().await.expect("connect");

    let peripheral: Arc<dyn Peripheral> = Arc::clone(&device) as Arc<dyn Peripheral>;
    let (queue, worker) = GattOpQueue::new(peripheral, QueueConfig { capacity: 64 });
    let worker = tokio::spawn(worker.run());

    let mut submissions = Vec::with_capacity(CONCURRENT_WRITERS);
    for index in 0..CONCURRENT_WRITERS {
        let payload = vec![index as u8; 8];
        let submission = queue
            .submit(GattOperation::Write {
                ch: echo,
                data: payload.clone(),
                mode: WriteMode::WithResponse,
            })
            .await
            .expect("submit");
        submissions.push((payload, submission));
    }

    let mut outcomes = Vec::new();
    for (payload, submission) in submissions {
        submission
            .result
            .await
            .expect("result dropped")
            .expect("queued write");
        outcomes.push(payload);
    }
    let value = device.read(&echo).await.expect("read");
    assert_eq!(value.len(), 8, "queued payload must not be torn");
    assert_eq!(
        value,
        *outcomes.last().expect("writers"),
        "serialized queue must apply writes in submission order"
    );

    drop(queue);
    let _ = worker.await;
}

#[tokio::test]
async fn force_disconnect_during_held_write_returns_not_connected() {
    let (_adapter, device, echo) = fixture();
    device.connect().await.expect("connect");
    let long_payload = vec![0xAB; 256];

    device.hold_writes();
    let writer = Arc::clone(&device);
    let payload = long_payload.clone();
    let write =
        tokio::spawn(async move { writer.write(&echo, &payload, WriteMode::WithResponse).await });
    wait_until_write_parked(&device).await;

    device.force_disconnect(DisconnectReason::Remote);
    device.release_writes();
    assert!(matches!(
        write.await.expect("join"),
        Err(Error::NotConnected)
    ));
    assert!(!device.is_connected().await.expect("is_connected"));
}

#[tokio::test]
async fn force_disconnect_during_queued_write_fails_in_flight_and_pending() {
    let (_adapter, device, echo) = fixture();
    device.connect().await.expect("connect");

    let peripheral: Arc<dyn Peripheral> = Arc::clone(&device) as Arc<dyn Peripheral>;
    let (queue, worker) = GattOpQueue::new(peripheral, QueueConfig::default());
    let worker = tokio::spawn(worker.run());

    device.hold_writes();
    let in_flight = queue
        .submit(GattOperation::Write {
            ch: echo,
            data: vec![0x11; 128],
            mode: WriteMode::WithResponse,
        })
        .await
        .expect("submit in-flight");
    wait_until_write_parked(&device).await;

    let pending = queue
        .submit(GattOperation::Write {
            ch: echo,
            data: vec![0x22; 128],
            mode: WriteMode::WithResponse,
        })
        .await
        .expect("submit pending");

    device.force_disconnect(DisconnectReason::Remote);
    device.release_writes();

    assert!(matches!(
        in_flight.result.await.expect("in-flight result dropped"),
        Err(Error::NotConnected)
    ));
    assert!(matches!(
        pending.result.await.expect("pending result dropped"),
        Err(Error::NotConnected)
    ));

    drop(queue);
    let _ = worker.await;
}

#[tokio::test]
async fn injected_133_then_supervisor_recovers() {
    let (adapter, device, echo) = fixture();
    adapter.initialize().await.expect("initialize");
    device.advertise();
    device.set_failure(MockOp::Connect, None, Some(MockFailure::platform(133)));

    let (supervisor, handle, mut events) =
        ConnectionSupervisor::new(Arc::clone(&adapter) as Arc<dyn Adapter>, fast_policy());
    let loop_handle = tokio::spawn(supervisor.run());

    handle
        .auto_connect(device_id())
        .await
        .expect("arm supervisor");

    for expected_attempt in 1..=2u32 {
        match next_supervisor_event(&mut events).await {
            SupervisorEvent::AttemptFailed {
                platform_133,
                attempt,
                next_retry,
                ..
            } => {
                assert!(platform_133, "injected 133 must be classified as such");
                assert_eq!(attempt, expected_attempt);
                assert!(next_retry.is_some());
            }
            other => panic!("expected AttemptFailed, got {other:?}"),
        }
    }

    device.clear_failures();
    match next_supervisor_event(&mut events).await {
        SupervisorEvent::Connected(id) => assert_eq!(id, device_id()),
        other => panic!("expected Connected after clearing 133, got {other:?}"),
    }
    assert!(device.is_connected().await.expect("is_connected"));
    device
        .write(&echo, b"recovered", WriteMode::WithResponse)
        .await
        .expect("write after 133 recovery");

    handle.shutdown().await;
    let _ = loop_handle.await;
}

#[tokio::test]
async fn sticky_133_opens_the_supervisor_breaker() {
    let (adapter, device, _) = fixture();
    adapter.initialize().await.expect("initialize");
    device.advertise();
    device.set_failure(MockOp::Connect, None, Some(MockFailure::platform(133)));

    let mut policy = fast_policy();
    policy.breaker_threshold = Some(2);
    let (supervisor, handle, mut events) =
        ConnectionSupervisor::new(Arc::clone(&adapter) as Arc<dyn Adapter>, policy);
    let loop_handle = tokio::spawn(supervisor.run());

    handle
        .auto_connect(device_id())
        .await
        .expect("arm supervisor");

    match next_supervisor_event(&mut events).await {
        SupervisorEvent::AttemptFailed { attempt, .. } => assert_eq!(attempt, 1),
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
    match next_supervisor_event(&mut events).await {
        SupervisorEvent::AttemptFailed {
            attempt,
            next_retry,
            platform_133,
            ..
        } => {
            assert_eq!(attempt, 2);
            assert!(platform_133);
            assert!(next_retry.is_none(), "breaker open means no retry");
        }
        other => panic!("expected AttemptFailed, got {other:?}"),
    }
    match next_supervisor_event(&mut events).await {
        SupervisorEvent::BreakerOpen {
            consecutive_failures,
            ..
        } => assert_eq!(consecutive_failures, 2),
        other => panic!("expected BreakerOpen, got {other:?}"),
    }

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(
        !device.is_connected().await.expect("is_connected"),
        "breaker must not keep connecting"
    );

    handle.shutdown().await;
    let _ = loop_handle.await;
}
