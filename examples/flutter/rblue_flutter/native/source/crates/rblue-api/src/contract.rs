//! Cross-backend contract test suite for the [`Adapter`]/[`Peripheral`](crate::Peripheral)
//! semantics.
//!
//! The suite asserts *semantics*, not implementation details, and is the
//! quality gate every backend must pass: the mock backend runs it in
//! `cargo test`, real backends run it per backend behind opt-in features or
//! `#[ignore]`d hardware tests. When a behavior is added to the API, extend
//! this suite.
//!
//! Capability-gated methods that exist on the trait surface (`pair`, `rssi`,
//! `mtu`, `request_device`, `system_connected_peripherals`, advertising, and
//! GATT server methods) must return [`Error::Unsupported`] with a non-empty
//! `feature` name when the backend does not advertise the corresponding
//! [`Capability`]. Silent `Ok` no-ops are forbidden.
//!
//! # Fixture contract
//!
//! The adapter handed to [`run_contract_suite`] must expose a connectable
//! peripheral advertising [`CONTRACT_SERVICE_UUID`] containing a
//! characteristic with UUID [`CONTRACT_CHARACTERISTIC_UUID`] that supports
//! `READ | WRITE | NOTIFY` and **echoes** every written value back as a
//! notification (the scripted nRF52840 test peripheral and the mock backend
//! both do this). `max_write_len` must be at most 517, the largest ATT_MTU
//! the Bluetooth spec allows.
//!
//! Assertion failures panic with the offending step name; the returned
//! [`Result`] exists for signature symmetry with the trait surface.

use core::future::Future;
use core::time::Duration;

use std::sync::Arc;

use futures::future::{select, Either};
use futures::stream::StreamExt;
use futures_timer::Delay;

use crate::adapter::AdapterEventStream;
use crate::{
    Adapter, AdapterEvent, AdapterState, AdvertiseParams, Capability, CharacteristicId,
    CharacteristicProperties, ConnectOptions, ConnectionPriority, DeviceId, Error,
    LocalCharacteristic, LocalCharacteristicId, LocalService, LocalServiceId, RequestDeviceOptions,
    ScanFilter, ServiceId, WriteMode, WriteOptions,
};

/// Service UUID every contract fixture must advertise.
///
/// Randomly chosen custom 128-bit UUID (not a Bluetooth SIG listed value).
pub const CONTRACT_SERVICE_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0001);

/// Characteristic UUID of the read/write/notify echo characteristic every
/// contract fixture must expose inside [`CONTRACT_SERVICE_UUID`].
pub const CONTRACT_CHARACTERISTIC_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0002);

/// Service UUID used by the contract suite when exercising the local GATT
/// server (distinct from the fixture's client-discovered service).
const CONTRACT_LOCAL_SERVICE_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0003);

/// Characteristic UUID registered inside [`CONTRACT_LOCAL_SERVICE_UUID`].
const CONTRACT_LOCAL_CHARACTERISTIC_UUID: uuid::Uuid =
    uuid::Uuid::from_u128(0x5b6e_1f2a_9c3d_4e8f_a1b2_c3d4_e5f6_0004);

/// Largest ATT_MTU allowed by the Bluetooth spec (517); sanity bound for
/// reported `max_write_len` values.
const MAX_REASONABLE_WRITE_LEN: usize = 517;
/// Minimum ATT_MTU defined by the Bluetooth spec.
const MIN_ATT_MTU: u16 = 23;
/// Largest ATT_MTU allowed by the Bluetooth spec.
const MAX_ATT_MTU: u16 = 517;

const ECHO_PAYLOAD: &[u8] = b"rblue-contract-echo";

