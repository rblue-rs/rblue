//! ohos-rs Node-API facade (feature `napi`, off by default).
//!
//! This is the JS-facing side of the `BleDriver` bridge, following the
//! `Rblue` shape of implementation plan section 3.6:
//!
//! - The ArkTS driver object is injected once through
//!   [`Rblue::register_driver`]; its `call` method is captured as a
//!   `ThreadsafeFunction` behind a [`crate::DriverTransport`], so every
//!   request leaves the JS thread immediately (plan section 9: no napi entry
//!   may block the JS thread).
//! - The driver completes requests through [`Rblue::complete_request`]
//!   (control-plane JSON) and pushes unsolicited traffic through
//!   [`Rblue::emit_event`] / [`Rblue::emit_frame`]; both are cheap parse-and-
//!   route calls, never blocking I/O.
//! - Application-facing operations are `async` methods: napi-ohos (with the
//!   `async` feature) turns them into promises running on its embedded tokio
//!   runtime, so the JS thread is never blocked waiting for the driver.
//!
//! Wire format: identifiers, filters, and results cross the boundary as
//! serde JSON strings of the `rblue-api`/`rblue-driver` types - the same
//! shapes the protocol crate defines - while bulk payloads (write data, read
//! results, notification values) ride `ArrayBuffer`s, never JSON. Control-
//! plane JSON is low-frequency by protocol decision; the hot path is binary.
//!
//! The final `.so` for the HAR is built against the OHOS NDK with
//! `aarch64-unknown-linux-ohos` (device) and `x86_64-unknown-linux-ohos`
//! (simulator); host builds of this module are check-only (see
//! `docs/driver-roundtrip.md`).

use std::sync::Arc;

use napi_derive_ohos::napi;
use napi_ohos::threadsafe_function::{
    ThreadsafeCallContext, ThreadsafeFunction, ThreadsafeFunctionCallMode,
};
use napi_ohos::{
    bindgen_prelude::{spawn, start_async_runtime, Buffer, Function, JsObjectValue, Object},
    Error as NapiError, Result as NapiResult, Status, Unknown,
};

use rblue_api::{Adapter as _, DeviceId, Error as RblueError, Peripheral as _, ScanFilter};
use rblue_driver::{DriverEvent, DriverRequest, DriverResponse};

use crate::{DriverRegistry, DriverTransport, OhosAdapter, OhosPeripheral};

/// JS function handle with `'static` phantom types.
///
/// The `ThreadsafeFunction` built from a function carries its `Args`/`Return`
/// phantom parameters; annotating them `'static` keeps the resulting handle
/// nameable outside the napi call scope (the values themselves are never
/// touched - the callbacks pass plain JSON strings and objects).
type JsCallback<'a> = Function<'a, Unknown<'static>, Unknown<'static>>;

/// One request handed to the ArkTS driver's `call` method.
///
/// `request` is the serde JSON of [`DriverRequest`] (`reqId` + command);
/// `payload` carries the binary data-plane bytes (write data; empty for every
/// other command). The driver answers through [`Rblue::complete_request`]
/// (or a frame with the same `reqId` for reads).
#[napi(object)]
pub struct DriverCall {
    /// JSON-encoded `DriverRequest` (`{ reqId, type, ... }`).
    pub request: String,
    /// Binary data-plane payload; empty except for `write`.
    pub payload: Buffer,
}

/// Node-API facade over one [`DriverRegistry`].
///
/// The application creates one instance at startup, registers the ArkTS
/// driver, and consumes the async API; the driver object calls back into the
/// same instance.
#[napi]
pub struct Rblue {
    registry: DriverRegistry,
}

impl Default for Rblue {
    fn default() -> Self {
        Self::new()
    }
}

#[napi]
impl Rblue {
    /// Creates the facade with a fresh registry.
    #[napi(constructor)]
    pub fn new() -> Self {
        Self {
            registry: DriverRegistry::new(),
        }
    }

