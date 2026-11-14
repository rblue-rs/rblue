//! Minimal in-process fake backend, used only to self-test the contract
//! suite until the real mock backend lands in `rblue-mock`. Safe to delete
//! once `rblue-mock` implements the same fixture.

use std::sync::{Arc, Mutex};

use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::stream::{BoxStream, StreamExt};

use crate::adapter::{Adapter, AdapterEventStream};
use crate::capability::CapabilitySet;
use crate::contract::{CONTRACT_CHARACTERISTIC_UUID, CONTRACT_SERVICE_UUID};
use crate::device::{AdapterId, AddressType, DeviceId};
use crate::error::Error;
use crate::event::{AdapterEvent, DisconnectReason};
use crate::gatt::{
    Characteristic, CharacteristicId, CharacteristicProperties, Notification, Service, ServiceSet,
    WriteMode,
};
use crate::peripheral::Peripheral;
use crate::scan::{AdvertisementData, DiscoveredDevice, RequestDeviceOptions, ScanFilter};

/// Fan-out hub: every subscriber gets an independent stream of future values.
#[derive(Debug)]
struct BroadcastHub<T> {
    subscribers: Mutex<Vec<UnboundedSender<T>>>,
}

impl<T> Default for BroadcastHub<T> {
    fn default() -> Self {
        Self {
            subscribers: Mutex::new(Vec::new()),
        }
    }
}

impl<T: Clone> BroadcastHub<T> {
    fn publish(&self, value: T) {
        let mut subscribers = self.lock();
        subscribers.retain(|tx| tx.unbounded_send(value.clone()).is_ok());
    }

