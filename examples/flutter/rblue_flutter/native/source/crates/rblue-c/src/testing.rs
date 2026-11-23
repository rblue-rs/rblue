//! Test-only support: a mock-backed facade fixture with a callback recorder.
//!
//! The recorder's `user_data` slots are intentionally leaked `Arc`s - they
//! outlive every test through the facade's worker threads and never get
//! reclaimed, which is exactly the embedder contract the callbacks rely on.

use std::ffi::CStr;
use std::os::raw::c_void;
use std::sync::Arc;
use std::time::Duration;

use rblue::prelude::*;
use rblue_api::{CharacteristicId, ServiceId};
use rblue_mock::{MockAdapter, MockCharacteristic, MockPeripheral, MockService};
use tokio::sync::mpsc::UnboundedSender;

use crate::handle::Core;

/// How long a test waits for a callback delivery before failing.
pub(crate) const RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);

/// Short window used to prove the absence of a delivery.
pub(crate) const ABSENCE_WINDOW: Duration = Duration::from_millis(300);

/// UUID of the fixture service.
pub(crate) const SERVICE_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x180F);

/// UUID of the fixture characteristic.
pub(crate) const CHAR_UUID: uuid::Uuid = uuid::Uuid::from_u128(0x2A19);

/// Properties of the fixture characteristic: readable, writable both ways,
/// and notifiable/indicatable.
pub(crate) const CHAR_PROPERTIES: CharacteristicProperties = CharacteristicProperties::READ
    .union(CharacteristicProperties::WRITE)
    .union(CharacteristicProperties::WRITE_WITHOUT_RESPONSE)
    .union(CharacteristicProperties::NOTIFY)
    .union(CharacteristicProperties::INDICATE);

/// Identity of the fixture device.
pub(crate) fn device_id() -> DeviceId {
    DeviceId::Mac(
        [0x02, 0x00, 0x00, 0x00, 0x00, 0x01],
        AddressType::RandomStatic,
    )
}

/// The fixture characteristic's session id, exactly as discovery reports it.
pub(crate) fn char_id() -> CharacteristicId {
    CharacteristicId {
        service: ServiceId {
            uuid: SERVICE_UUID,
            instance: 0,
        },
        uuid: CHAR_UUID,
        instance: 0,
    }
}

/// `{"device": ...}` parameter JSON for the fixture device.
pub(crate) fn device_params() -> String {
    format!(
        r#"{{"device":{}}}"#,
        serde_json::to_string(&device_id()).expect("device JSON serialization cannot fail")
    )
}

/// `{"device": ..., "characteristic": ...}` parameter JSON for the fixture.
pub(crate) fn characteristic_params() -> String {
    format!(
        r#"{{"device":{},"characteristic":{}}}"#,
        serde_json::to_string(&device_id()).expect("device JSON serialization cannot fail"),
        serde_json::to_string(&char_id()).expect("characteristic JSON serialization cannot fail")
    )
}

/// One callback delivery.
#[derive(Debug)]
pub(crate) enum Record {
    /// A completed request.
    Response {
        /// The caller-chosen correlation id.
        req_id: u64,
        /// The `status_json` string, null on success.
        status: Option<String>,
        /// The response payload bytes.
        payload: Vec<u8>,
    },
    /// One event envelope JSON string.
    Event(String),
}

/// Collects everything the facade delivers through its callbacks. Records
/// arrive interleaved (a `connect` response and its `DeviceConnected` event
/// race), so each type keeps its own queue and the awaiters skip the other
/// type instead of failing on it.
pub(crate) struct Recorder {
    rx: tokio::sync::mpsc::UnboundedReceiver<Record>,
    responses: std::collections::VecDeque<(u64, Option<String>, Vec<u8>)>,
    events: std::collections::VecDeque<String>,
}