    /// Registers the ArkTS `BleDriver` implementation.
    ///
    /// The driver object must expose a `call(call: DriverCall)` method. The
    /// call is captured as a `ThreadsafeFunction` and wrapped into the
    /// registry's [`DriverTransport`]; registering again replaces the
    /// previous transport (driver restart).
    #[napi]
    pub fn register_driver(&self, driver: Object<'_>) -> NapiResult<()> {
        let call_fn: JsCallback = driver.get_named_property("call")?;
        let call: ThreadsafeFunction<DriverCall> = call_fn
            .build_threadsafe_function::<DriverCall>()
            .callee_handled::<true>()
            .build_callback(
                |ctx: ThreadsafeCallContext<DriverCall>| -> NapiResult<DriverCall> {
                    Ok(ctx.value)
                },
            )?;
        self.registry
            .register_transport(Arc::new(TsfnTransport::new(call)));
        Ok(())
    }

    /// Driver callback: completes one pending request.
    ///
    /// `response` is the serde JSON of [`DriverResponse`] (`{ reqId, status,
    /// ... }`). Responses for timed-out or unknown request ids are dropped
    /// (the awaiting side already failed); that race is expected, not an
    /// error.
    #[napi]
    pub fn complete_request(&self, response: String) -> NapiResult<()> {
        let response: DriverResponse = parse_json("driver response", &response)?;
        self.registry.complete_request(response);
        Ok(())
    }

    /// Driver callback: pushes one control-plane event.
    ///
    /// `event` is the serde JSON of [`DriverEvent`]. Notification events with
    /// empty `data` park the routing key for the paired binary frame
    /// (delivered through [`Rblue::emit_frame`]).
    #[napi]
    pub fn emit_event(&self, event: String) -> NapiResult<()> {
        let event: DriverEvent = parse_json("driver event", &event)?;
        self.registry.emit_event(event);
        Ok(())
    }

    /// Driver callback: ingests one binary data-plane frame.
    ///
    /// Completes pending `read` requests (`ReadResult` frames) and delivers
    /// notification payloads (`Notification` frames) - the hot path that
    /// never touches JSON.
    #[napi]
    pub fn emit_frame(&self, frame: Buffer) -> NapiResult<()> {
        self.registry.handle_frame(&frame).map_err(napi_error)
    }

    /// Forwards the adapter event stream to a JS callback.
    ///
    /// The stream follows the snapshot-then-live contract: every device known
    /// at subscription time is replayed as a `DeviceDiscovered`, then live
    /// events follow. Each callback invocation receives the serde JSON of one
    /// [`rblue_api::AdapterEvent`].
    #[napi]
    pub fn set_event_sink(&self, sink: JsCallback<'_>) -> NapiResult<()> {
        let forward: ThreadsafeFunction<String> = string_sink(sink)?;
        let events = self.registry.subscribe_events();
        forward_stream(events, |event| serde_json::to_string(&event), forward);
        Ok(())
    }

    /// Forwards the notification stream of one device to a JS callback.
    ///
    /// Values flow only for characteristics subscribed through
    /// [`Rblue::subscribe`]; each callback invocation receives the serde JSON
    /// of one [`rblue_api::Notification`]. Register exactly one sink per
    /// device: each call opens an independent forwarder on the broadcast
    /// stream, and there is no per-sink cancel handle (register the sink
    /// once when the device handle is obtained).
    #[napi]
    pub fn set_notification_sink(&self, device: String, sink: JsCallback<'_>) -> NapiResult<()> {
        let device: DeviceId = parse_json("device id", &device)?;
        let peripheral: Arc<OhosPeripheral> =
            self.registry.peripheral(&device).map_err(napi_error)?;
        let forward: ThreadsafeFunction<String> = string_sink(sink)?;
        forward_stream(
            peripheral.notifications(),
            |notification| serde_json::to_string(&notification),
            forward,
        );
        Ok(())
    }

    /// Opens or wakes the adapter (permission prompt + adapter state).
    #[napi]
    pub async fn initialize(&self) -> NapiResult<()> {
        OhosAdapter::new(&self.registry)
            .initialize()
            .await
            .map_err(napi_error)
    }

    /// Starts scanning with the serde-JSON-encoded `ScanFilter`.
    #[napi]
    pub async fn start_scan(&self, filter: String) -> NapiResult<()> {
        let filter: ScanFilter = parse_json("scan filter", &filter)?;
        OhosAdapter::new(&self.registry)
            .start_scan(&filter)
            .await
            .map_err(napi_error)
    }

