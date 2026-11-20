//! Request dispatch: a method name and JSON parameters resolved against the
//! facade's adapter.
//!
//! Method arguments reuse the serde shapes of the `rblue-api` types
//! (`ScanFilter`, `DeviceId`, `CharacteristicId`, `WriteMode`, ...), the same
//! JSON contracts the rblue-ffi facade uses. Malformed arguments fail with
//! [`RequestFailure::InvalidArgument`] before touching the backend.

use std::ffi::CString;

use rblue::prelude::*;
use serde::de::DeserializeOwned;

use crate::handle::CoreInner;

/// One dispatched request. The fields were copied out of caller memory by the
/// ABI entry point, so ownership moves freely through the runtime.
pub(crate) struct Request {
    /// Method selector, e.g. `"start_scan"`.
    pub(crate) method: String,
    /// JSON object with the method's parameters; `"{}"` when none.
    pub(crate) params: String,
    /// Binary payload (write data); empty when the method takes none.
    pub(crate) payload: Vec<u8>,
}

/// Why a dispatched request failed.
#[derive(Debug)]
pub(crate) enum RequestFailure {
    /// A backend error, serialized exactly as `rblue_api::Error` JSON
    /// (`Platform { code }` raw codes preserved verbatim).
    Backend(Error),
    /// A facade-level argument error: the method is unknown or the parameter
    /// JSON does not parse. Never reaches the backend. Serialized as
    /// `{"InvalidArgument":{"message":...}}`, extending the serde-default
    /// tagging of `rblue_api::Error` the way the rblue-ffi facade extends it
    /// with its facade-only variant.
    InvalidArgument(String),
}

impl From<Error> for RequestFailure {
    fn from(error: Error) -> Self {
        Self::Backend(error)
    }
}

impl RequestFailure {
    /// The `status_json` string the response callback receives; success is
    /// encoded as a null pointer instead.
    pub(crate) fn to_status_cstring(&self) -> CString {
        let json = match self {
            Self::Backend(error) => serde_json::to_string(error)
                .expect("rblue_api::Error JSON serialization cannot fail"),
            Self::InvalidArgument(message) => {
                serde_json::json!({ "InvalidArgument": { "message": message } }).to_string()
            }
        };
        CString::new(json).expect("JSON strings never contain NUL bytes")
    }
}

/// Device-addressing parameters (`{"device": <DeviceId>}`).
#[derive(serde::Deserialize)]
struct DeviceParams {
    /// The device to operate on.
    device: DeviceId,
}

/// Characteristic-addressing parameters
/// (`{"device": <DeviceId>, "characteristic": <CharacteristicId>}`).
#[derive(serde::Deserialize)]
struct CharacteristicParams {
    /// The connected device.
    device: DeviceId,
    /// The characteristic, from the `discover_services` result.
    characteristic: CharacteristicId,
}

/// Connection-priority parameters
/// (`{"device": <DeviceId>, "priority": "low"|"balanced"|"high"}`).
#[derive(serde::Deserialize)]
struct ConnectionPriorityParams {
    /// The connected device.
    device: DeviceId,
    /// Requested connection trade-off.
    priority: ConnectionPriority,
}

/// Write parameters: like [`CharacteristicParams`] plus the write mode.
#[derive(serde::Deserialize)]
struct WriteParams {
    /// The connected device.
    device: DeviceId,
    /// The characteristic, from the `discover_services` result.
    characteristic: CharacteristicId,
    /// Acknowledged (`with_response`) or best-effort write; defaults to
    /// `with_response`.
    #[serde(default)]
    mode: WriteMode,
}

/// Summary of one known device, the result shape of the `peripherals` method.
#[derive(serde::Serialize)]
struct PeripheralInfo {
    /// Stable-per-session identifier, reusable as a `DeviceId` argument.
    id: DeviceId,
    /// Advertised name, when known.
    name: Option<String>,
}