impl Recorder {
    /// Registers recorder callbacks on both slots of `core` (which also
    /// starts the event pump).
    pub(crate) fn attach(core: &Core) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        core.set_response_callback(Some(on_response), leak_sender(tx.clone()), false);
        core.set_event_callback(Some(on_event), leak_sender(tx));
        Self {
            rx,
            responses: std::collections::VecDeque::new(),
            events: std::collections::VecDeque::new(),
        }
    }

    /// Registers recorder callbacks on a raw ABI handle; the `ffi` tests use
    /// this to exercise the real entry points.
    pub(crate) fn attach_to_handle(handle: *mut crate::RblueHandle) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let code =
            crate::rblue_set_response_callback(handle, Some(on_response), leak_sender(tx.clone()));
        assert_eq!(code, 0, "response callback registration must succeed");
        let code = crate::rblue_set_event_callback(handle, Some(on_event), leak_sender(tx));
        assert_eq!(code, 0, "event callback registration must succeed");
        Self {
            rx,
            responses: std::collections::VecDeque::new(),
            events: std::collections::VecDeque::new(),
        }
    }

    /// Awaits the next response record.
    ///
    /// # Panics
    ///
    /// When nothing arrives within [`RESPONSE_TIMEOUT`].
    pub(crate) async fn next_response(&mut self) -> (u64, Option<String>, Vec<u8>) {
        loop {
            if let Some(response) = self.responses.pop_front() {
                return response;
            }
            match tokio::time::timeout(RESPONSE_TIMEOUT, self.rx.recv()).await {
                Ok(Some(Record::Response {
                    req_id,
                    status,
                    payload,
                })) => return (req_id, status, payload),
                Ok(Some(Record::Event(json))) => self.events.push_back(json),
                Ok(None) => panic!("callback channel closed"),
                Err(_) => panic!("no response within {RESPONSE_TIMEOUT:?}"),
            }
        }
    }

    /// Awaits the next event envelope JSON.
    ///
    /// # Panics
    ///
    /// When nothing arrives within [`RESPONSE_TIMEOUT`].
    pub(crate) async fn next_event(&mut self) -> String {
        loop {
            if let Some(event) = self.events.pop_front() {
                return event;
            }
            match tokio::time::timeout(RESPONSE_TIMEOUT, self.rx.recv()).await {
                Ok(Some(Record::Event(json))) => return json,
                Ok(Some(Record::Response {
                    req_id,
                    status,
                    payload,
                })) => self.responses.push_back((req_id, status, payload)),
                Ok(None) => panic!("callback channel closed"),
                Err(_) => panic!("no event within {RESPONSE_TIMEOUT:?}"),
            }
        }
    }

    /// Returns every event already queued or delivered within [`ABSENCE_WINDOW`].
    ///
    /// Soak tests use this to separate events delivered before a notify flood
    /// (including the snapshot [`Self::drain_events`] queued in [`Fixture::new`])
    /// from events delivered afterwards, so shutdown totals can be checked
    /// against what the host actually received.
    pub(crate) async fn take_pending_events(&mut self) -> Vec<String> {
        self.drain_events().await;
        self.events.drain(..).collect()
    }

    /// Collects everything delivered within [`ABSENCE_WINDOW`] (e.g. the
    /// adapter's device snapshot right after the pump starts), queueing it by
    /// type for later awaiters.
    pub(crate) async fn drain_events(&mut self) {
        loop {
            match tokio::time::timeout(ABSENCE_WINDOW, self.rx.recv()).await {
                Ok(Some(Record::Event(json))) => self.events.push_back(json),
                Ok(Some(Record::Response {
                    req_id,
                    status,
                    payload,
                })) => self.responses.push_back((req_id, status, payload)),
                Ok(None) | Err(_) => break,
            }
        }
    }

    /// Asserts that no notification (and no response) arrives within
    /// [`ABSENCE_WINDOW`]. Late adapter lifecycle events are tolerated; they
    /// are queued for later awaiters.
    ///
    /// # Panics
    ///
    /// When a notification or response arrives.
    pub(crate) async fn expect_no_notification(&mut self) {
        loop {
            match tokio::time::timeout(ABSENCE_WINDOW, self.rx.recv()).await {
                Ok(Some(Record::Event(json))) => {
                    assert!(
                        !json.contains("\"type\":\"notification\""),
                        "unexpected notification: {json}"
                    );
                    self.events.push_back(json);
                }
                Ok(Some(record @ Record::Response { .. })) => {
                    panic!("unexpected response: {record:?}")
                }
                Ok(None) | Err(_) => break,
            }
        }
    }
}