    /// Stops the active scan.
    #[napi]
    pub async fn stop_scan(&self) -> NapiResult<()> {
        OhosAdapter::new(&self.registry)
            .stop_scan()
            .await
            .map_err(napi_error)
    }

    /// Snapshot of all known devices, as serde-JSON `DeviceId` strings.
    #[napi]
    pub async fn peripherals(&self) -> NapiResult<Vec<String>> {
        let peripherals = OhosAdapter::new(&self.registry)
            .peripherals()
            .await
            .map_err(napi_error)?;
        Ok(peripherals
            .iter()
            .filter_map(|peripheral| serde_json::to_string(peripheral.id()).ok())
            .collect())
    }

    /// Connects to the device identified by the serde-JSON `DeviceId`.
    #[napi]
    pub async fn connect(&self, device: String) -> NapiResult<()> {
        self.with_peripheral(&device)?
            .connect()
            .await
            .map_err(napi_error)
    }

    /// Disconnects; idempotent.
    #[napi]
    pub async fn disconnect(&self, device: String) -> NapiResult<()> {
        self.with_peripheral(&device)?
            .disconnect()
            .await
            .map_err(napi_error)
    }

    /// Discovers the GATT tree; returns the serde JSON of the `ServiceSet`.
    #[napi]
    pub async fn discover_services(
        &self,
        device: String,
        filter: Vec<String>,
    ) -> NapiResult<String> {
        let filter = filter
            .iter()
            .map(|uuid| {
                uuid.parse::<uuid::Uuid>()
                    .map_err(|err| invalid_argument("service uuid", &err.to_string()))
            })
            .collect::<NapiResult<Vec<uuid::Uuid>>>()?;
        let services = self
            .with_peripheral(&device)?
            .discover_services(&filter)
            .await
            .map_err(napi_error)?;
        serde_json::to_string(&services)
            .map_err(|err| invalid_argument("service set", &err.to_string()))
    }

    /// Reads a characteristic; the value comes back as raw bytes.
    #[napi]
    pub async fn read(&self, device: String, characteristic: String) -> NapiResult<Buffer> {
        let characteristic: rblue_api::CharacteristicId =
            parse_json("characteristic id", &characteristic)?;
        let data = self
            .with_peripheral(&device)?
            .read(&characteristic)
            .await
            .map_err(napi_error)?;
        Ok(data.into())
    }

    /// Writes raw bytes to a characteristic.
    ///
    /// Unified write semantics apply before dispatch (connection, property
    /// bits, `max_write_len`); the payload travels on the binary data plane.
    #[napi]
    pub async fn write(
        &self,
        device: String,
        characteristic: String,
        data: Buffer,
        with_response: bool,
    ) -> NapiResult<()> {
        let characteristic: rblue_api::CharacteristicId =
            parse_json("characteristic id", &characteristic)?;
        let mode = if with_response {
            rblue_api::WriteMode::WithResponse
        } else {
            rblue_api::WriteMode::WithoutResponse
        };
        self.with_peripheral(&device)?
            .write(&characteristic, &data, mode)
            .await
            .map_err(napi_error)
    }

    /// Reads a descriptor; the value comes back as raw bytes.
    #[napi]
    pub async fn read_descriptor(&self, device: String, descriptor: String) -> NapiResult<Buffer> {
        let descriptor: rblue_api::DescriptorId = parse_json("descriptor id", &descriptor)?;
        let data = self
            .with_peripheral(&device)?
            .read_descriptor(&descriptor)
            .await
            .map_err(napi_error)?;
        Ok(data.into())
    }

    /// Writes raw bytes to a descriptor. Oversize payloads are rejected
    /// with `InvalidLength` before dispatch; the payload travels on the
    /// binary data plane.
    #[napi]
    pub async fn write_descriptor(
        &self,
        device: String,
        descriptor: String,
        data: Buffer,
    ) -> NapiResult<()> {
        let descriptor: rblue_api::DescriptorId = parse_json("descriptor id", &descriptor)?;
        self.with_peripheral(&device)?
            .write_descriptor(&descriptor, &data)
            .await
            .map_err(napi_error)
    }

