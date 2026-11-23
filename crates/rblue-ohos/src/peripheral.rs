//! [`OhosPeripheral`]: the GATT client session with one device, speaking the
//! `BleDriver` protocol through the registry.

use std::collections::HashSet;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

use async_trait::async_trait;
use rblue_api::{
    AdvertisementData, AttError, CharacteristicId, CharacteristicProperties, ConnectOptions,
    DeviceId, Error, Notification, NotificationStream, Peripheral, ServiceSet, WriteMode,
};
use rblue_driver::{BleDriverCommand, DriverValue};

use crate::broadcast::BoundedBroadcast;
use crate::registry::{expect_data, expect_none, expect_services, RegistryInner, DEFAULT_ATT_MTU};

/// ATT error code the unified model reports when the write mode does not
/// match the characteristic properties (write not permitted).
pub(crate) const ATT_WRITE_NOT_PERMITTED: u8 = 0x03;

/// GATT client session with one OHOS peripheral.
///
/// Handles live in the registry's device table (so events can route
/// notifications into them) and are handed out as `Arc<OhosPeripheral>`; the
/// registry itself is referenced weakly, avoiding a reference cycle. All
/// session state - services, connection flag, negotiated MTU - is
/// per-connection and cleared on disconnect, per the session-scoped
/// identifier rule.
pub struct OhosPeripheral {
    core: Weak<RegistryInner>,
    id: DeviceId,
    shared: RwLock<PeripheralShared>,
}

#[derive(Debug)]
struct PeripheralShared {
    advertisement: AdvertisementData,
    connected: bool,
    services: Option<ServiceSet>,
    mtu: u16,
    /// Characteristics with an active CCCD subscription on this connection.
    /// Cleared on disconnect: the peer stops sending and the platform drops
    /// its CCCD state, so notifications must not resume until `subscribe`
    /// runs again on the fresh connection.
    subscriptions: HashSet<CharacteristicId>,
    notifications: BoundedBroadcast<Notification>,
}

impl OhosPeripheral {
    pub(crate) fn new(
        core: &Arc<RegistryInner>,
        id: DeviceId,
        advertisement: AdvertisementData,
    ) -> Arc<Self> {
        let notifications =
            BoundedBroadcast::new(core.notification_capacity(), drop_oldest_notification);
        Arc::new(Self {
            core: Arc::downgrade(core),
            id,
            shared: RwLock::new(PeripheralShared {
                advertisement,
                connected: false,
                services: None,
                mtu: DEFAULT_ATT_MTU,
                subscriptions: HashSet::new(),
                notifications,
            }),
        })
    }

    /// Identity of the peripheral.
    #[must_use]
    pub fn id(&self) -> &DeviceId {
        &self.id
    }

    /// Latest stored advertisement (used for the event-stream snapshot).
    pub(crate) fn advertisement(&self) -> AdvertisementData {
        self.lock_shared().advertisement.clone()
    }

    /// Advertised name from the latest sighting.
    #[must_use]
    pub fn cached_name(&self) -> Option<String> {
        self.lock_shared().advertisement.local_name.clone()
    }

    pub(crate) fn update_advertisement(&self, advertisement: &AdvertisementData) {
        self.lock_shared_mut().advertisement = advertisement.clone();
    }

    pub(crate) fn update_rssi(&self, rssi: i16) {
        self.lock_shared_mut().advertisement.rssi = Some(rssi);
    }

    pub(crate) fn set_connected(&self, connected: bool) {
        let mut shared = self.lock_shared_mut();
        shared.connected = connected;
        if !connected {
            // Session-scoped state dies with the connection: identifiers and
            // the negotiated MTU are invalid after a reconnect, and the CCCD
            // subscriptions are gone (the peer stops sending).
            shared.services = None;
            shared.mtu = DEFAULT_ATT_MTU;
            shared.subscriptions.clear();
        }
    }

    pub(crate) fn set_services(&self, services: ServiceSet) {
        self.lock_shared_mut().services = Some(services);
    }

    pub(crate) fn set_mtu(&self, mtu: u16) {
        // The ATT MTU has a floor of 23; clamp lower driver reports.
        self.lock_shared_mut().mtu = mtu.max(DEFAULT_ATT_MTU);
    }

    /// Delivers a notification to subscribers. Values for characteristics
    /// without an active subscription on this connection are dropped: the
    /// trait guarantees delivery only after `subscribe` (CCCD semantics).
    pub(crate) fn push_notification(&self, notification: Notification) {
        let shared = self.lock_shared();
        if shared.subscriptions.contains(&notification.id) {
            shared.notifications.push(&notification);
        }
    }

