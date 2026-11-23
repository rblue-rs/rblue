//! The `wasm32` implementation of the [`rblue_api`] trait surface over Web
//! Bluetooth.
//!
//! The Web Bluetooth DOM interfaces used here are declared directly with
//! `wasm_bindgen` (see the crate docs for why `web-sys`'s own bindings are
//! not used). `web-sys` supplies the stable `Window`/`Navigator`/`Event`
//! types.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use futures::StreamExt;
use js_sys::{Array, Object, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::Event;

use rblue_api::{
    Adapter, AdapterEvent, AdapterEventStream, AdapterId, AdapterState, AdvertisementData,
    Capability, CapabilitySet, Characteristic, CharacteristicId, CharacteristicProperties,
    DeviceId, DisconnectReason, DiscoveredDevice, Error, Notification, NotificationStream,
    Peripheral, RequestDeviceOptions, ScanFilter, Service, ServiceId, ServiceSet, WriteMode,
};

use crate::broadcast::{BroadcastBus, Delivery};
use crate::error_map::{map_dom_error, DomError, ErrorSite, PLATFORM};
use crate::id::wrap_bluetooth_device_id;
use crate::options::{plan_request_device, RequestDevicePlan};
use crate::sync::lock;

/// Conservative write ceiling: default ATT MTU (23) minus the 3-byte
/// opcode+handle header. The browser does not chunk writes, and the real
/// negotiated MTU is not observable, so payloads above this fail with
/// [`Error::InvalidLength`] before any browser call.
const MAX_WRITE_LEN: usize = 20;

/// `Send`/`Sync` wrapper for JS values and futures.
///
/// # Safety
///
/// `wasm32-unknown-unknown` executes all Rust on a single JS thread unless an
/// embedder opts into wasm threads, and Web Bluetooth is main-thread-only.
/// The wrapper never makes values accessible from a second thread in
/// practice; it exists so the runtime-neutral `rblue_api` traits (whose
/// futures must be `Send`) can hold them. A worker realm must construct its
/// own adapter instead of moving one across realms.
struct JsSend<T>(T);

unsafe impl<T> Send for JsSend<T> {}
unsafe impl<T> Sync for JsSend<T> {}

impl<T> JsSend<T> {
    fn new(value: T) -> Self {
        Self(value)
    }
}

impl<T: Clone> Clone for JsSend<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> std::ops::Deref for JsSend<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<F: Future + Unpin> Future for JsSend<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

// Web Bluetooth DOM surface. One `extern` block per interface keeps the
// method-name declarations independent.

#[wasm_bindgen]
extern "C" {
    /// The `navigator.bluetooth` object.
    #[derive(Clone)]
    type Bluetooth;

    /// Opens the device chooser. Must run inside a user gesture.
    #[wasm_bindgen(catch, method, js_name = "requestDevice")]
    fn request_device(this: &Bluetooth, options: &Object) -> Result<js_sys::Promise, JsValue>;

    /// Resolves whether a Bluetooth radio is available.
    #[wasm_bindgen(catch, method, js_name = "getAvailability")]
    fn get_availability(this: &Bluetooth) -> Result<js_sys::Promise, JsValue>;
}

#[wasm_bindgen]
extern "C" {
    /// A device picked through the chooser.
    #[derive(Clone)]
    type BluetoothDevice;

    #[wasm_bindgen(method, getter, js_name = "id")]
    fn id(this: &BluetoothDevice) -> String;
    #[wasm_bindgen(method, getter, js_name = "name")]
    fn name(this: &BluetoothDevice) -> Option<String>;
    #[wasm_bindgen(method, getter, js_name = "gatt")]
    fn gatt(this: &BluetoothDevice) -> Option<BluetoothRemoteGattServer>;
    #[wasm_bindgen(method, js_name = "addEventListener")]
    fn add_event_listener_with_callback(
        this: &BluetoothDevice,
        type_: &str,
        callback: &js_sys::Function,
    );
}

#[wasm_bindgen]
extern "C" {
    /// The GATT server of one device.
    #[derive(Clone)]
    type BluetoothRemoteGattServer;

    #[wasm_bindgen(catch, method, js_name = "connect")]
    fn connect(this: &BluetoothRemoteGattServer) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(method, js_name = "disconnect")]
    fn disconnect(this: &BluetoothRemoteGattServer);
    #[wasm_bindgen(method, getter, js_name = "connected")]
    fn is_connected(this: &BluetoothRemoteGattServer) -> bool;
    #[wasm_bindgen(catch, method, js_name = "getPrimaryServices")]
    fn get_primary_services(this: &BluetoothRemoteGattServer) -> Result<js_sys::Promise, JsValue>;
}