/// Dispatches one request against the facade core.
///
/// # Errors
///
/// [`RequestFailure::InvalidArgument`] for unknown methods and malformed
/// parameter JSON (nothing reached the backend), [`RequestFailure::Backend`]
/// for everything the adapter raised.
pub(crate) async fn execute(
    core: &CoreInner,
    Request {
        method,
        params,
        payload,
    }: Request,
) -> Result<Vec<u8>, RequestFailure> {
    match method.as_str() {
        "backend_version" => Ok(json_payload(rblue::platform::backend_version())),
        "capabilities" => Ok(json_payload(&core.adapter().capabilities().bits())),
        "initialize" => {
            core.adapter().initialize().await?;
            Ok(Vec::new())
        }
        "start_scan" => {
            let filter: ScanFilter = parse_with_defaults(&params, "ScanFilter")?;
            core.adapter().start_scan(&filter).await?;
            Ok(Vec::new())
        }
        "stop_scan" => {
            core.adapter().stop_scan().await?;
            Ok(Vec::new())
        }
        "peripherals" | "system_connected_peripherals" => {
            let adapter = core.adapter();
            let peripherals = if method == "system_connected_peripherals" {
                let filter: ScanFilter = parse_with_defaults(&params, "ScanFilter")?;
                if filter.services.is_empty() {
                    return Err(RequestFailure::InvalidArgument(
                        "at least one service UUID is required".into(),
                    ));
                }
                adapter
                    .system_connected_peripherals(&filter.services)
                    .await?
            } else {
                adapter.peripherals().await?
            };
            let infos: Vec<PeripheralInfo> = peripherals
                .iter()
                .map(|peripheral| PeripheralInfo {
                    id: peripheral.id().clone(),
                    name: peripheral.name(),
                })
                .collect();
            Ok(json_payload(&infos))
        }
        "connect" => {
            let DeviceParams { device } = parse_params(&params)?;
            core.peripheral(&device).await?.connect().await?;
            Ok(Vec::new())
        }
        "disconnect" => {
            let DeviceParams { device } = parse_params(&params)?;
            let peripheral = core.peripheral(&device).await?;
            let result = peripheral.disconnect().await;
            core.drop_device_subscriptions(&device);
            result?;
            Ok(Vec::new())
        }
        "discover_services" => {
            let DeviceParams { device } = parse_params(&params)?;
            let services: ServiceSet = core
                .peripheral(&device)
                .await?
                .discover_services(&[])
                .await?;
            Ok(json_payload(&services))
        }
        "read" => {
            let CharacteristicParams {
                device,
                characteristic,
            } = parse_params(&params)?;
            Ok(core
                .peripheral(&device)
                .await?
                .read(&characteristic)
                .await?)
        }
        "write" => {
            let WriteParams {
                device,
                characteristic,
                mode,
            } = parse_params(&params)?;
            core.peripheral(&device)
                .await?
                .write(&characteristic, &payload, mode)
                .await?;
            Ok(Vec::new())
        }
        "subscribe" => {
            let CharacteristicParams {
                device,
                characteristic,
            } = parse_params(&params)?;
            let peripheral = core.peripheral(&device).await?;
            peripheral.subscribe(&characteristic).await?;
            core.attach_subscriber(&device, &characteristic, peripheral)
                .await;
            Ok(Vec::new())
        }
        "unsubscribe" => {
            let CharacteristicParams {
                device,
                characteristic,
            } = parse_params(&params)?;
            let peripheral = core.peripheral(&device).await?;
            let result = peripheral.unsubscribe(&characteristic).await;
            core.detach_subscriber(&device, &characteristic);
            result?;
            Ok(Vec::new())
        }
        "rssi" | "mtu" => {
            let DeviceParams { device } = parse_params(&params)?;
            let peripheral = core.peripheral(&device).await?;
            if method == "rssi" {
                Ok(json_payload(&peripheral.rssi().await?))
            } else {
                Ok(json_payload(&peripheral.mtu().await?))
            }
        }
        "max_write_len" => {
            let WriteParams {
                device,
                characteristic,
                mode,
            } = parse_params(&params)?;
            Ok(json_payload(
                &core
                    .peripheral(&device)
                    .await?
                    .max_write_len(&characteristic, mode)
                    .await?,
            ))
        }
        "is_connected" => {
            let DeviceParams { device } = parse_params(&params)?;
            let connected = core.peripheral(&device).await?.is_connected().await?;
            Ok(json_payload(&connected))
        }
        "set_connection_priority" => {
            let ConnectionPriorityParams { device, priority } = parse_params(&params)?;
            core.peripheral(&device)
                .await?
                .set_connection_priority(priority)
                .await?;
            Ok(Vec::new())
        }
        unknown => Err(RequestFailure::InvalidArgument(format!(
            "unknown method {unknown:?}"
        ))),
    }
}

