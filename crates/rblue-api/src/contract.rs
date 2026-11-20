//! Cross-backend contract test suite for the [`Adapter`]/[`Peripheral`]
//! semantics.
//!
//! The suite asserts *semantics*, not implementation details, and is the
//! quality gate every backend must pass: the mock backend runs it in
//! `cargo test`, real backends run it per backend behind opt-in features or
//! `#[ignore]`d hardware tests. When a behavior is added to the API, extend
//! this suite.
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
    Adapter, AdapterEvent, Capability, CharacteristicId, CharacteristicProperties, DeviceId, Error,
    ScanFilter, ServiceId, WriteMode,
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

/// Largest ATT_MTU allowed by the Bluetooth spec (517); sanity bound for
/// reported `max_write_len` values.
const MAX_REASONABLE_WRITE_LEN: usize = 517;

/// Payload written to trigger the notification echo.
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
    assert!(
        adapter.capabilities().has(Capability::GattClient),
        "contract violation: fixture backend must report Capability::GattClient"
    );

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

    // -- connect -------------------------------------------------------------
    require("connect", peripheral.connect().await);
    let connected = require("is_connected", peripheral.is_connected().await);
    assert!(
        connected,
        "contract violation: is_connected must be true right after connect"
    );

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

    Ok(())
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

    #[test]
    fn contract_suite_passes_on_fake_backend() {
        let result = futures::executor::block_on(run_contract_suite(|| async {
            let adapter: Arc<dyn Adapter> = Arc::new(FakeAdapter::new());
            Ok(adapter)
        }));
        result.expect("contract suite must pass on the built-in fake backend");
    }
}