#[wasm_bindgen]
extern "C" {
    /// One primary GATT service.
    #[derive(Clone)]
    type BluetoothRemoteGattService;

    #[wasm_bindgen(method, getter, js_name = "uuid")]
    fn uuid(this: &BluetoothRemoteGattService) -> String;
    #[wasm_bindgen(method, getter, js_name = "isPrimary")]
    fn is_primary(this: &BluetoothRemoteGattService) -> bool;
    #[wasm_bindgen(catch, method, js_name = "getCharacteristics")]
    fn get_characteristics(this: &BluetoothRemoteGattService) -> Result<js_sys::Promise, JsValue>;
}

#[wasm_bindgen]
extern "C" {
    /// One GATT characteristic.
    #[derive(Clone)]
    type BluetoothRemoteGattCharacteristic;

    #[wasm_bindgen(method, getter, js_name = "uuid")]
    fn uuid(this: &BluetoothRemoteGattCharacteristic) -> String;
    #[wasm_bindgen(method, getter, js_name = "properties")]
    fn properties(this: &BluetoothRemoteGattCharacteristic) -> BluetoothCharacteristicProperties;
    #[wasm_bindgen(method, getter, js_name = "value")]
    fn value(this: &BluetoothRemoteGattCharacteristic) -> Option<js_sys::DataView>;
    #[wasm_bindgen(catch, method, js_name = "readValue")]
    fn read_value(this: &BluetoothRemoteGattCharacteristic) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, method, js_name = "writeValueWithResponse")]
    fn write_value_with_response(
        this: &BluetoothRemoteGattCharacteristic,
        value: &[u8],
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, method, js_name = "writeValueWithoutResponse")]
    fn write_value_without_response(
        this: &BluetoothRemoteGattCharacteristic,
        value: &[u8],
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, method, js_name = "startNotifications")]
    fn start_notifications(
        this: &BluetoothRemoteGattCharacteristic,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(catch, method, js_name = "stopNotifications")]
    fn stop_notifications(
        this: &BluetoothRemoteGattCharacteristic,
    ) -> Result<js_sys::Promise, JsValue>;
    #[wasm_bindgen(method, js_name = "addEventListener")]
    fn add_event_listener_with_callback(
        this: &BluetoothRemoteGattCharacteristic,
        type_: &str,
        callback: &js_sys::Function,
    );
    #[wasm_bindgen(method, js_name = "removeEventListener")]
    fn remove_event_listener_with_callback(
        this: &BluetoothRemoteGattCharacteristic,
        type_: &str,
        callback: &js_sys::Function,
    );
}

#[wasm_bindgen]
extern "C" {
    /// Property bits of one characteristic.
    #[derive(Clone)]
    type BluetoothCharacteristicProperties;

    #[wasm_bindgen(method, getter, js_name = "broadcast")]
    fn broadcast(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "read")]
    fn read(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "writeWithoutResponse")]
    fn write_without_response(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "write")]
    fn write(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "notify")]
    fn notify(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "indicate")]
    fn indicate(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "authenticatedSignedWrites")]
    fn authenticated_signed_writes(this: &BluetoothCharacteristicProperties) -> bool;
    #[wasm_bindgen(method, getter, js_name = "extendedProperties")]
    fn extended_properties(this: &BluetoothCharacteristicProperties) -> bool;
}