    /// Current `max_write_len`: the ATT MTU minus the 3-byte ATT header, for
    /// both write modes (caller has done the connection check).
    fn lock_shared_mtu(&self) -> Result<usize, Error> {
        let shared = self.lock_shared();
        Ok(usize::from(shared.mtu.saturating_sub(3)))
    }

    /// Properties of a characteristic in the current session's discovery
    /// tree; the basis for the notify-vs-indicate choice.
    fn characteristic_properties(
        &self,
        ch: &CharacteristicId,
    ) -> Result<CharacteristicProperties, Error> {
        let shared = self.lock_shared();
        let services = shared.services.as_ref().ok_or_else(|| {
            Error::InvalidState("services not discovered on this connection".to_owned())
        })?;
        let service = services
            .services
            .iter()
            .filter(|service| service.uuid == ch.service.uuid)
            .nth(usize::from(ch.service.instance))
            .ok_or_else(|| {
                Error::InvalidState(format!(
                    "service {} instance {} not in session tree",
                    ch.service.uuid, ch.service.instance
                ))
            })?;
        service
            .characteristics
            .iter()
            .filter(|characteristic| characteristic.uuid == ch.uuid)
            .nth(usize::from(ch.instance))
            .map(|characteristic| characteristic.properties)
            .ok_or_else(|| {
                Error::InvalidState(format!(
                    "characteristic {} instance {} not in session tree",
                    ch.uuid, ch.instance
                ))
            })
    }

    fn lock_shared(&self) -> RwLockReadGuard<'_, PeripheralShared> {
        self.shared
            .read()
            .expect("peripheral state poisoned by a panicking operation")
    }

    fn lock_shared_mut(&self) -> RwLockWriteGuard<'_, PeripheralShared> {
        self.shared
            .write()
            .expect("peripheral state poisoned by a panicking operation")
    }

    fn core(&self) -> Result<Arc<RegistryInner>, Error> {
        self.core.upgrade().ok_or_else(|| {
            Error::Backend("driver registry dropped; peripheral handle is stale".to_owned())
        })
    }

    /// Unified connection pre-check shared by every GATT operation.
    fn require_connected(&self) -> Result<(), Error> {
        if self.lock_shared().connected {
            Ok(())
        } else {
            Err(Error::NotConnected)
        }
    }

    async fn request(&self, command: BleDriverCommand) -> Result<DriverValue, Error> {
        self.core()?.request(command).await
    }

    async fn request_with_payload(
        &self,
        command: BleDriverCommand,
        payload: &[u8],
    ) -> Result<DriverValue, Error> {
        self.core()?.request_with_payload(command, payload).await
    }
}

fn drop_oldest_notification(_: &Notification) -> bool {
    true
}

#[async_trait]
impl Peripheral for OhosPeripheral {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn name(&self) -> Option<String> {
        self.lock_shared().advertisement.local_name.clone()
    }

    async fn is_connected(&self) -> Result<bool, Error> {
        Ok(self.lock_shared().connected)
    }

    async fn connect(&self) -> Result<(), Error> {
        expect_none(
            self.request(BleDriverCommand::Connect {
                device: self.id.clone(),
            })
            .await?,
        )?;
        self.set_connected(true);
        Ok(())
    }

    async fn connect_with(&self, opts: &ConnectOptions) -> Result<(), Error> {
        if opts.auto_connect {
            return Err(Error::Unsupported {
                platform: "ohos",
                feature: "auto_connect",
            });
        }
        if let Some(mtu) = opts.mtu {
            rblue_api::reject_invalid_att_mtu(mtu)?;
        }
        self.connect().await?;
        if let Some(mtu) = opts.mtu {
            self.request_mtu(mtu).await?;
        }
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), Error> {
        // Idempotent: an already-down link has nothing to tear down, so the
        // command is not dispatched (the driver rejects it with a platform
        // code that would only confuse the idempotent contract).
        if !self.lock_shared().connected {
            return Ok(());
        }
        expect_none(
            self.request(BleDriverCommand::Disconnect {
                device: self.id.clone(),
            })
            .await?,
        )?;
        self.set_connected(false);
        Ok(())
    }

    async fn discover_services(&self, filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
        self.require_connected()?;
        let value = self
            .request(BleDriverCommand::DiscoverServices {
                device: self.id.clone(),
            })
            .await?;
        let mut set = expect_services(value)?;
        if !filter.is_empty() {
            set.services
                .retain(|service| filter.contains(&service.uuid));
        }
        self.set_services(set.clone());
        Ok(set)
    }

    async fn read(&self, ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
        self.require_connected()?;
        let value = self
            .request(BleDriverCommand::Read {
                device: self.id.clone(),
                characteristic: *ch,
            })
            .await?;
        expect_data(value)
    }