const SCAN_EVENT_TIMEOUT: Duration = Duration::from_secs(30);
const GATT_TIMEOUT: Duration = Duration::from_secs(10);
const DISCONNECT_EVENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the full semantic contract suite against the adapter produced by
/// `make_adapter`.
///
/// # Arguments
///
/// * `make_adapter` - factory for a fresh [`Adapter`]; called exactly once per
///   run so each run starts from a clean backend state.
///
/// # Fixture requirements
///
/// See the [module documentation](self) for the service/characteristic UUIDs
/// and the echo behavior the fixture must provide.
///
/// # Panics
///
/// Panics with a step description when a backend violates the contract. The
/// [`Result`] return value mirrors the trait surface and is always `Ok` on
/// success.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
/// use rblue_api::contract::run_contract_suite;
/// use rblue_api::{Adapter, Error};
///
/// # async fn some_adapter() -> Result<Arc<dyn Adapter>, Error> { todo!() }
/// # async fn demo() {
/// let result = run_contract_suite(|| some_adapter()).await;
/// assert!(result.is_ok());
/// # }
/// ```
pub async fn run_contract_suite<F, Fut>(make_adapter: F) -> Result<(), Error>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<Arc<dyn Adapter>, Error>>,
{
    let adapter = require("make_adapter", make_adapter().await);

    // -- initialize ---------------------------------------------------------
    require("initialize", adapter.initialize().await);
    let caps = adapter.capabilities();
    assert!(
        caps.has(Capability::GattClient),
        "contract violation: fixture backend must report Capability::GattClient"
    );

    // `state()` is always implemented (at least as Unknown). `set_powered`
    // is only exercised here when the capability is absent, so the suite
    // never toggles a real system radio. Backends that advertise
    // AdapterPowerControl cover the switch in their own tests.
    let state = require("state", adapter.state().await);
    let _: AdapterState = state;
    if !adapter.capabilities().has(Capability::AdapterPowerControl) {
        match adapter.set_powered(true).await {
            Err(Error::Unsupported { feature: "set_powered", .. }) => {}
            other => panic!(
                "contract violation at `set_powered` without AdapterPowerControl:                  expected Err(Unsupported {{ feature: \"set_powered\" }}), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
    }

    // The event stream is subscribed before scanning so no discovery event
    // can be missed.
    let mut events = adapter.events();

    // -- scan and discovery ---------------------------------------------------
    require(
        "start_scan",
        adapter.start_scan(&ScanFilter::default()).await,
    );
    let device_id = with_timeout(
        "DeviceDiscovered event",
        SCAN_EVENT_TIMEOUT,
        discover_device(&mut events),
    )
    .await;
    require("stop_scan", adapter.stop_scan().await);

    // Unknown identities must resolve to Error::NotFound, never a hang or a
    // generic failure.
    let bogus_id = unknown_device_id(&device_id);
    match adapter.peripheral(&bogus_id).await {
        Err(Error::NotFound(id)) => assert_eq!(id, bogus_id),
        other => panic!(
            "contract violation at `peripheral(unknown id)`: expected Err(NotFound), got {:?}",
            other.as_ref().map(|_| ())
        ),
    }

    let snapshot = require("peripherals", adapter.peripherals().await);
    assert!(
        snapshot.iter().any(|p| p.id() == &device_id),
        "contract violation: peripherals() snapshot must include the discovered device"
    );

    let peripheral = require("peripheral", adapter.peripheral(&device_id).await);

    // -- pair / unpair -------------------------------------------------------
    // Active pairing can raise a system dialog on real hardware, so the
    // suite only asserts Unsupported when Capability::Pairing is absent.
    // Live success and idempotence are covered by the mock unit tests.
    if adapter.capabilities().has(Capability::Pairing) {
        // skip live pair/unpair
    } else {
        match peripheral.pair().await {
            Err(Error::Unsupported { feature, .. }) => {
                assert_eq!(
                    feature, "pair",
                    "contract violation at `pair`: feature name"
                )
            }
            other => panic!(
                "contract violation at `pair`: expected Err(Unsupported) with feature pair, got {:?}",
                other
            ),
        }
        match peripheral.unpair().await {
            Err(Error::Unsupported { feature, .. }) => {
                assert_eq!(
                    feature, "unpair",
                    "contract violation at `unpair`: feature name"
                )
            }
            other => panic!(
                "contract violation at `unpair`: expected Err(Unsupported) with feature unpair, got {:?}",
                other
            ),
        }
    }

    match peripheral
        .set_connection_priority(ConnectionPriority::Balanced)
        .await
    {
        Err(Error::NotConnected) => {}
        other => panic!(
            "contract violation at `set_connection_priority` before connect: expected Err(NotConnected), got {:?}",
            other
        ),
    }

    // -- system-connected peripherals ---------------------------------------
    // Empty service list is invalid even on backends that do not implement
    // the lookup; a platform gap is reported only after a nonempty filter.
    match adapter.system_connected_peripherals(&[]).await {
        Err(Error::InvalidState(message)) => assert!(
            message.contains("at least one service UUID"),
            "contract violation: empty system_connected_peripherals must mention the required UUID list, got {message:?}"
        ),
        other => panic!(
            "contract violation at `system_connected_peripherals([])`: expected Err(InvalidState), got {:?}",
            other.as_ref().map(|_| ())
        ),
    }
    if adapter
        .capabilities()
        .has(Capability::SystemConnectedPeripherals)
    {
        let connected = require(
            "system_connected_peripherals",
            adapter
                .system_connected_peripherals(&[CONTRACT_SERVICE_UUID])
                .await,
        );
        // The fixture is not necessarily system-connected, so the list may
        // be empty. Identities of any returned handles must be usable.
        for found in &connected {
            let _ = found.id();
        }
    } else {
        match adapter
            .system_connected_peripherals(&[CONTRACT_SERVICE_UUID])
            .await
        {
            Err(Error::Unsupported { feature, .. }) => {
                assert_eq!(feature, "system_connected_peripherals")
            }
            other => panic!(
                "contract violation at `system_connected_peripherals` without the capability: expected Err(Unsupported), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
    }

    let picker_opts = RequestDeviceOptions {
        services: vec![CONTRACT_SERVICE_UUID],
        ..RequestDeviceOptions::default()
    };
    if caps.has(Capability::RequestDevicePicker) {
        let picked =
            require_supported("request_device", adapter.request_device(&picker_opts).await);
        assert_eq!(
            picked.id(),
            &device_id,
            "contract violation: request_device must pick the fixture peripheral advertising CONTRACT_SERVICE_UUID"
        );
    } else {
        require_unsupported("request_device", adapter.request_device(&picker_opts).await);
    }

    // -- connect_with --------------------------------------------------------
    // Default options are equivalent to connect(); non-default options that
    // the backend does not advertise must return Unsupported, never succeed
    // silently.
    require(
        "connect_with default",
        peripheral.connect_with(&ConnectOptions::default()).await,
    );
    let connected = require(
        "is_connected after connect_with default",
        peripheral.is_connected().await,
    );
    assert!(
        connected,
        "contract violation: connect_with(default) must connect"
    );
    require(
        "disconnect after connect_with default",
        peripheral.disconnect().await,
    );

    if adapter.capabilities().has(Capability::AutoConnect) {
        // A live auto_connect=true attempt may wait for the device to appear
        // and is not required here; the capability bit is the advertised
        // contract.
    } else {
        match peripheral
            .connect_with(&ConnectOptions {
                auto_connect: true,
                mtu: None,
            })
            .await
        {
            Err(Error::Unsupported {
                feature: "auto_connect",
                ..
            }) => {}
            other => panic!(
                "contract violation at `connect_with auto_connect`: expected Err(Unsupported {{ feature: auto_connect }}), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
    }

    match peripheral
        .connect_with(&ConnectOptions {
            auto_connect: false,
            mtu: Some(512),
        })
        .await
    {
        Err(Error::Unsupported {
            feature: "request_mtu",
            ..
        }) => {
            if adapter.capabilities().has(Capability::MtuNegotiation) {
                panic!(
                    "contract violation at `connect_with mtu`: MtuNegotiation is set but the request was rejected as Unsupported"
                );
            }
        }
        Ok(()) => {
            if !adapter.capabilities().has(Capability::MtuNegotiation) {
                panic!(
                    "contract violation at `connect_with mtu`: request_mtu must not be silently ignored"
                );
            }
            let negotiated = require("mtu after connect_with", peripheral.mtu().await);
            assert!(
                negotiated >= 23,
                "contract violation: negotiated ATT MTU must be at least 23, got {negotiated}"
            );
            require(
                "disconnect after connect_with mtu",
                peripheral.disconnect().await,
            );
        }
        other => panic!(
            "contract violation at `connect_with mtu`: expected Ok or Err(Unsupported {{ feature: request_mtu }}), got {:?}",
            other.as_ref().map(|_| ())
        ),
    }

    // -- connect -------------------------------------------------------------
    require("connect", peripheral.connect().await);
    let connected = require("is_connected", peripheral.is_connected().await);
    assert!(
        connected,
        "contract violation: is_connected must be true right after connect"
    );

    // -- connection priority ----------------------------------------------------
    if adapter.capabilities().has(Capability::ConnectionPriority) {
        for priority in [
            ConnectionPriority::Low,
            ConnectionPriority::Balanced,
            ConnectionPriority::High,
        ] {
            require(
                "set_connection_priority",
                peripheral.set_connection_priority(priority).await,
            );
        }
    } else {
        match peripheral
            .set_connection_priority(ConnectionPriority::Balanced)
            .await
        {
            Err(Error::Unsupported { feature, .. }) => {
                assert_eq!(feature, "set_connection_priority")
            }
            other => panic!(
                "contract violation at `set_connection_priority` without the capability: expected Err(Unsupported), got {:?}",
                other
            ),
        }
    }

    // -- capability-gated peripheral APIs -------------------------------------
    if caps.has(Capability::Pairing) {
        match peripheral.pair().await {
            Ok(()) | Err(Error::Pairing(_)) => {}
            Err(Error::Unsupported { platform, feature }) => panic!(
                "contract violation at `pair`: backend advertised Capability::Pairing but returned Unsupported {{ platform: {platform}, feature: {feature} }}"
            ),
            Err(err) => panic!(
                "contract violation at `pair`: expected Ok or Err(Pairing), got {err:?}"
            ),
        }
    } else {
        require_unsupported("pair", peripheral.pair().await);
    }

    if caps.has(Capability::RssiDuringConnection) {
        let _rssi = require_supported("rssi", peripheral.rssi().await);
    } else {
        require_unsupported("rssi", peripheral.rssi().await);
    }

    if caps.has(Capability::MtuQuery) {
        let mtu = require_supported("mtu", peripheral.mtu().await);
        assert!(
            (MIN_ATT_MTU..=MAX_ATT_MTU).contains(&mtu),
            "contract violation: mtu {mtu} is outside the ATT_MTU range {MIN_ATT_MTU}..={MAX_ATT_MTU}"
        );
    } else {
        require_unsupported("mtu", peripheral.mtu().await);
    }

    // -- discover services ------------------------------------------------------
    let services = require("discover_services", peripheral.discover_services(&[]).await);
    assert!(
        !services.services.is_empty(),
        "contract violation: discover_services must return a non-empty tree"
    );
    let (service_position, service) = services
        .services
        .iter()
        .enumerate()
        .find(|(_, s)| s.uuid == CONTRACT_SERVICE_UUID)
        .expect("contract violation: fixture must expose CONTRACT_SERVICE_UUID");
    let service_instance = services.services[..service_position]
        .iter()
        .filter(|s| s.uuid == CONTRACT_SERVICE_UUID)
        .count() as u16;
    let characteristic = service
        .characteristics
        .iter()
        .find(|c| c.uuid == CONTRACT_CHARACTERISTIC_UUID)
        .expect("contract violation: fixture must expose CONTRACT_CHARACTERISTIC_UUID");
    let required = CharacteristicProperties::READ
        | CharacteristicProperties::WRITE
        | CharacteristicProperties::NOTIFY;
    assert!(
        characteristic.properties.contains(required),
        "contract violation: echo characteristic must support READ | WRITE | NOTIFY"
    );

    let char_id = CharacteristicId {
        service: ServiceId {
            uuid: service.uuid,
            instance: service_instance,
        },
        uuid: characteristic.uuid,
        instance: 0,
    };

    // -- read and unified write semantics ------------------------------------
    let read_back = require("read", peripheral.read(&char_id).await);
    assert!(
        !read_back.is_empty(),
        "contract violation: fixture read characteristic must return non-empty data"
    );

    for (mode, property) in [
        (WriteMode::WithResponse, CharacteristicProperties::WRITE),
        (
            WriteMode::WithoutResponse,
            CharacteristicProperties::WRITE_WITHOUT_RESPONSE,
        ),
    ] {
        if !characteristic.properties.contains(property) {
            continue;
        }
        let max_len = require(
            "max_write_len",
            peripheral.max_write_len(&char_id, mode).await,
        );
        assert!(
            max_len > 0,
            "contract violation: max_write_len must be positive"
        );
        assert!(
            max_len <= MAX_REASONABLE_WRITE_LEN,
            "contract violation: max_write_len {max_len} exceeds the spec ATT_MTU bound"
        );

        // Oversize data must be rejected with InvalidLength - never truncated,
        // never chunked.
        let oversize = vec![0u8; max_len + 1];
        match peripheral.write(&char_id, &oversize, mode).await {
            Err(Error::InvalidLength { len, max }) => {
                assert_eq!(len, max_len + 1, "InvalidLength must report the data length");
                assert_eq!(max, max_len, "InvalidLength must report max_write_len");
            }
            other => panic!(
                "contract violation at `write oversize ({mode:?})`: expected Err(InvalidLength), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }

        let in_bounds = vec![0u8; max_len];
        require(
            "write (in bounds)",
            peripheral.write(&char_id, &in_bounds, mode).await,
        );

        // Default write_with keeps the unified InvalidLength semantics.
        match peripheral
            .write_with(
                &char_id,
                &oversize,
                mode,
                &WriteOptions {
                    allow_long_write: false,
                },
            )
            .await
        {
            Err(Error::InvalidLength { len, max }) => {
                assert_eq!(len, max_len + 1);
                assert_eq!(max, max_len);
            }
            other => panic!(
                "contract violation at `write_with default oversize ({mode:?})`: expected Err(InvalidLength), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }

        // Long writes are chunked by the current max_write_len; a backend
        // that cannot chunk this WriteMode may return Unsupported.
        match peripheral
            .write_with(
                &char_id,
                &oversize,
                mode,
                &WriteOptions {
                    allow_long_write: true,
                },
            )
            .await
        {
            Ok(()) => {}
            Err(Error::Unsupported {
                feature: "allow_long_write",
                ..
            }) => {}
            other => panic!(
                "contract violation at `write_with allow_long_write ({mode:?})`: expected Ok or Unsupported, got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
    }

    // -- subscribe and notification echo -----------------------------------
    require("subscribe", peripheral.subscribe(&char_id).await);
    let mut notifications = peripheral.notifications();
    let echo_len = ECHO_PAYLOAD.len().min(require(
        "max_write_len (echo)",
        peripheral
            .max_write_len(&char_id, WriteMode::WithResponse)
            .await,
    ));
    let echo_payload = &ECHO_PAYLOAD[..echo_len];
    require(
        "write (echo trigger)",
        peripheral
            .write(&char_id, echo_payload, WriteMode::WithResponse)
            .await,
    );
    let notification = with_timeout("notification echo", GATT_TIMEOUT, notifications.next())
        .await
        .expect("contract violation: notification stream must not end while connected");
    assert_eq!(
        notification.id, char_id,
        "contract violation: notification must carry its characteristic id"
    );
    assert_eq!(
        notification.data, echo_payload,
        "contract violation: fixture must echo written data as a notification"
    );
    require("unsubscribe", peripheral.unsubscribe(&char_id).await);

    // -- disconnect ------------------------------------------------------------
    require("disconnect", peripheral.disconnect().await);
    let connected = require("is_connected", peripheral.is_connected().await);
    assert!(
        !connected,
        "contract violation: is_connected must be false right after disconnect"
    );
    with_timeout(
        "DeviceDisconnected event",
        DISCONNECT_EVENT_TIMEOUT,
        wait_disconnected(&mut events, &device_id),
    )
    .await;

    // -- peripheral advertising and GATT server ------------------------------
    // Run after the central-role fixture flow so loopback advertisements
    // cannot be mistaken for the contract peripheral during scan.
    if caps.has(Capability::Advertise) {
        let advertising =
            require_supported("is_advertising (idle)", adapter.is_advertising().await);
        assert!(
            !advertising,
            "contract violation: is_advertising must be false before start_advertising"
        );
        let params = AdvertiseParams {
            local_name: Some("rblue-contract-adv".to_owned()),
            services: vec![CONTRACT_LOCAL_SERVICE_UUID],
            connectable: false,
            ..AdvertiseParams::default()
        };
        require_supported(
            "start_advertising",
            adapter.start_advertising(&params).await,
        );
        let advertising =
            require_supported("is_advertising (active)", adapter.is_advertising().await);
        assert!(
            advertising,
            "contract violation: is_advertising must be true after start_advertising"
        );
        let replaced = AdvertiseParams {
            local_name: Some("rblue-contract-adv-2".to_owned()),
            services: vec![CONTRACT_LOCAL_SERVICE_UUID],
            connectable: false,
            ..AdvertiseParams::default()
        };
        require_supported(
            "start_advertising (replace)",
            adapter.start_advertising(&replaced).await,
        );
        require_supported("stop_advertising", adapter.stop_advertising().await);
        let advertising =
            require_supported("is_advertising (stopped)", adapter.is_advertising().await);
        assert!(
            !advertising,
            "contract violation: is_advertising must be false after stop_advertising"
        );
        match adapter.stop_advertising().await {
            Err(Error::InvalidState(_)) => {}
            other => panic!(
                "contract violation at `stop_advertising` when idle: expected Err(InvalidState), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
    } else {
        require_unsupported(
            "start_advertising",
            adapter.start_advertising(&AdvertiseParams::default()).await,
        );
        require_unsupported("stop_advertising", adapter.stop_advertising().await);
        require_unsupported("is_advertising", adapter.is_advertising().await);
    }

    if caps.has(Capability::GattServer) {
        let service = LocalService {
            uuid: CONTRACT_LOCAL_SERVICE_UUID,
            primary: true,
            characteristics: vec![LocalCharacteristic {
                uuid: CONTRACT_LOCAL_CHARACTERISTIC_UUID,
                properties: CharacteristicProperties::READ
                    | CharacteristicProperties::WRITE
                    | CharacteristicProperties::NOTIFY,
                value: b"contract-local".to_vec(),
                max_len: Some(20),
            }],
        };
        let service_id = require_supported(
            "add_local_service",
            adapter.add_local_service(&service).await,
        );
        assert_eq!(service_id.uuid, CONTRACT_LOCAL_SERVICE_UUID);
        let snapshot = require_supported("local_services", adapter.local_services().await);
        let registered = snapshot
            .iter()
            .find(|entry| entry.id == service_id)
            .expect("contract violation: local_services must include the service just added");
        assert_eq!(registered.characteristics.len(), 1);
        let char_id = registered.characteristics[0].id;
        assert_eq!(char_id.uuid, CONTRACT_LOCAL_CHARACTERISTIC_UUID);
        require_supported(
            "write_local_characteristic",
            adapter
                .write_local_characteristic(&char_id, b"updated")
                .await,
        );
        let after_write =
            require_supported("local_services after write", adapter.local_services().await);
        let written = after_write
            .iter()
            .find(|entry| entry.id == service_id)
            .expect("service remains")
            .characteristics[0]
            .value
            .as_slice();
        assert_eq!(
            written, b"updated",
            "contract violation: write_local_characteristic must update the stored value"
        );
        require_supported(
            "notify_local_characteristic (no subscribers)",
            adapter.notify_local_characteristic(&char_id, b"ping").await,
        );
        match adapter
            .write_local_characteristic(&char_id, &[0u8; 21])
            .await
        {
            Err(Error::InvalidLength { len, max }) => {
                assert_eq!(len, 21);
                assert_eq!(max, 20);
            }
            other => panic!(
                "contract violation at `write_local_characteristic oversize`: expected Err(InvalidLength), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
        let unknown = LocalCharacteristicId {
            service: LocalServiceId {
                uuid: CONTRACT_LOCAL_SERVICE_UUID,
                instance: 99,
            },
            uuid: CONTRACT_LOCAL_CHARACTERISTIC_UUID,
            instance: 0,
        };
        match adapter.write_local_characteristic(&unknown, b"x").await {
            Err(Error::InvalidState(_)) => {}
            other => panic!(
                "contract violation at `write_local_characteristic` unknown id: expected Err(InvalidState), got {:?}",
                other.as_ref().map(|_| ())
            ),
        }
        let _stream = require_supported("server_events", adapter.server_events());
        require_supported(
            "remove_local_service",
            adapter.remove_local_service(&service_id).await,
        );
        let leftover = require_supported(
            "local_services after remove",
            adapter.local_services().await,
        );
        assert!(
            leftover.iter().all(|entry| entry.id != service_id),
            "contract violation: remove_local_service must drop the service from local_services"
        );
    } else {
        let service = LocalService::new(CONTRACT_LOCAL_SERVICE_UUID);
        require_unsupported(
            "add_local_service",
            adapter.add_local_service(&service).await,
        );
        require_unsupported(
            "remove_local_service",
            adapter
                .remove_local_service(&LocalServiceId {
                    uuid: CONTRACT_LOCAL_SERVICE_UUID,
                    instance: 0,
                })
                .await,
        );
        require_unsupported("local_services", adapter.local_services().await);
        let char_id = LocalCharacteristicId {
            service: LocalServiceId {
                uuid: CONTRACT_LOCAL_SERVICE_UUID,
                instance: 0,
            },
            uuid: CONTRACT_LOCAL_CHARACTERISTIC_UUID,
            instance: 0,
        };
        require_unsupported(
            "write_local_characteristic",
            adapter.write_local_characteristic(&char_id, b"x").await,
        );
        require_unsupported(
            "notify_local_characteristic",
            adapter.notify_local_characteristic(&char_id, b"x").await,
        );
        require_unsupported("server_events", adapter.server_events());
    }

    Ok(())
}

/// Asserts that a call rejected because the backend does not advertise the
/// corresponding [`Capability`] is a non-empty [`Error::Unsupported`], never a
/// silent `Ok`.
fn require_unsupported<T>(step: &str, result: Result<T, Error>) {
    match result {
        Err(Error::Unsupported { feature, .. }) => {
            assert!(
                !feature.is_empty(),
                "contract violation at `{step}`: Unsupported feature must be a non-empty name"
            );
        }
        Ok(_) => panic!(
            "contract violation at `{step}`: capability is absent, expected Err(Unsupported), got Ok (silent no-op is forbidden)"
        ),
        Err(err) => panic!(
            "contract violation at `{step}`: capability is absent, expected Err(Unsupported), got {err:?}"
        ),
    }
}

/// Unwraps a backend result on a path the backend advertised a [`Capability`]
/// for. [`Error::Unsupported`] is a contract violation even if the operation
/// would otherwise be allowed to fail.
fn require_supported<T>(step: &str, result: Result<T, Error>) -> T {
    match result {
        Err(Error::Unsupported { platform, feature }) => panic!(
            "contract violation at `{step}`: backend advertised the capability but returned Unsupported {{ platform: {platform}, feature: {feature} }}"
        ),
        other => require(step, other),
    }
}

/// Unwraps a backend result, panicking with the step name on error.
fn require<T>(step: &str, result: Result<T, Error>) -> T {
    match result {
        Ok(value) => value,
        Err(err) => panic!("contract violation at `{step}`: expected Ok, got {err:?}"),
    }
}

/// Races `fut` against a runtime-agnostic timer; panics on timeout.
async fn with_timeout<T>(label: &str, timeout: Duration, fut: impl Future<Output = T>) -> T {
    match select(Box::pin(fut), Box::pin(Delay::new(timeout))).await {
        Either::Left((value, _)) => value,
        Either::Right(_) => panic!("contract violation at `{label}`: timed out after {timeout:?}"),
    }
}

/// Pulls events until a discovery is seen; everything else is skipped.
async fn discover_device(events: &mut AdapterEventStream) -> DeviceId {
    loop {
        let event = with_timeout(
            "adapter event during scan",
            SCAN_EVENT_TIMEOUT,
            events.next(),
        )
        .await
        .expect("contract violation: adapter event stream must not end during a scan");
        if let AdapterEvent::DeviceDiscovered(discovered) = event {
            return discovered.id;
        }
    }
}

/// Pulls events until the given device reports disconnected.
async fn wait_disconnected(events: &mut AdapterEventStream, device_id: &DeviceId) {
    loop {
        let event = with_timeout(
            "adapter event after disconnect",
            DISCONNECT_EVENT_TIMEOUT,
            events.next(),
        )
        .await
        .expect("contract violation: adapter event stream must not end after disconnect");
        if let AdapterEvent::DeviceDisconnected(id, _) = event {
            if id == *device_id {
                return;
            }
        }
    }
}

/// Builds an identity of the same variant shape as `reference` that is
/// guaranteed to be unknown to the backend.
fn unknown_device_id(reference: &DeviceId) -> DeviceId {
    match reference {
        DeviceId::Mac(_, address_type) => DeviceId::Mac([0xFF; 6], *address_type),
        DeviceId::Uuid(_) => DeviceId::Uuid(uuid::Uuid::nil()),
        DeviceId::Web(_) => DeviceId::Web("rblue-contract/unknown-device".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeAdapter;

    /// Bootstrap self-test of the suite against the in-crate fake backend.
    /// The CI default fixture is the mock integration test in
    /// `tests/contract_mock.rs`.
    #[test]
    fn contract_suite_passes_on_fake_backend() {
        let result = futures::executor::block_on(run_contract_suite(|| async {
            let adapter: Arc<dyn Adapter> = Arc::new(FakeAdapter::new());
            Ok(adapter)
        }));
        result.expect("contract suite must pass on the built-in fake backend");
    }
}