/// Connection-scoped state of a peripheral. Discardable when the link drops;
/// identifiers only survive for the current connection.
#[derive(Default)]
struct Connection {
    server: Option<JsSend<BluetoothRemoteGattServer>>,
    characteristics: HashMap<CharacteristicId, JsSend<BluetoothRemoteGattCharacteristic>>,
    listeners: HashMap<CharacteristicId, NotifyListener>,
    disconnect_listener: Option<NotifyListener>,
}

/// A kept-alive event listener; dropping it releases the JS closure.
type NotifyListener = JsSend<Closure<dyn FnMut(JsValue)>>;

/// The web peripheral: one `BluetoothDevice` plus its per-connection GATT
/// state.
struct WebPeripheral {
    device: JsSend<BluetoothDevice>,
    device_id: DeviceId,
    name: Option<String>,
    /// Adapter-level event bus (shared with the adapter).
    events: BroadcastBus<AdapterEvent>,
    /// Notification broadcast of this peripheral.
    notifications: BroadcastBus<Notification>,
    /// Set by an explicit `disconnect` so the disconnect event can be
    /// attributed to the local side.
    local_disconnect: Arc<AtomicBool>,
    connection: Mutex<Connection>,
}

/// The web adapter: chooser entry point plus the device registry and event
/// bus.
struct WebAdapter {
    adapter_id: AdapterId,
    bluetooth: JsSend<Option<Bluetooth>>,
    devices: Mutex<Vec<Arc<WebPeripheral>>>,
    events: BroadcastBus<AdapterEvent>,
    watch_probed: AtomicBool,
    watch_supported: AtomicBool,
}

/// Constructs the web adapter, detecting `navigator.bluetooth` availability.
pub(crate) fn default_adapter() -> Arc<dyn Adapter> {
    Arc::new(WebAdapter::new())
}

/// Detects `BluetoothDevice.watchAdvertisements()` support (Chrome flag).
///
/// Inspects `window.BluetoothDevice.prototype`, so no device instance is
/// needed; returns `false` outside a browser realm.
pub(crate) fn detect_watch_advertisements() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let window = Object::from(JsValue::from(window));
    let constructor = match Reflect::get(&window, &JsValue::from_str("BluetoothDevice")) {
        Ok(constructor) if !constructor.is_undefined() => constructor,
        _ => return false,
    };
    let constructor = Object::from(constructor);
    let prototype = match Reflect::get(&constructor, &JsValue::from_str("prototype")) {
        Ok(prototype) => Object::from(prototype),
        Err(_) => return false,
    };
    Reflect::has(&prototype, &JsValue::from_str("watchAdvertisements")).unwrap_or(false)
}

impl WebAdapter {
    fn new() -> Self {
        Self {
            adapter_id: AdapterId::default_adapter(),
            bluetooth: JsSend::new(locate_bluetooth()),
            devices: Mutex::new(Vec::new()),
            events: BroadcastBus::new(),
            watch_probed: AtomicBool::new(false),
            watch_supported: AtomicBool::new(false),
        }
    }

    /// The `navigator.bluetooth` object, when the platform has one.
    fn bluetooth(&self) -> Option<&Bluetooth> {
        self.bluetooth.as_ref()
    }

    /// Best-effort radio probe. Failures (insecure context, missing API)
    /// leave the state [`AdapterState::Unknown`] and are not fatal.
    async fn probe_availability(&self) -> AdapterState {
        let Some(bluetooth) = self.bluetooth() else {
            return AdapterState::Unavailable;
        };
        match bluetooth.get_availability() {
            Ok(promise) => match JsSend::new(JsFuture::from(promise)).await {
                Ok(available) => {
                    if let Some(available) = available.as_bool() {
                        if available {
                            AdapterState::PoweredOn
                        } else {
                            AdapterState::PoweredOff
                        }
                    } else {
                        AdapterState::Unknown
                    }
                }
                Err(js) => {
                    tracing::debug!(?js, "navigator.bluetooth.getAvailability() rejected");
                    AdapterState::Unknown
                }
            },
            Err(js) => {
                tracing::debug!(?js, "navigator.bluetooth.getAvailability() threw");
                AdapterState::Unknown
            }
        }
    }
}