/// Parses a JSON object parameter.
///
/// # Errors
///
/// [`RequestFailure::InvalidArgument`] naming `what` when `json` does not
/// deserialize into `T`.
fn parse_params<T: DeserializeOwned>(json: &str) -> Result<T, RequestFailure> {
    serde_json::from_str(json)
        .map_err(|error| RequestFailure::InvalidArgument(format!("invalid params: {error}")))
}

/// Parses a JSON object parameter with defaults, overlaying the given object
/// on the serialized `T::default()` so omitted fields keep their default
/// (`{}` is the zero-value request, e.g. the unfiltered `ScanFilter`).
///
/// # Errors
///
/// [`RequestFailure::InvalidArgument`] naming `what` when `json` does not
/// deserialize into `T`.
fn parse_with_defaults<T>(json: &str, what: &str) -> Result<T, RequestFailure>
where
    T: DeserializeOwned + serde::Serialize + Default,
{
    let value: serde_json::Value = parse_params(json)?;
    let defaults =
        serde_json::to_value(T::default()).expect("default request JSON serialization cannot fail");
    let merged = match (defaults, value) {
        (serde_json::Value::Object(mut base), serde_json::Value::Object(overrides)) => {
            for (key, value) in overrides {
                base.insert(key, value);
            }
            serde_json::Value::Object(base)
        }
        (_, value) => value,
    };
    serde_json::from_value(merged)
        .map_err(|error| RequestFailure::InvalidArgument(format!("invalid {what}: {error}")))
}

