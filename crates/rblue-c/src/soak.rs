//! Longer soak coverage for the C ABI: out-of-order completions, notification
//! backlogs, and shutdown while requests are still in flight. All of this
//! uses the in-process mock backend; no Bluetooth hardware is required.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::StreamExt;
use rblue::prelude::*;
use tokio::sync::Notify;

use crate::dispatch::Request;
use crate::handle::Core;
use crate::testing::{
    characteristic_params, device_params, Fixture, Recorder, ABSENCE_WINDOW, CHAR_UUID,
};

const OUT_OF_ORDER_BURST: u64 = 64;
const NOTIFY_BACKLOG: usize = 1_000;
const IN_FLIGHT_GATED: u64 = 40;
const IN_FLIGHT_FAST: u64 = 40;

/// Waits for `count` responses and asserts each `req_id` arrives exactly once.
async fn collect_unique_responses(recorder: &mut Recorder, count: u64) -> Vec<u64> {
    let mut seen = Vec::with_capacity(count as usize);
    let mut unique = std::collections::HashSet::new();
    for _ in 0..count {
        let (req_id, status, _) = recorder.next_response().await;
        assert!(
            status.is_none(),
            "request {req_id} failed during soak: {status:?}"
        );
        assert!(
            unique.insert(req_id),
            "request {req_id} completed more than once"
        );
        seen.push(req_id);
    }
    seen
}

