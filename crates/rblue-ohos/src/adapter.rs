//! [`OhosAdapter`]: the central-role adapter over the driver registry.

use std::sync::Arc;

use async_trait::async_trait;
use rblue_api::{
    Adapter, AdapterEventStream, AdapterId, CapabilitySet, DeviceId, Error, Peripheral,
    RequestDeviceOptions, ScanFilter,
};
use rblue_driver::{BleDriverCommand, DriverValue};

use crate::capabilities::ohos_capabilities;
use crate::registry::{expect_adapter_state, expect_none, DriverRegistry, RegistryInner};

/// Central-role adapter for OHOS, driving an injected ArkTS `BleDriver`
/// through a [`DriverRegistry`].
///
/// All adapters over one registry share the registry's device table, event
/// broadcast, and pending-request table.
pub struct OhosAdapter {
    core: Arc<RegistryInner>,
    id: AdapterId,
}

impl OhosAdapter {
    /// Wraps a registry (usually built once per process by the napi facade).
    #[must_use]
    pub fn new(registry: &DriverRegistry) -> Self {
        Self {
            core: Arc::clone(registry.inner()),
            id: AdapterId::default_adapter(),
        }
    }

    /// Registry backing this adapter.
    #[must_use]
    pub fn registry(&self) -> DriverRegistry {
        DriverRegistry::from_inner(Arc::clone(&self.core))
    }

    async fn request(&self, command: BleDriverCommand) -> Result<DriverValue, Error> {
        self.core.request(command).await
    }
}

#[async_trait]
impl Adapter for OhosAdapter {
    fn id(&self) -> &AdapterId {
        &self.id
    }

    fn capabilities(&self) -> CapabilitySet {
        ohos_capabilities()
    }

    async fn initialize(&self) -> Result<(), Error> {
        // Init completes first (the ArkTS driver may prompt for
        // ohos.permission.ACCESS_BLUETOOTH here), then the adapter state is
        // learned from the driver, whose source is `@ohos.bluetooth.access`,
        // not the `ble` module.
        expect_none(self.request(BleDriverCommand::Init).await?)?;
        let state = expect_adapter_state(self.request(BleDriverCommand::GetAdapterState).await?)?;
        self.core.set_adapter_state(state);
        Ok(())
    }

    async fn start_scan(&self, filter: &ScanFilter) -> Result<(), Error> {
        self.core.set_scan_filter(Some(filter.clone()));
        expect_none(
            self.request(BleDriverCommand::StartScan {
                filter: filter.clone(),
            })
            .await?,
        )?;
        self.core.set_scanning(true);
        Ok(())
    }

    async fn stop_scan(&self) -> Result<(), Error> {
        if !self.core.is_scanning() {
            return Err(Error::InvalidState("no scan is running".to_owned()));
        }
        expect_none(self.request(BleDriverCommand::StopScan).await?)?;
        self.core.set_scanning(false);
        self.core.set_scan_filter(None);
        Ok(())
    }

    fn events(&self) -> AdapterEventStream {
        self.core.subscribe_events()
    }

    async fn peripherals(&self) -> Result<Vec<Arc<dyn Peripheral>>, Error> {
        Ok(self.core.known_peripherals())
    }

    async fn peripheral(&self, id: &DeviceId) -> Result<Arc<dyn Peripheral>, Error> {
        Ok(self.core.peripheral(id)?)
    }

    async fn request_device(
        &self,
        _opts: &RequestDeviceOptions,
    ) -> Result<Arc<dyn Peripheral>, Error> {
        Err(Error::Unsupported {
            platform: "ohos",
            feature: "request_device",
        })
    }
}