    fn subscribe(&self) -> UnboundedReceiver<T> {
        let (tx, rx) = unbounded();
        self.lock().push(tx);
        rx
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<UnboundedSender<T>>> {
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Fake adapter exposing exactly one scripted peripheral that satisfies the
/// contract fixture: it advertises `CONTRACT_SERVICE_UUID` and its echo
/// characteristic reflects every write as a notification.
pub struct FakeAdapter {
    id: AdapterId,
    events: Arc<BroadcastHub<AdapterEvent>>,
    peripheral: Arc<FakePeripheral>,
}

impl FakeAdapter {
    /// Creates the fake with its scripted device. Adapter and peripheral
    /// share one event hub so connection events reach [`Adapter::events`].
    pub fn new() -> Self {
        let events = Arc::new(BroadcastHub::default());
        let peripheral = Arc::new(FakePeripheral::with_event_hub(Arc::clone(&events)));
        Self {
            id: AdapterId::default_adapter(),
            events,
            peripheral,
        }
    }
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Adapter for FakeAdapter {
    fn id(&self) -> &AdapterId {
        &self.id
    }

    fn capabilities(&self) -> CapabilitySet {
        CapabilitySet::bit(crate::Capability::ActiveScan)
            | CapabilitySet::bit(crate::Capability::GattClient)
            | CapabilitySet::bit(crate::Capability::RssiDuringConnection)
            | CapabilitySet::bit(crate::Capability::MtuQuery)
    }

    async fn initialize(&self) -> Result<(), Error> {
        Ok(())
    }

    async fn start_scan(&self, _filter: &ScanFilter) -> Result<(), Error> {
        self.events
            .publish(AdapterEvent::DeviceDiscovered(DiscoveredDevice {
                id: self.peripheral.id.clone(),
                advertisement: AdvertisementData {
                    local_name: Some("rblue-fake".to_owned()),
                    services: vec![CONTRACT_SERVICE_UUID],
                    ..AdvertisementData::default()
                },
            }));
        Ok(())
    }

    async fn stop_scan(&self) -> Result<(), Error> {
        Ok(())
    }

    fn events(&self) -> AdapterEventStream {
        self.events.subscribe().boxed()
    }

    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        Ok(vec![self.peripheral.clone()])
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        if *id == self.peripheral.id {
            Ok(self.peripheral.clone())
        } else {
            Err(Error::NotFound(id.clone()))
        }
    }

    async fn request_device(
        &self,
        _opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error> {
        Err(Error::Unsupported {
            platform: "fake",
            feature: "request_device",
        })
    }
}

#[derive(Debug)]
struct FakePeripheral {
    id: DeviceId,
    connected: Mutex<bool>,
    notifications: BroadcastHub<Notification>,
    events: Arc<BroadcastHub<AdapterEvent>>,
}

impl FakePeripheral {
    fn with_event_hub(events: Arc<BroadcastHub<AdapterEvent>>) -> Self {
        Self {
            id: DeviceId::Mac(
                [0x02, 0x00, 0x00, 0xCA, 0xFE, 0x01],
                AddressType::RandomStatic,
            ),
            connected: Mutex::new(false),
            notifications: BroadcastHub::default(),
            events,
        }
    }

    fn lock_connected(&self) -> std::sync::MutexGuard<'_, bool> {
        self.connected
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait::async_trait]
impl Peripheral for FakePeripheral {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn name(&self) -> Option<String> {
        Some("rblue-fake".to_owned())
    }

    async fn is_connected(&self) -> Result<bool, Error> {
        Ok(*self.lock_connected())
    }

    async fn connect(&self) -> Result<(), Error> {
        *self.lock_connected() = true;
        self.events
            .publish(AdapterEvent::DeviceConnected(self.id.clone()));
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), Error> {
        *self.lock_connected() = false;
        self.events.publish(AdapterEvent::DeviceDisconnected(
            self.id.clone(),
            Some(DisconnectReason::Local),
        ));
        Ok(())
    }

    async fn discover_services(&self, _filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
        Ok(ServiceSet {
            services: vec![Service {
                uuid: CONTRACT_SERVICE_UUID,
                primary: true,
                characteristics: vec![Characteristic {
                    uuid: CONTRACT_CHARACTERISTIC_UUID,
                    properties: CharacteristicProperties::READ
                        | CharacteristicProperties::WRITE
                        | CharacteristicProperties::WRITE_WITHOUT_RESPONSE
                        | CharacteristicProperties::NOTIFY,
                    descriptors: Vec::new(),
                }],
            }],
        })
    }

    async fn read(&self, _ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
        Ok(b"rblue-fake-value".to_vec())
    }

    async fn write(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        _mode: WriteMode,
    ) -> Result<(), Error> {
        const FAKE_MAX_WRITE_LEN: usize = 20;
        if data.len() > FAKE_MAX_WRITE_LEN {
            return Err(Error::InvalidLength {
                len: data.len(),
                max: FAKE_MAX_WRITE_LEN,
            });
        }
        // Echo behavior: every written value comes back as a notification.
        self.notifications.publish(Notification {
            id: *ch,
            data: data.to_vec(),
        });
        Ok(())
    }

    async fn max_write_len(
        &self,
        _ch: &CharacteristicId,
        _mode: WriteMode,
    ) -> Result<usize, Error> {
        Ok(20)
    }

    async fn subscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
        Ok(())
    }

    async fn unsubscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
        Ok(())
    }

    fn notifications(&self) -> BoxStream<'static, Notification> {
        self.notifications.subscribe().boxed()
    }

    async fn pair(&self) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: "fake",
            feature: "pairing",
        })
    }

    async fn rssi(&self) -> Result<i16, Error> {
        Ok(-40)
    }

    async fn mtu(&self) -> Result<u16, Error> {
        Ok(23)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gatt::ServiceId;

    #[test]
    fn fake_write_rejects_oversize() {
        let peripheral = FakePeripheral::with_event_hub(Arc::new(BroadcastHub::default()));
        let ch = CharacteristicId {
            service: ServiceId {
                uuid: CONTRACT_SERVICE_UUID,
                instance: 0,
            },
            uuid: CONTRACT_CHARACTERISTIC_UUID,
            instance: 0,
        };
        let data = vec![0u8; 21];
        let result =
            futures::executor::block_on(peripheral.write(&ch, &data, WriteMode::WithResponse));
        assert!(matches!(
            result,
            Err(Error::InvalidLength { len: 21, max: 20 })
        ));
    }
}
