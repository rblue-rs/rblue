//! Hardware-gated integration tests for the BlueZ backend.
//!
//! These drive the real `org.bluez` daemon over D-Bus: they expect a Linux
//! machine with `bluetoothd` running and actual Bluetooth hardware. They are
//! `#[ignore]`-gated and never run in CI; on a real Linux machine with
//! Bluetooth enabled, run:
//!
//! ```text
//! cargo test -p rblue-bluez -- --ignored --nocapture
//! ```

#![cfg(target_os = "linux")]

use std::time::Duration;

use futures::StreamExt;
use rblue_api::event::{AdapterEvent, AdapterState};
use rblue_bluez::{backend_version, default_adapter};

/// Initializes the real adapter and expects the radio to be usable. Fails
/// informatively when Bluetooth is powered off or bluetoothd is not running.
#[tokio::test]
#[ignore = "requires real Bluetooth hardware and a running bluetoothd"]
async fn initialize_resolves_adapter_state() {
    let adapter = default_adapter().expect("adapter constructed");
    adapter
        .initialize()
        .await
        .expect("Bluetooth powered on and bluetoothd running");
}

/// Scans for five seconds and reports how many devices were sighted. Not an
/// assertion on device count (the environment may be empty) but on the event
/// pipeline being live end to end. Note that BlueZ's persistent cache is
/// replayed as discoveries at initialize, so `sighted` includes stale cache
/// entries before any new advertisement arrives.
#[tokio::test]
#[ignore = "requires real Bluetooth hardware and a running bluetoothd"]
async fn scan_streams_sightings() {
    let adapter = default_adapter().expect("adapter constructed");
    adapter
        .initialize()
        .await
        .expect("Bluetooth powered on and bluetoothd running");
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
                // BlueZ identities are MACs, nothing else.
                assert!(matches!(device.id, rblue_api::DeviceId::Mac(..)));
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
    assert!(backend_version().contains("bluez/btleplug"));
}
