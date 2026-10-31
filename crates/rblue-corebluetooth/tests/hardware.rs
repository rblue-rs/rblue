//! Hardware-gated integration tests for the CoreBluetooth backend.
//!
//! These drive the real `CBCentralManager`: they trigger the authorization
//! prompt on macOS and expect actual Bluetooth hardware. They are
//! `#[ignore]`-gated and never run in CI; on a real Mac with Bluetooth
//! enabled, run:
//!
//! ```text
//! cargo test -p rblue-corebluetooth -- --ignored --nocapture
//! ```

use std::time::Duration;

use futures::StreamExt;
use rblue_api::event::{AdapterEvent, AdapterState};
use rblue_corebluetooth::{backend_version, default_adapter};

/// Initializes the real adapter and expects the radio to be usable. Fails
/// informatively when Bluetooth is off or authorization was denied.
#[tokio::test]
#[ignore = "requires real Bluetooth hardware and user authorization"]
async fn initialize_resolves_adapter_state() {
    let adapter = default_adapter().expect("adapter constructed");
    adapter
        .initialize()
        .await
        .expect("Bluetooth powered on and authorized");
}

/// Scans for five seconds and reports how many devices were sighted. Not an
/// assertion on device count (the environment may be empty) but on the event
/// pipeline being live end to end.
#[tokio::test]
#[ignore = "requires real Bluetooth hardware and user authorization"]
async fn scan_streams_sightings() {
    let adapter = default_adapter().expect("adapter constructed");
    adapter
        .initialize()
        .await
        .expect("Bluetooth powered on and authorized");
    assert!(adapter
        .capabilities()
        .has(rblue_api::Capability::ActiveScan));

    let mut events = adapter.events();
    adapter
        .start_scan(&rblue_api::ScanFilter::default())
        .await
        .expect("scan started");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut sighted = 0usize;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(AdapterEvent::DeviceDiscovered(device))) => {
                sighted += 1;
                // CoreBluetooth identities are UUIDs, nothing else.
                assert!(matches!(device.id, rblue_api::DeviceId::Uuid(_)));
            }
            Ok(Some(AdapterEvent::StateChanged(AdapterState::PoweredOn))) => {}
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => break,
        }
    }
    adapter.stop_scan().await.expect("scan stopped");
    println!("sighted {sighted} devices in 5 seconds");
}

#[test]
fn backend_version_is_reported() {
    assert!(backend_version().contains("corebluetooth/btleplug"));
}
