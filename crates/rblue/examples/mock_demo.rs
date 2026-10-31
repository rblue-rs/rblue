//! End-to-end smoke demo of the rblue surface against the in-process mock
//! backend: scan, connect, discover, read, write (echoed back as a
//! notification), and event handling - all without Bluetooth hardware.
//!
//! Run with:
//!
//! ```text
//! cargo run -p rblue --example mock_demo
//! ```

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use rblue::prelude::*;
use rblue_mock::{MockAdapter, MockCharacteristic, MockService};
use uuid::uuid;

const SERVICE_UUID: uuid::Uuid = uuid!("0000180f-0000-1000-8000-00805f9b34fb");
const CHARACTERISTIC_UUID: uuid::Uuid = uuid!("00002a19-0000-1000-8000-00805f9b34fb");
const DEVICE_ID: DeviceId = DeviceId::Mac(
    [0x02, 0x5a, 0x01, 0x02, 0x03, 0x04],
    AddressType::RandomStatic,
);

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mock = Arc::new(MockAdapter::new());

    // A simulated peripheral with one service: a readable, writable
    // characteristic whose accepted writes are echoed back as notifications.
    let simulated = mock.add_device(DEVICE_ID, "Demo Sensor");
    simulated.add_service(
        MockService::new(SERVICE_UUID).characteristic(
            MockCharacteristic::new(
                CHARACTERISTIC_UUID,
                CharacteristicProperties::READ
                    | CharacteristicProperties::WRITE
                    | CharacteristicProperties::WRITE_WITHOUT_RESPONSE
                    | CharacteristicProperties::NOTIFY,
            )
            .with_initial_value(b"boot".to_vec())
            .with_echo(),
        ),
    );
    simulated.advertise();

    let adapter: Arc<dyn Adapter> = mock;

    // Subscribing replays devices already seen, so discovery shows up even
    // though it happened before the scan started.
    let mut events = adapter.events();
    adapter.initialize().await?;
    adapter.start_scan(&ScanFilter::default()).await?;

    match events.next().await {
        Some(AdapterEvent::DeviceDiscovered(device)) => {
            println!(
                "discovered: {} ({:?})",
                device.id, device.advertisement.rssi
            );
        }
        Some(other) => println!("event while scanning: {other:?}"),
        None => return Err(Error::InvalidState("event stream ended".into())),
    }

    let peripheral = adapter.peripheral(&DEVICE_ID).await?;
    peripheral.connect().await?;
    println!("connected; adapter version {}", rblue::backend_version());

    let services = peripheral.discover_services(&[]).await?;
    let service = services
        .services
        .iter()
        .find(|service| service.uuid == SERVICE_UUID)
        .ok_or_else(|| Error::InvalidState("service missing".into()))?;
    if !service
        .characteristics
        .iter()
        .any(|characteristic| characteristic.uuid == CHARACTERISTIC_UUID)
    {
        return Err(Error::InvalidState("characteristic missing".into()));
    }
    // Characteristic ids are session-scoped coordinates into the discovered
    // tree, rebuilt here from the discovery result.
    let characteristic = CharacteristicId {
        service: ServiceId {
            uuid: SERVICE_UUID,
            instance: 0,
        },
        uuid: CHARACTERISTIC_UUID,
        instance: 0,
    };
    println!("discovered {} service(s)", services.services.len());

    let initial = peripheral.read(&characteristic).await?;
    println!("read: {:?}", String::from_utf8_lossy(&initial));

    peripheral.subscribe(&characteristic).await?;
    let mut notifications = peripheral.notifications();

    peripheral
        .write(&characteristic, b"ping", WriteMode::WithResponse)
        .await?;
    let echo = tokio::time::timeout(Duration::from_secs(1), notifications.next())
        .await
        .map_err(|_| Error::Timeout)?
        .ok_or_else(|| Error::InvalidState("notification stream ended".into()))?;
    println!(
        "wrote b\"ping\"; echoed back: {:?}",
        String::from_utf8_lossy(&echo.data)
    );

    peripheral.unsubscribe(&characteristic).await?;
    peripheral.disconnect().await?;
    adapter.stop_scan().await?;
    println!("done");
    Ok(())
}