/// Reads `navigator.bluetooth` from the global window; `None` where the
/// platform has no Web Bluetooth (Safari, Firefox, non-browser runtimes).
fn locate_bluetooth() -> Option<Bluetooth> {
    let navigator = Object::from(JsValue::from(web_sys::window()?.navigator()));
    let bluetooth = Reflect::get(&navigator, &JsValue::from_str("bluetooth")).ok()?;
    if bluetooth.is_undefined() || bluetooth.is_null() {
        return None;
    }
    Some(bluetooth.unchecked_into())
}

/// Extracts `name`/`message` from a rejected JS value and maps it.
fn js_to_error(js: JsValue, site: &ErrorSite) -> Error {
    let source = Object::from(js);
    let name = Reflect::get(&source, &JsValue::from_str("name"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| "UnknownError".to_owned());
    let message = Reflect::get(&source, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_default();
    map_dom_error(
        DomError {
            name: &name,
            message: &message,
        },
        site,
    )
}

/// Convenience wrapper for GATT-site errors.
fn gatt_error(js: JsValue, device: &DeviceId, feature: &'static str) -> Error {
    js_to_error(
        js,
        &ErrorSite::Gatt {
            device: device.clone(),
            feature,
        },
    )
}

/// Serializes a [`RequestDevicePlan`] into the `requestDevice` options
/// object.
fn request_options_object(plan: &RequestDevicePlan) -> Object {
    let options = Object::new();
    match plan {
        RequestDevicePlan::AcceptAll { optional_services } => {
            set_property(&options, "acceptAllDevices", &JsValue::from_bool(true));
            set_optional_services(&options, optional_services);
        }
        RequestDevicePlan::Filtered {
            services,
            name_prefix,
            optional_services,
        } => {
            let filter = Object::new();
            let services: Array = services
                .iter()
                .map(|uuid| JsValue::from_str(uuid))
                .collect();
            set_property(&filter, "services", services.as_ref());
            if let Some(prefix) = name_prefix {
                set_property(&filter, "namePrefix", &JsValue::from_str(prefix));
            }
            let filters = Array::new();
            filters.push(filter.as_ref());
            set_property(&options, "filters", filters.as_ref());
            set_optional_services(&options, optional_services);
        }
    }
    options
}

/// Sets one property; only reachable failure is a non-object target, and the
/// targets here are freshly created plain objects.
fn set_property(target: &Object, key: &str, value: &JsValue) {
    Reflect::set(target, &JsValue::from_str(key), value)
        .expect("setting a property on a fresh plain object cannot fail");
}

fn set_optional_services(options: &Object, services: &[String]) {
    let array: Array = services
        .iter()
        .map(|service| JsValue::from_str(service))
        .collect();
    set_property(options, "optionalServices", array.as_ref());
}

/// Allocates the next zero-based instance index for a repeated UUID.
fn next_instance_index(counters: &mut HashMap<uuid::Uuid, u16>, uuid: uuid::Uuid) -> u16 {
    let counter = counters.entry(uuid).or_insert(0);
    let index = *counter;
    *counter += 1;
    index
}

/// Copies a `DataView` into owned bytes.
fn data_view_bytes(view: &js_sys::DataView) -> Vec<u8> {
    js_sys::Uint8Array::new_with_byte_offset_and_length(
        &view.buffer(),
        view.byte_offset() as u32,
        view.byte_length() as u32,
    )
    .to_vec()
}

/// Extracts the notification payload from a `characteristicvaluechanged`
/// event; `None` when the event does not carry a characteristic value.
fn notification_payload(event: &JsValue) -> Option<Vec<u8>> {
    let event = event.dyn_ref::<Event>()?;
    let target = event.target()?;
    let characteristic = target.dyn_ref::<BluetoothRemoteGattCharacteristic>()?;
    characteristic.value().as_ref().map(data_view_bytes)
}

/// Reads the property bits of a characteristic into the unified model. The
/// reliable-write/writable-auxiliaries bits live in a descriptor the web
/// does not expose and stay unset.
fn read_properties(characteristic: &BluetoothRemoteGattCharacteristic) -> CharacteristicProperties {
    let properties = characteristic.properties();
    let mut mapped = CharacteristicProperties::default();
    for (flag, supported) in [
        (CharacteristicProperties::BROADCAST, properties.broadcast()),
        (CharacteristicProperties::READ, properties.read()),
        (
            CharacteristicProperties::WRITE_WITHOUT_RESPONSE,
            properties.write_without_response(),
        ),
        (CharacteristicProperties::WRITE, properties.write()),
        (CharacteristicProperties::NOTIFY, properties.notify()),
        (CharacteristicProperties::INDICATE, properties.indicate()),
        (
            CharacteristicProperties::AUTHENTICATED_SIGNED_WRITES,
            properties.authenticated_signed_writes(),
        ),
        (
            CharacteristicProperties::EXTENDED_PROPERTIES,
            properties.extended_properties(),
        ),
    ] {
        if supported {
            mapped |= flag;
        }
    }
    mapped
}

/// Builds the `gattserverdisconnected` listener. It attributes the event to
/// the local side when an explicit `disconnect` preceded it and publishes
/// the adapter-level disconnect event.
fn make_disconnect_closure(
    events: BroadcastBus<AdapterEvent>,
    device_id: DeviceId,
    local_disconnect: Arc<AtomicBool>,
) -> Closure<dyn FnMut(JsValue)> {
    Closure::new(move |_event: JsValue| {
        let local = local_disconnect.swap(false, Ordering::Relaxed);
        let reason = if local {
            DisconnectReason::Local
        } else {
            DisconnectReason::Unknown
        };
        events.publish(
            AdapterEvent::DeviceDisconnected(device_id.clone(), Some(reason)),
            Delivery::Guaranteed,
        );
    })
}

impl WebPeripheral {
    /// Wraps a freshly picked device into a peripheral.
    fn new(device: BluetoothDevice, events: BroadcastBus<AdapterEvent>) -> Self {
        let device_id = wrap_bluetooth_device_id(&device.id());
        let name = device.name();
        Self {
            device: JsSend::new(device),
            device_id,
            name,
            events,
            notifications: BroadcastBus::new(),
            local_disconnect: Arc::new(AtomicBool::new(false)),
            connection: Mutex::new(Connection::default()),
        }
    }

    /// The advertisement-shaped snapshot this backend can offer: the
    /// advertised name seen at chooser time.
    fn snapshot(&self) -> DiscoveredDevice {
        DiscoveredDevice {
            id: self.device_id.clone(),
            advertisement: AdvertisementData {
                local_name: self.name.clone(),
                ..AdvertisementData::default()
            },
        }
    }

    /// The live GATT server handle, when a connection was established.
    fn connected_server(&self) -> Option<JsSend<BluetoothRemoteGattServer>> {
        lock(&self.connection).server.clone()
    }

    /// The discovered characteristic for `id`. Characteristics only exist
    /// after discovery on the current connection.
    fn characteristic(
        &self,
        id: &CharacteristicId,
    ) -> Result<JsSend<BluetoothRemoteGattCharacteristic>, Error> {
        lock(&self.connection)
            .characteristics
            .get(id)
            .cloned()
            .ok_or_else(|| {
                Error::InvalidState(
                    "characteristic is unknown; run discover_services on this connection first"
                        .to_owned(),
                )
            })
    }
}

#[async_trait]
impl Adapter for WebAdapter {
    fn id(&self) -> &AdapterId {
        &self.adapter_id
    }

    fn capabilities(&self) -> CapabilitySet {
        let mut capabilities = CapabilitySet::bit(Capability::GattClient)
            | CapabilitySet::bit(Capability::RequestDevicePicker);
        // Static default: off. The bit appears only after a successful
        // runtime probe, cached so repeated calls stay cheap.
        if !self.watch_probed.swap(true, Ordering::Relaxed) {
            self.watch_supported
                .store(detect_watch_advertisements(), Ordering::Relaxed);
        }
        if self.watch_supported.load(Ordering::Relaxed) {
            capabilities |= CapabilitySet::bit(Capability::WatchAdvertisements);
        }
        capabilities
    }

    async fn initialize(&self) -> Result<(), Error> {
        if self.bluetooth().is_none() {
            return Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "bluetooth",
            });
        }
        let state = self.probe_availability().await;
        if state != AdapterState::Unknown {
            self.events
                .publish(AdapterEvent::StateChanged(state), Delivery::Guaranteed);
        }
        Ok(())
    }

    async fn state(&self) -> Result<AdapterState, Error> {
        if self.bluetooth().is_none() {
            return Ok(AdapterState::Unavailable);
        }
        Ok(self.probe_availability().await)
    }

    async fn set_powered(&self, _powered: bool) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "set_powered",
        })
    }

    async fn start_scan(&self, _filter: &ScanFilter) -> Result<(), Error> {
        // The browser has no background scanning; devices arrive through the
        // request_device chooser instead.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "scan",
        })
    }

    async fn stop_scan(&self) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "scan",
        })
    }

    fn events(&self) -> AdapterEventStream {
        // Snapshot-then-live: devices known so far are replayed as fresh
        // discoveries to the new subscriber only.
        let snapshot = lock(&self.devices)
            .iter()
            .map(|device| AdapterEvent::DeviceDiscovered(device.snapshot()))
            .collect::<Vec<_>>();
        self.events.subscribe_with(snapshot).boxed()
    }

    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        Ok(lock(&self.devices)
            .iter()
            .cloned()
            .map(|device| device as Arc<dyn Peripheral>)
            .collect())
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        lock(&self.devices)
            .iter()
            .find(|device| device.id() == id)
            .map(|device| Arc::clone(device) as Arc<dyn Peripheral>)
            .ok_or_else(|| Error::NotFound(id.clone()))
    }

    async fn request_device(
        &self,
        opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error> {
        let plan = plan_request_device(opts)?;
        let Some(bluetooth) = self.bluetooth() else {
            return Err(Error::Unsupported {
                platform: PLATFORM,
                feature: "bluetooth",
            });
        };
        // The options object is dropped before the await: it is a JS value
        // and must not sit in the (Send) future state.
        let promise = {
            let options = request_options_object(&plan);
            bluetooth
                .request_device(&options)
                .map_err(|js| js_to_error(js, &ErrorSite::RequestDevice))?
        };
        let picked = JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| js_to_error(js, &ErrorSite::RequestDevice))?;
        let device: BluetoothDevice = picked.unchecked_into();
        let peripheral = Arc::new(WebPeripheral::new(device, self.events.clone()));
        lock(&self.devices).push(peripheral.clone());
        self.events.publish(
            AdapterEvent::DeviceDiscovered(peripheral.snapshot()),
            Delivery::BestEffort,
        );
        Ok(peripheral)
    }
}