    /// Subscribes to value changes (notify or indicate per the properties).
    #[napi]
    pub async fn subscribe(&self, device: String, characteristic: String) -> NapiResult<()> {
        let characteristic: rblue_api::CharacteristicId =
            parse_json("characteristic id", &characteristic)?;
        self.with_peripheral(&device)?
            .subscribe(&characteristic)
            .await
            .map_err(napi_error)
    }

    /// Cancels a subscription; idempotent.
    #[napi]
    pub async fn unsubscribe(&self, device: String, characteristic: String) -> NapiResult<()> {
        let characteristic: rblue_api::CharacteristicId =
            parse_json("characteristic id", &characteristic)?;
        self.with_peripheral(&device)?
            .unsubscribe(&characteristic)
            .await
            .map_err(napi_error)
    }

    /// Requests an MTU change (`setBLEMtuSize`, API 10+); returns the
    /// negotiated ATT MTU.
    #[napi]
    pub async fn request_mtu(&self, device: String, mtu: u16) -> NapiResult<u16> {
        self.with_peripheral(&device)?
            .request_mtu(mtu)
            .await
            .map_err(napi_error)
    }

    fn with_peripheral(&self, device: &str) -> NapiResult<Arc<OhosPeripheral>> {
        let device: DeviceId = parse_json("device id", device)?;
        self.registry.peripheral(&device).map_err(napi_error)
    }
}

/// Wraps the driver's `call` ThreadsafeFunction as the registry transport.
struct TsfnTransport {
    call: ThreadsafeFunction<DriverCall>,
}

impl TsfnTransport {
    fn new(call: ThreadsafeFunction<DriverCall>) -> Self {
        Self { call }
    }
}

impl DriverTransport for TsfnTransport {
    fn send(&self, request: DriverRequest, payload: &[u8]) -> Result<(), RblueError> {
        let request = serde_json::to_string(&request).map_err(|err| {
            RblueError::Backend(format!("driver request does not serialize: {err}"))
        })?;
        let status = self.call.call(
            Ok(DriverCall {
                request,
                payload: payload.to_vec().into(),
            }),
            ThreadsafeFunctionCallMode::NonBlocking,
        );
        if status == Status::Ok {
            Ok(())
        } else {
            Err(RblueError::Backend(format!(
                "driver call queue rejected a request: {status:?}"
            )))
        }
    }
}

/// Turns a JS callback into a `ThreadsafeFunction` over JSON strings.
fn string_sink(sink: JsCallback<'_>) -> NapiResult<ThreadsafeFunction<String>> {
    sink.build_threadsafe_function::<String>()
        .callee_handled::<true>()
        .build_callback(|ctx: ThreadsafeCallContext<String>| -> NapiResult<String> {
            Ok(ctx.value)
        })
}

/// Spawns the stream-forwarding task on the embedded tokio runtime.
///
/// The runtime is started lazily here so the sink works even when no `async`
/// facade method ran before it.
fn forward_stream<T, F, S>(stream: S, encode: F, sink: ThreadsafeFunction<String>)
where
    S: futures::Stream<Item = T> + Unpin + Send + 'static,
    F: FnMut(&T) -> Result<String, serde_json::Error> + Send + 'static,
{
    start_async_runtime();
    spawn(async move {
        let mut encode = encode;
        futures::pin_mut!(stream);
        while let Some(item) = futures::StreamExt::next(&mut stream).await {
            let Ok(json) = encode(&item) else {
                continue;
            };
            sink.call(Ok(json), ThreadsafeFunctionCallMode::NonBlocking);
        }
    });
}

fn parse_json<T: serde::de::DeserializeOwned>(what: &str, json: &str) -> NapiResult<T> {
    serde_json::from_str(json).map_err(|err| invalid_argument(what, &err.to_string()))
}

fn invalid_argument(what: &str, detail: &str) -> NapiError {
    NapiError::new(Status::InvalidArg, format!("invalid {what}: {detail}"))
}

/// Maps the unified error model onto a Node-API error.
///
/// The serialized `rblue_api::Error` JSON is the error reason, so ArkTS sees
/// the structured variant (including `Error::Platform { code }` with the raw
/// OHOS error code) instead of a flat string.
fn napi_error(err: RblueError) -> NapiError {
    let reason = serde_json::to_string(&err).unwrap_or_else(|_| err.to_string());
    NapiError::new(Status::GenericFailure, reason)
}