/// Serializes a result value into the response payload. JSON serialization of
/// the result model cannot fail.
fn json_payload<T: serde::Serialize + ?Sized>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("result JSON serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::StreamExt;
    use rblue_mock::{MockFailure, MockOp};

    use crate::testing::{characteristic_params, device_id, device_params, Fixture};

    use super::{Request, *};

    /// Sends a request and awaits its response.
    async fn roundtrip(
        fixture: &mut Fixture,
        req_id: u64,
        method: &str,
        params: &str,
        payload: &[u8],
    ) -> (Option<String>, Vec<u8>) {
        fixture.request(req_id, method, params, payload);
        let (got_id, status, payload) = fixture.recorder.next_response().await;
        assert_eq!(got_id, req_id, "response must carry the request id");
        (status, payload)
    }

    #[tokio::test]
    async fn unknown_method_reports_invalid_argument() {
        let mut fixture = Fixture::new().await;
        let (status, payload) = roundtrip(&mut fixture, 1, "teleport", "{}", &[]).await;
        let status = status.expect("unknown methods must fail");
        assert!(status.contains("InvalidArgument"), "{status}");
        assert!(status.contains("teleport"), "{status}");
        assert!(payload.is_empty());
    }

    #[tokio::test]
    async fn malformed_params_never_reach_the_backend() {
        let mut fixture = Fixture::new().await;
        let (status, _) = roundtrip(&mut fixture, 1, "connect", "{not json", &[]).await;
        let status = status.expect("malformed params must fail");
        assert!(status.contains("InvalidArgument"), "{status}");
    }

    #[tokio::test]
    async fn backend_version_and_initialize_round_trip() {
        let mut fixture = Fixture::new().await;
        let (status, payload) = roundtrip(&mut fixture, 7, "backend_version", "{}", &[]).await;
        assert!(status.is_none());
        assert_eq!(
            String::from_utf8(payload).expect("payload utf-8"),
            "\"mock/in-memory\""
        );
        let (status, payload) = roundtrip(&mut fixture, 8, "initialize", "{}", &[]).await;
        assert!(status.is_none());
        assert!(payload.is_empty());
    }

    #[tokio::test]
    async fn start_scan_defaults_overlay_and_update_events() {
        let mut fixture = Fixture::new().await;
        fixture
            .device
            .set_advertisement_script(vec![rblue_api::AdvertisementData {
                rssi: Some(-50),
                ..rblue_api::AdvertisementData::default()
            }]);
        // `{}` overlays on the serialized default: the unfiltered scan filter.
        let (status, _) = roundtrip(&mut fixture, 1, "start_scan", "{}", &[]).await;
        assert!(status.is_none());
        assert!(fixture.device.advance_script());
        // `start_scan` re-snapshots the visible device first; wait through
        // discoveries until the scripted update arrives.
        // Scan re-sightings and intermediate updates pass through; wait for
        // the scripted entry, which is the update carrying the new RSSI.
        let scripted_update = loop {
            let candidate = fixture.recorder.next_event().await;
            assert!(
                candidate.contains("\"type\":\"adapter_event\""),
                "envelope must be tagged: {candidate}"
            );
            if candidate.contains("device_updated") && candidate.contains("-50") {
                break candidate;
            }
        };
        assert!(scripted_update.contains("device_updated"));
    }

    #[tokio::test]
    async fn metrics_are_returned_as_json_and_require_a_connection() {
        let mut fixture = Fixture::new().await;
        let (status, _) = roundtrip(&mut fixture, 1, "rssi", &device_params(), &[]).await;
        assert!(status.is_some());
        let (status, capabilities) = roundtrip(&mut fixture, 2, "capabilities", "{}", &[]).await;
        assert!(status.is_none());
        assert!(serde_json::from_slice::<u32>(&capabilities).unwrap() > 0);
        roundtrip(&mut fixture, 3, "connect", &device_params(), &[]).await;
        for (id, method, params) in [
            (4, "rssi", device_params()),
            (5, "mtu", device_params()),
            (6, "max_write_len", characteristic_params()),
        ] {
            let (status, value) = roundtrip(&mut fixture, id, method, &params, &[]).await;
            assert!(status.is_none(), "{method}: {status:?}");
            assert!(serde_json::from_slice::<i64>(&value).is_ok());
        }
    }

    #[tokio::test]
    async fn discover_read_write_flow() {
        let mut fixture = Fixture::new().await;
        let (status, _) = roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        assert!(status.is_none());
        let (status, payload) =
            roundtrip(&mut fixture, 2, "discover_services", &device_params(), &[]).await;
        assert!(status.is_none());
        let services: rblue_api::ServiceSet =
            serde_json::from_slice(&payload).expect("ServiceSet JSON");
        let service = services.services.first().expect("fixture service");
        assert_eq!(service.uuid, crate::testing::SERVICE_UUID);
        assert!(service.primary);
        let characteristic = service
            .characteristics
            .first()
            .expect("fixture characteristic");
        assert_eq!(characteristic.uuid, crate::testing::CHAR_UUID);
        assert!(characteristic
            .properties
            .contains(rblue_api::CharacteristicProperties::READ));
        // The discovered id must equal the canonical shape the client uses.
        assert_eq!(
            characteristic,
            &rblue_api::Characteristic {
                uuid: crate::testing::CHAR_UUID,
                properties: crate::testing::CHAR_PROPERTIES,
                descriptors: Vec::new(),
            }
        );

        let (status, payload) =
            roundtrip(&mut fixture, 3, "read", &characteristic_params(), &[]).await;
        assert!(status.is_none());
        assert_eq!(payload, vec![1, 2, 3], "read returns the raw bytes");

        let (status, payload) =
            roundtrip(&mut fixture, 4, "write", &characteristic_params(), b"ping").await;
        assert!(status.is_none());
        assert!(payload.is_empty());
    }

    #[tokio::test]
    async fn service_set_json_shape_is_locked_for_binding_consumers() {
        // The Dart binding parses this JSON without a Rust round trip, so the
        // exact shapes (uuid text form, bitflags "|"-separated names) are part
        // of the contract.
        let mut fixture = Fixture::new().await;
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        let (status, payload) =
            roundtrip(&mut fixture, 2, "discover_services", &device_params(), &[]).await;
        assert!(status.is_none());
        let json = String::from_utf8(payload).expect("payload utf-8");
        assert_eq!(
            json,
            "{\"services\":[{\"uuid\":\"00000000-0000-0000-0000-00000000180f\",\"primary\":true,\"characteristics\":[{\"uuid\":\"00000000-0000-0000-0000-000000002a19\",\"properties\":\"READ | WRITE_WITHOUT_RESPONSE | WRITE | NOTIFY | INDICATE\",\"descriptors\":[]}]}]}"
        );
    }

    #[tokio::test]
    async fn oversize_write_reports_invalid_length() {
        let mut fixture = Fixture::new().await;
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        let oversize = vec![0u8; 600];
        let (status, _) = roundtrip(
            &mut fixture,
            2,
            "write",
            &characteristic_params(),
            &oversize,
        )
        .await;
        let status = status.expect("oversize writes must fail");
        let value: serde_json::Value = serde_json::from_str(&status).expect("status JSON");
        assert_eq!(value["InvalidLength"]["len"], 600, "{status}");
        assert_eq!(value["InvalidLength"]["max"], 514, "{status}");
    }

    #[tokio::test]
    async fn platform_codes_are_preserved_verbatim() {
        let mut fixture = Fixture::new().await;
        fixture
            .device
            .set_failure(MockOp::Write, None, Some(MockFailure::platform(133)));
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        let (status, _) = roundtrip(&mut fixture, 2, "write", &characteristic_params(), b"x").await;
        let status = status.expect("injected failure must surface");
        let value: serde_json::Value = serde_json::from_str(&status).expect("status JSON");
        assert_eq!(value["Platform"]["code"], 133, "{status}");
        assert_eq!(value["Platform"]["message"], "injected mock failure");
    }

    #[tokio::test]
    async fn notifications_route_and_unsubscribe_stops_them() {
        let mut fixture = Fixture::new().await;
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        let (status, _) =
            roundtrip(&mut fixture, 2, "subscribe", &characteristic_params(), &[]).await;
        assert!(status.is_none());

        fixture
            .device
            .push_notify(crate::testing::CHAR_UUID, b"hello");
        // Adapter lifecycle events (connect-side discoveries) may arrive
        // first; wait for the notification envelope.
        let event = loop {
            let event = fixture.recorder.next_event().await;
            if event.contains("\"type\":\"notification\"") {
                break event;
            }
        };
        let value: serde_json::Value = serde_json::from_str(&event).expect("envelope JSON");
        assert_eq!(value["type"], "notification", "{event}");
        assert_eq!(
            value["notification"]["data"],
            serde_json::json!([104, 101, 108, 108, 111]),
            "{event}"
        );
        assert_eq!(
            value["device"],
            serde_json::to_value(device_id()).expect("device JSON")
        );

        let (status, _) = roundtrip(
            &mut fixture,
            3,
            "unsubscribe",
            &characteristic_params(),
            &[],
        )
        .await;
        assert!(status.is_none());
        fixture
            .device
            .push_notify(crate::testing::CHAR_UUID, b"lost");
        fixture.recorder.expect_no_notification().await;
    }

    #[tokio::test]
    async fn write_echo_reaches_subscribers() {
        let mut fixture = Fixture::new().await;
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        let (status, _) =
            roundtrip(&mut fixture, 2, "subscribe", &characteristic_params(), &[]).await;
        assert!(status.is_none());
        let (status, _) =
            roundtrip(&mut fixture, 3, "write", &characteristic_params(), b"echo").await;
        assert!(status.is_none());
        let event = loop {
            let event = fixture.recorder.next_event().await;
            if event.contains("\"type\":\"notification\"") {
                break event;
            }
        };
        assert!(event.contains("[101,99,104,111]"), "{event}");
    }

    #[tokio::test]
    async fn system_connected_lookup_requires_service_uuids() {
        let mut fixture = Fixture::new().await;
        for params in ["{}", r#"{"services":["gen3-5f21"]}"#] {
            let (status, _) =
                roundtrip(&mut fixture, 1, "system_connected_peripherals", params, &[]).await;
            assert!(status.is_some_and(|error| error.contains("InvalidArgument")));
        }
    }

    #[tokio::test]
    async fn peripherals_and_is_connected() {
        let mut fixture = Fixture::new().await;
        let (status, payload) = roundtrip(&mut fixture, 1, "peripherals", "{}", &[]).await;
        assert!(status.is_none());
        let infos: serde_json::Value = serde_json::from_slice(&payload).expect("infos JSON");
        assert_eq!(infos[0]["name"], "simulated-sensor", "{infos}");
        assert_eq!(
            infos[0]["id"],
            serde_json::to_value(device_id()).expect("device JSON")
        );

        roundtrip(&mut fixture, 2, "connect", &device_params(), &[]).await;
        let (status, payload) =
            roundtrip(&mut fixture, 3, "is_connected", &device_params(), &[]).await;
        assert!(status.is_none());
        assert_eq!(payload, b"true");
    }

    #[tokio::test]
    async fn set_connection_priority_requires_connection_and_valid_enum() {
        let mut fixture = Fixture::new().await;
        let params = format!(
            r#"{{"device":{},"priority":"high"}}"#,
            serde_json::to_string(&device_id()).expect("device JSON")
        );
        let (status, _) = roundtrip(&mut fixture, 1, "set_connection_priority", &params, &[]).await;
        assert!(
            status
                .as_deref()
                .is_some_and(|error| error.contains("NotConnected")),
            "{status:?}"
        );

        roundtrip(&mut fixture, 2, "connect", &device_params(), &[]).await;
        let (status, _) = roundtrip(&mut fixture, 3, "set_connection_priority", &params, &[]).await;
        assert!(status.is_none(), "{status:?}");
        assert_eq!(
            fixture.device.connection_priority(),
            Some(ConnectionPriority::High)
        );

        let (status, _) = roundtrip(
            &mut fixture,
            4,
            "set_connection_priority",
            &format!(
                r#"{{"device":{},"priority":"turbo"}}"#,
                serde_json::to_string(&device_id()).expect("device JSON")
            ),
            &[],
        )
        .await;
        assert!(
            status
                .as_deref()
                .is_some_and(|error| error.contains("InvalidArgument")),
            "{status:?}"
        );
    }

    #[tokio::test]
    async fn disconnect_stops_forwarder_and_forgets_subscriptions() {
        let mut fixture = Fixture::new().await;
        roundtrip(&mut fixture, 1, "connect", &device_params(), &[]).await;
        roundtrip(&mut fixture, 2, "subscribe", &characteristic_params(), &[]).await;
        let (status, _) = roundtrip(&mut fixture, 3, "disconnect", &device_params(), &[]).await;
        assert!(status.is_none());
        fixture
            .device
            .push_notify(crate::testing::CHAR_UUID, b"lost");
        fixture.recorder.expect_no_notification().await;
    }

    #[tokio::test]
    async fn out_of_order_completion_is_correlated_by_req_id() {
        let adapter = Arc::new(GatedAdapter::new());
        let core = crate::handle::Core::with_adapter(Arc::clone(&adapter) as Arc<dyn Adapter>);
        let mut recorder = crate::testing::Recorder::attach(&core);
        core.spawn_request(
            1,
            Request {
                method: "initialize".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
        core.spawn_request(
            2,
            Request {
                method: "backend_version".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
        // The gated `initialize` cannot finish before `backend_version`, so
        // response 2 arrives first while both stay correlated by id.
        let (req_id, status, _) = recorder.next_response().await;
        assert_eq!(req_id, 2);
        assert!(status.is_none());
        adapter.open();
        let (req_id, status, _) = recorder.next_response().await;
        assert_eq!(req_id, 1);
        assert!(status.is_none());
    }

    /// An adapter whose `initialize` blocks on a test-side gate, to force a
    /// request to complete after a later one.
    struct GatedAdapter {
        gate: Arc<Gate>,
        id: AdapterId,
    }

    #[derive(Default)]
    struct Gate {
        opened: std::sync::atomic::AtomicBool,
        notify: tokio::sync::Notify,
    }

    impl GatedAdapter {
        fn new() -> Self {
            Self {
                gate: Arc::new(Gate::default()),
                id: AdapterId::default_adapter(),
            }
        }

        fn open(&self) {
            self.gate
                .opened
                .store(true, std::sync::atomic::Ordering::Release);
            self.gate.notify.notify_waiters();
        }
    }

    #[async_trait::async_trait]
    impl Adapter for GatedAdapter {
        fn id(&self) -> &AdapterId {
            &self.id
        }

        fn capabilities(&self) -> CapabilitySet {
            CapabilitySet::default()
        }

        async fn initialize(&self) -> Result<(), Error> {
            loop {
                if self.gate.opened.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                let notified = self.gate.notify.notified();
                if self.gate.opened.load(std::sync::atomic::Ordering::Acquire) {
                    return Ok(());
                }
                notified.await;
            }
        }

        async fn start_scan(&self, _filter: &ScanFilter) -> Result<(), Error> {
            Ok(())
        }

        async fn stop_scan(&self) -> Result<(), Error> {
            Ok(())
        }

        fn events(&self) -> AdapterEventStream {
            futures::stream::empty().boxed()
        }

        async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
            Ok(Vec::new())
        }

        async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
            Err(Error::NotFound(id.clone()))
        }

        async fn request_device(
            &self,
            _opts: &RequestDeviceOptions,
        ) -> Result<Arc<dyn Peripheral>, Error> {
            Err(Error::Unsupported {
                platform: "test",
                feature: "request_device",
            })
        }
    }
}