#[async_trait]
impl Peripheral for WebPeripheral {
    fn id(&self) -> &DeviceId {
        &self.device_id
    }

    fn name(&self) -> Option<String> {
        self.name.clone()
    }

    async fn is_connected(&self) -> Result<bool, Error> {
        Ok(self
            .connected_server()
            .map(|server| server.is_connected())
            .unwrap_or(false))
    }

    async fn connect(&self) -> Result<(), Error> {
        let server = JsSend::new(
            self.device
                .gatt()
                .ok_or_else(|| Error::InvalidState("device has no GATT server".to_owned()))?,
        );
        // Idempotent: connecting an already-connected device succeeds.
        if server.is_connected() {
            return Ok(());
        }
        let promise = server
            .connect()
            .map_err(|js| gatt_error(js, &self.device_id, "connect"))?;
        JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, "connect"))?;
        {
            let mut connection = lock(&self.connection);
            if connection.server.is_none() {
                connection.server = Some(server);
            }
            if connection.disconnect_listener.is_none() {
                let closure = JsSend::new(make_disconnect_closure(
                    self.events.clone(),
                    self.device_id.clone(),
                    Arc::clone(&self.local_disconnect),
                ));
                self.device.add_event_listener_with_callback(
                    "gattserverdisconnected",
                    closure.as_ref().unchecked_ref(),
                );
                connection.disconnect_listener = Some(closure);
            }
        }
        self.events.publish(
            AdapterEvent::DeviceConnected(self.device_id.clone()),
            Delivery::Guaranteed,
        );
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), Error> {
        let Some(server) = self.connected_server() else {
            return Ok(()); // never connected: already disconnected
        };
        if !server.is_connected() {
            return Ok(());
        }
        // The disconnect event completes the state transition asynchronously;
        // the flag attributes it to the local side.
        self.local_disconnect.store(true, Ordering::Relaxed);
        server.disconnect();
        Ok(())
    }

    async fn discover_services(&self, filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
        let server = self.connected_server().ok_or(Error::NotConnected)?;
        let promise = server
            .get_primary_services()
            .map_err(|js| gatt_error(js, &self.device_id, "discover_services"))?;
        let resolved = JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, "discover_services"))?;
        // The browser permission-filters this list against optionalServices.
        let services = JsSend::new(resolved.dyn_into::<Array>().map_err(|_| {
            Error::Backend("getPrimaryServices() did not resolve to an array".to_owned())
        })?);
        let mut found = Vec::with_capacity(services.length() as usize);
        let mut characteristics = HashMap::new();
        let mut service_instances: HashMap<uuid::Uuid, u16> = HashMap::new();
        for service_value in services.iter() {
            let service: JsSend<BluetoothRemoteGattService> =
                JsSend::new(service_value.unchecked_into());
            let uuid_text = service.uuid();
            let service_uuid = uuid::Uuid::parse_str(&uuid_text).map_err(|_| {
                Error::Backend(format!(
                    "browser reported unparseable service UUID {uuid_text}"
                ))
            })?;
            if !filter.is_empty() && !filter.contains(&service_uuid) {
                continue;
            }
            let instance = next_instance_index(&mut service_instances, service_uuid);
            let chars_promise = service
                .get_characteristics()
                .map_err(|js| gatt_error(js, &self.device_id, "discover_services"))?;
            let chars_resolved = JsSend::new(JsFuture::from(chars_promise))
                .await
                .map_err(|js| gatt_error(js, &self.device_id, "discover_services"))?;
            let chars = chars_resolved.dyn_into::<Array>().map_err(|_| {
                Error::Backend("getCharacteristics() did not resolve to an array".to_owned())
            })?;
            let mut listed = Vec::with_capacity(chars.length() as usize);
            let mut characteristic_instances: HashMap<uuid::Uuid, u16> = HashMap::new();
            for characteristic_value in chars.iter() {
                let characteristic: JsSend<BluetoothRemoteGattCharacteristic> =
                    JsSend::new(characteristic_value.unchecked_into());
                let uuid_text = characteristic.uuid();
                let characteristic_uuid = uuid::Uuid::parse_str(&uuid_text).map_err(|_| {
                    Error::Backend(format!(
                        "browser reported unparseable characteristic UUID {uuid_text}"
                    ))
                })?;
                let id = CharacteristicId {
                    service: ServiceId {
                        uuid: service_uuid,
                        instance,
                    },
                    uuid: characteristic_uuid,
                    instance: next_instance_index(
                        &mut characteristic_instances,
                        characteristic_uuid,
                    ),
                };
                listed.push(Characteristic {
                    uuid: characteristic_uuid,
                    properties: read_properties(&characteristic),
                    descriptors: Vec::new(),
                });
                characteristics.insert(id, characteristic);
            }
            found.push(Service {
                uuid: service_uuid,
                primary: service.is_primary(),
                characteristics: listed,
            });
        }
        lock(&self.connection).characteristics = characteristics;
        Ok(ServiceSet { services: found })
    }

    async fn read(&self, ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
        let characteristic = self.characteristic(ch)?;
        let promise = characteristic
            .read_value()
            .map_err(|js| gatt_error(js, &self.device_id, "read"))?;
        JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, "read"))?;
        Ok(characteristic
            .value()
            .as_ref()
            .map(data_view_bytes)
            .unwrap_or_default())
    }

    async fn write(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        // Unified write semantics: no silent truncation, no hidden chunking.
        if data.len() > MAX_WRITE_LEN {
            return Err(Error::InvalidLength {
                len: data.len(),
                max: MAX_WRITE_LEN,
            });
        }
        let feature = match mode {
            WriteMode::WithResponse => "write_with_response",
            WriteMode::WithoutResponse => "write_without_response",
        };
        let characteristic = self.characteristic(ch)?;
        let promise = match mode {
            WriteMode::WithResponse => characteristic.write_value_with_response(data),
            WriteMode::WithoutResponse => characteristic.write_value_without_response(data),
        }
        .map_err(|js| gatt_error(js, &self.device_id, feature))?;
        JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, feature))?;
        Ok(())
    }

    async fn max_write_len(
        &self,
        _ch: &CharacteristicId,
        _mode: WriteMode,
    ) -> Result<usize, Error> {
        // The negotiated MTU is not observable; report the conservative
        // default-ATT-MTU ceiling for both modes.
        Ok(MAX_WRITE_LEN)
    }

    async fn subscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let characteristic = self.characteristic(ch)?;
        let properties = read_properties(&characteristic);
        if !properties
            .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE)
        {
            return Err(Error::InvalidState(
                "characteristic supports neither notify nor indicate".to_owned(),
            ));
        }
        let promise = characteristic
            .start_notifications()
            .map_err(|js| gatt_error(js, &self.device_id, "subscribe"))?;
        JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, "subscribe"))?;
        let mut connection = lock(&self.connection);
        if connection.listeners.contains_key(ch) {
            // Already listening on this connection; the browser-side
            // startNotifications call is idempotent.
            return Ok(());
        }
        let bus = self.notifications.clone();
        let id = *ch;
        let closure = JsSend::new(Closure::new(move |event: JsValue| {
            if let Some(data) = notification_payload(&event) {
                bus.publish(Notification { id, data }, Delivery::BestEffort);
            }
        }));
        characteristic.add_event_listener_with_callback(
            "characteristicvaluechanged",
            closure.as_ref().unchecked_ref(),
        );
        connection.listeners.insert(*ch, closure);
        Ok(())
    }

    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        let (characteristic, closure) = {
            let mut connection = lock(&self.connection);
            let Some(closure) = connection.listeners.remove(ch) else {
                return Ok(()); // idempotent
            };
            let Some(characteristic) = connection.characteristics.get(ch).cloned() else {
                return Ok(());
            };
            (characteristic, closure)
        };
        characteristic.remove_event_listener_with_callback(
            "characteristicvaluechanged",
            closure.as_ref().unchecked_ref(),
        );
        let promise = characteristic
            .stop_notifications()
            .map_err(|js| gatt_error(js, &self.device_id, "unsubscribe"))?;
        JsSend::new(JsFuture::from(promise))
            .await
            .map_err(|js| gatt_error(js, &self.device_id, "unsubscribe"))?;
        Ok(())
    }

    fn notifications(&self) -> NotificationStream {
        self.notifications.subscribe().boxed()
    }

    async fn pair(&self) -> Result<(), Error> {
        // The web has no active-pairing API; the browser pairs implicitly
        // when an encrypted characteristic is accessed.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "pair",
        })
    }

    async fn unpair(&self) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "unpair",
        })
    }

    async fn rssi(&self) -> Result<i16, Error> {
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "rssi",
        })
    }

    async fn mtu(&self) -> Result<u16, Error> {
        // The browser does not expose the negotiated MTU; use max_write_len.
        Err(Error::Unsupported {
            platform: PLATFORM,
            feature: "mtu",
        })
    }
}