#[tokio::test]
async fn out_of_order_burst_is_correlated_by_req_id_exactly_once() {
    let adapter = Arc::new(GatedAdapter::new());
    let core = Core::with_adapter(Arc::clone(&adapter) as Arc<dyn Adapter>);
    let mut recorder = Recorder::attach(&core);

    for req_id in 1..=OUT_OF_ORDER_BURST {
        core.spawn_request(
            req_id,
            Request {
                method: "initialize".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
        let fast_id = req_id + OUT_OF_ORDER_BURST;
        core.spawn_request(
            fast_id,
            Request {
                method: "backend_version".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
    }

    // Every `backend_version` must finish before the gated `initialize` calls.
    let first = collect_unique_responses(&mut recorder, OUT_OF_ORDER_BURST).await;
    for req_id in &first {
        assert!(
            *req_id > OUT_OF_ORDER_BURST,
            "gated initialize {req_id} completed before the gate opened"
        );
    }

    adapter.open();
    let second = collect_unique_responses(&mut recorder, OUT_OF_ORDER_BURST).await;
    let mut all: std::collections::HashSet<u64> = first.into_iter().chain(second).collect();
    assert_eq!(all.len(), (OUT_OF_ORDER_BURST * 2) as usize);
    for req_id in 1..=OUT_OF_ORDER_BURST * 2 {
        assert!(all.remove(&req_id), "missing response for req_id {req_id}");
    }

    let (responses, events) = core.shutdown();
    assert_eq!(responses, OUT_OF_ORDER_BURST * 2);
    assert_eq!(events, 0);
}

#[tokio::test]
async fn notification_backlog_arrives_in_order_and_drains_on_shutdown() {
    let mut fixture = Fixture::new().await;
    fixture.request(1, "connect", &device_params(), &[]);
    let (req_id, status, _) = fixture.recorder.next_response().await;
    assert_eq!(req_id, 1);
    assert!(status.is_none());
    fixture.request(2, "subscribe", &characteristic_params(), &[]);
    let (req_id, status, _) = fixture.recorder.next_response().await;
    assert_eq!(req_id, 2);
    assert!(status.is_none());

    for index in 0..NOTIFY_BACKLOG {
        fixture
            .device
            .push_notify(CHAR_UUID, &(index as u32).to_be_bytes());
    }

    let mut payloads = Vec::with_capacity(NOTIFY_BACKLOG);
    while payloads.len() < NOTIFY_BACKLOG {
        let json = fixture.recorder.next_event().await;
        if let Some(data) = notification_payload(&json) {
            payloads.push(data);
        }
    }
    for (index, data) in payloads.iter().enumerate() {
        assert_eq!(
            data,
            &(index as u32).to_be_bytes(),
            "notification {index} arrived out of order or corrupted"
        );
    }

    let (responses, events) = fixture.core.shutdown();
    assert_eq!(responses, 2);
    assert!(
        events as usize >= NOTIFY_BACKLOG,
        "shutdown under-counted the notification backlog: {events}"
    );
}

#[tokio::test]
async fn shutdown_cancels_in_flight_requests_and_fires_no_callbacks_after_free() {
    let adapter = Arc::new(GatedAdapter::new());
    let core = Core::with_adapter(Arc::clone(&adapter) as Arc<dyn Adapter>);
    let mut recorder = Recorder::attach(&core);

    for req_id in 1..=IN_FLIGHT_GATED {
        core.spawn_request(
            req_id,
            Request {
                method: "initialize".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
    }
    for req_id in (IN_FLIGHT_GATED + 1)..=(IN_FLIGHT_GATED + IN_FLIGHT_FAST) {
        core.spawn_request(
            req_id,
            Request {
                method: "backend_version".to_owned(),
                params: "{}".to_owned(),
                payload: Vec::new(),
            },
        );
    }

    let finished = collect_unique_responses(&mut recorder, IN_FLIGHT_FAST).await;
    assert_eq!(finished.len(), IN_FLIGHT_FAST as usize);
    for req_id in &finished {
        assert!(*req_id > IN_FLIGHT_GATED);
    }

    let (responses, events) = core.shutdown();
    assert_eq!(
        responses, IN_FLIGHT_FAST,
        "in-flight gated requests must not deliver after shutdown"
    );
    assert_eq!(events, 0);

    // Opening the gate after shutdown must not produce late responses.
    adapter.open();
    tokio::time::sleep(ABSENCE_WINDOW).await;
    recorder.expect_no_notification().await;

    drop(core);
    tokio::time::sleep(Duration::from_millis(50)).await;
    recorder.expect_no_notification().await;
}

#[tokio::test]
async fn shutdown_during_notification_backlog_matches_delivered_counts() {
    let mut fixture = Fixture::new().await;
    fixture.request(1, "connect", &device_params(), &[]);
    let (_, status, _) = fixture.recorder.next_response().await;
    assert!(status.is_none());
    fixture.request(2, "subscribe", &characteristic_params(), &[]);
    let (_, status, _) = fixture.recorder.next_response().await;
    assert!(status.is_none());

    // Snapshot events Fixture::new queued, plus connect/subscribe lifecycle
    // events, are already in Core's delivered counts. Pull them off the
    // recorder now so the shutdown total can be checked against pre + post.
    let pre = fixture.recorder.take_pending_events().await.len() as u64;

    for index in 0..NOTIFY_BACKLOG {
        fixture
            .device
            .push_notify(CHAR_UUID, &(index as u32).to_be_bytes());
    }

    // Wait for at least one notification so shutdown races a live backlog
    // rather than cancelling an empty pump. Late lifecycle events that slip
    // through after the pre-drain still count toward the shutdown total.
    let mut post = Vec::new();
    loop {
        let json = fixture.recorder.next_event().await;
        let is_notification = notification_payload(&json).is_some();
        post.push(json);
        if is_notification {
            break;
        }
    }

    let (responses, events) = fixture.core.shutdown();
    assert_eq!(responses, 2);

    post.extend(fixture.recorder.take_pending_events().await);
    assert_eq!(
        pre + post.len() as u64,
        events,
        "shutdown event count must equal events the recorder received (pre={pre}, post={}, shutdown={events})",
        post.len()
    );
    recorder_expect_silence(&mut fixture.recorder).await;
}

fn notification_payload(json: &str) -> Option<Vec<u8>> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if value.get("type")?.as_str()? != "notification" {
        return None;
    }
    let data = value.get("notification")?.get("data")?.as_array()?;
    Some(
        data.iter()
            .map(|byte| u8::try_from(byte.as_u64()?).ok())
            .collect::<Option<Vec<_>>>()?,
    )
}

async fn recorder_expect_silence(recorder: &mut Recorder) {
    // Reuse the absence window: notifications and responses are forbidden;
    // late adapter lifecycle events are tolerated by the helper.
    recorder.expect_no_notification().await;
}

/// Adapter whose `initialize` blocks until [`GatedAdapter::open`] is called.
struct GatedAdapter {
    gate: Arc<Gate>,
    id: AdapterId,
}

struct Gate {
    opened: std::sync::atomic::AtomicBool,
    notify: Notify,
}

impl GatedAdapter {
    fn new() -> Self {
        Self {
            gate: Arc::new(Gate {
                opened: std::sync::atomic::AtomicBool::new(false),
                notify: Notify::new(),
            }),
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