/// Leaks a channel sender behind an opaque `user_data` pointer for callback
/// registration; test recorders live for the whole process.
fn leak_sender(tx: UnboundedSender<Record>) -> *mut c_void {
    Arc::into_raw(Arc::new(tx)).cast::<c_void>().cast_mut()
}

/// Response callback for [`Recorder`].
unsafe extern "C" fn on_response(
    req_id: u64,
    status_json: *const std::os::raw::c_char,
    payload: *const u8,
    payload_len: usize,
    user_data: *mut c_void,
) {
    let tx = unsafe { &*(user_data as *const UnboundedSender<Record>) };
    let status = if status_json.is_null() {
        None
    } else {
        // SAFETY: rblue-c hands out valid NUL-terminated strings it owns for
        // the duration of the call.
        Some(
            unsafe { CStr::from_ptr(status_json) }
                .to_string_lossy()
                .into_owned(),
        )
    };
    let payload = if payload_len == 0 {
        Vec::new()
    } else {
        // SAFETY: rblue-c guarantees the payload borrow is live for the
        // duration of the call.
        unsafe { std::slice::from_raw_parts(payload, payload_len) }.to_vec()
    };
    let _ = tx.send(Record::Response {
        req_id,
        status,
        payload,
    });
}

/// Event callback for [`Recorder`].
unsafe extern "C" fn on_event(event_json: *const std::os::raw::c_char, user_data: *mut c_void) {
    let tx = unsafe { &*(user_data as *const UnboundedSender<Record>) };
    // SAFETY: rblue-c hands out valid NUL-terminated strings it owns for the
    // duration of the call.
    let json = unsafe { CStr::from_ptr(event_json) }
        .to_string_lossy()
        .into_owned();
    let _ = tx.send(Record::Event(json));
}

/// A mock-backed facade with one scripted device and a recorder attached.
pub(crate) struct Fixture {
    /// The facade under test.
    pub core: Core,
    /// The scripted device (the mock's control plane).
    pub device: Arc<MockPeripheral>,
    /// The callback recorder.
    pub recorder: Recorder,
}

impl Fixture {
    /// Builds the fixture: one device with a readable, writable, echo-on-
    /// write, notifiable characteristic; visible to the adapter so
    /// `connect`-style flows resolve. Buffered snapshot events are drained
    /// before returning, so tests start from a quiet stream.
    pub(crate) async fn new() -> Self {
        let adapter = Arc::new(MockAdapter::new());
        let device = adapter.add_device(device_id(), "simulated-sensor");
        device.add_service(
            MockService::new(SERVICE_UUID)
                .characteristic(MockCharacteristic::new(CHAR_UUID, CHAR_PROPERTIES).with_echo()),
        );
        device
            .find_characteristic(CHAR_UUID)
            .expect("fixture characteristic");
        device.set_value(CHAR_UUID, vec![1, 2, 3]);
        device.advertise();
        let core = Core::with_adapter(Arc::clone(&adapter) as Arc<dyn Adapter>);
        let mut recorder = Recorder::attach(&core);
        // The event pump replays the known-device snapshot; swallow it.
        recorder.drain_events().await;
        Self {
            core,
            device,
            recorder,
        }
    }

    /// Sends a request through the ABI entry point's Rust-side path.
    pub(crate) fn request(&self, req_id: u64, method: &str, params: &str, payload: &[u8]) {
        self.core.spawn_request(
            req_id,
            crate::dispatch::Request {
                method: method.to_owned(),
                params: params.to_owned(),
                payload: payload.to_vec(),
            },
        );
    }
}