    async fn write(
        &self,
        ch: &CharacteristicId,
        data: &[u8],
        mode: WriteMode,
    ) -> Result<(), Error> {
        // Unified write semantics, matching the other backends: connection
        // check, then the property-bit check (mode mismatch is ATT 0x03), then
        // the payload-length check with max exactly `max_write_len` - only
        // then does anything reach the driver.
        self.require_connected()?;
        let properties = self.characteristic_properties(ch)?;
        let required = match mode {
            WriteMode::WithResponse => CharacteristicProperties::WRITE,
            WriteMode::WithoutResponse => CharacteristicProperties::WRITE_WITHOUT_RESPONSE,
        };
        if !properties.contains(required) {
            return Err(Error::Att(AttError {
                code: ATT_WRITE_NOT_PERMITTED,
                handle: None,
            }));
        }
        let max = self.lock_shared_mtu()?;
        if data.len() > max {
            return Err(Error::InvalidLength {
                len: data.len(),
                max,
            });
        }
        // The bytes travel on the binary data plane, not through the JSON
        // command.
        expect_none(
            self.request_with_payload(
                BleDriverCommand::Write {
                    device: self.id.clone(),
                    characteristic: *ch,
                    with_response: mode.with_response(),
                },
                data,
            )
            .await?,
        )
    }

    async fn max_write_len(
        &self,
        _ch: &CharacteristicId,
        _mode: WriteMode,
    ) -> Result<usize, Error> {
        self.require_connected()?;
        self.lock_shared_mtu()
    }

    async fn subscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        // OHOS exposes distinct APIs for notification and indication; the
        // ArkTS driver picks between them from the properties, and the core
        // rejects characteristics that support neither.
        self.require_connected()?;
        let properties = self.characteristic_properties(ch)?;
        let supports_changes = properties
            .intersects(CharacteristicProperties::NOTIFY | CharacteristicProperties::INDICATE);
        if !supports_changes {
            return Err(Error::InvalidState(format!(
                "characteristic {} supports neither notify nor indicate",
                ch.uuid
            )));
        }
        expect_none(
            self.request(BleDriverCommand::Subscribe {
                device: self.id.clone(),
                characteristic: *ch,
            })
            .await?,
        )?;
        let mut shared = self.lock_shared_mut();
        shared.subscriptions.insert(*ch);
        Ok(())
    }

    async fn unsubscribe(&self, ch: &CharacteristicId) -> Result<(), Error> {
        self.require_connected()?;
        expect_none(
            self.request(BleDriverCommand::Unsubscribe {
                device: self.id.clone(),
                characteristic: *ch,
            })
            .await?,
        )?;
        // Only a confirmed unsubscribe stops the gate: while the CCCD is
        // still set on the peer, values keep flowing and must be delivered.
        self.lock_shared_mut().subscriptions.remove(ch);
        Ok(())
    }

    fn notifications(&self) -> NotificationStream {
        let subscription = self.lock_shared().notifications.subscribe();
        Box::pin(subscription)
    }

    async fn pair(&self) -> Result<(), Error> {
        // Phase 2: OHOS pairs implicitly when an encrypted characteristic is
        // accessed; the 1.0 protocol has no initiate-pairing command.
        Err(Error::Unsupported {
            platform: "ohos",
            feature: "pair",
        })
    }

    async fn unpair(&self) -> Result<(), Error> {
        Err(Error::Unsupported {
            platform: "ohos",
            feature: "unpair",
        })
    }

    async fn rssi(&self) -> Result<i16, Error> {
        // `getRssiValue` is pending real-device verification, so
        // `Capability::RssiDuringConnection` is not reported; the trait must
        // not pretend it works. The raw protocol path
        // (`registry.request(BleDriverCommand::ReadRssi)`) remains available
        // for the M4 round-trip PoC.
        Err(Error::Unsupported {
            platform: "ohos",
            feature: "read_rssi",
        })
    }

    async fn mtu(&self) -> Result<u16, Error> {
        let shared = self.lock_shared();
        if !shared.connected {
            return Err(Error::NotConnected);
        }
        Ok(shared.mtu)
    }

    async fn request_mtu(&self, mtu: u16) -> Result<u16, Error> {
        self.require_connected()?;
        rblue_api::reject_invalid_att_mtu(mtu)?;
        let value = self
            .request(BleDriverCommand::RequestMtu {
                device: self.id.clone(),
                mtu,
            })
            .await?;
        match value {
            DriverValue::Mtu { mtu } => Ok(mtu),
            other => Err(Error::Backend(format!(
                "expected mtu response, got {other:?}"
            ))),
        }
    }
}
