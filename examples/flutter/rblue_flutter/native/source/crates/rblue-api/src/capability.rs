//! Capability negotiation - the foundation for degrading gracefully on the
//! web and HarmonyOS.

/// A feature a backend may or may not support.
///
/// Backends build their [`CapabilitySet`] from a static per-platform table
/// plus runtime probes where one exists (e.g. `navigator.bluetooth.getAvailab
/// ility()` on the web). The authoritative per-platform matrix lives in the
/// implementation plan and in each backend's docs; both are generated from the
/// same table so code and documentation cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[repr(u32)]
pub enum Capability {
    /// Scanning that keeps the radio idle between channels (lower power, and
    /// required for background scanning on some platforms).
    PassiveScan,
    /// Scanning with active inquiry requests for scan responses.
    ActiveScan,
    /// Scan response payloads are delivered separately from the main
    /// advertisement.
    ScanResponse,
    /// Central role: connect to peripherals and use GATT client procedures.
    GattClient,
    /// Peripheral role: expose a GATT server (Phase 2).
    GattServer,
    /// Send advertisements as a peripheral (Phase 2).
    Advertise,
    /// Reading RSSI for an already-connected peripheral.
    RssiDuringConnection,
    /// Actively requesting an ATT MTU ([`crate::Peripheral::request_mtu`],
    /// Android `requestMtu`, OHOS `setBLEMtuSize`). Windows, macOS, and
    /// Linux negotiate automatically and have no request API.
    MtuNegotiation,
    /// Querying the current ATT MTU (read-only property).
    MtuQuery,
    /// Actively initiating or removing a bond (`Peripheral::pair` /
    /// `Peripheral::unpair`, BlueZ `Device1.Pair` / `Adapter1.RemoveDevice`,
    /// Android `createBond` / `removeBond`). iOS/macOS pair implicitly when
    /// an encrypted characteristic is accessed, so they do not report this.
    Pairing,
    /// `BluetoothDevice.watchAdvertisements()` (Chrome, behind a flag).
    WatchAdvertisements,
    /// `navigator.bluetooth.requestDevice()` user-gesture picker (web only).
    RequestDevicePicker,
    /// The OS permits background scanning (with whatever restrictions the
    /// platform documents).
    BackgroundScan,
    /// Platform/stack background connection (e.g. Android
    /// `connectGatt(autoConnect=true)`). This is not
    /// [`crate::SupervisorHandle::auto_connect`], which is the
    /// application-level reconnect supervisor.
    AutoConnect = 13,
    /// Programmatic radio on/off ([`crate::Adapter::set_powered`]). Querying
    /// [`crate::Adapter::state`] does not require this bit.
    AdapterPowerControl = 14,
    /// System-connected peripherals matching a nonempty service-UUID filter.
    /// Returned handles still require a local connect before GATT operations.
    SystemConnectedPeripherals = 15,
    /// `Peripheral::set_connection_priority`: request a connection-interval /
    /// latency trade-off for an already-connected peripheral.
    ConnectionPriority = 16,
}

bitflags::bitflags! {
    /// A set of supported [`Capability`] values, stored as a bitset.
    ///
    /// ```
    /// use rblue_api::{Capability, CapabilitySet};
    ///
    /// let caps = CapabilitySet::bit(Capability::GattClient);
    /// assert!(caps.has(Capability::GattClient));
    /// assert!(!caps.has(Capability::MtuNegotiation));
    /// ```
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
    pub struct CapabilitySet: u32 {
        /// See [`Capability::PassiveScan`].
        const PASSIVE_SCAN = 1 << Capability::PassiveScan as u32;
        /// See [`Capability::ActiveScan`].
        const ACTIVE_SCAN = 1 << Capability::ActiveScan as u32;
        /// See [`Capability::ScanResponse`].
        const SCAN_RESPONSE = 1 << Capability::ScanResponse as u32;
        /// See [`Capability::GattClient`].
        const GATT_CLIENT = 1 << Capability::GattClient as u32;
        /// See [`Capability::GattServer`].
        const GATT_SERVER = 1 << Capability::GattServer as u32;
        /// See [`Capability::Advertise`].
        const ADVERTISE = 1 << Capability::Advertise as u32;
        /// See [`Capability::RssiDuringConnection`].
        const RSSI_DURING_CONNECTION = 1 << Capability::RssiDuringConnection as u32;
        /// See [`Capability::MtuNegotiation`].
        const MTU_NEGOTIATION = 1 << Capability::MtuNegotiation as u32;
        /// See [`Capability::MtuQuery`].
        const MTU_QUERY = 1 << Capability::MtuQuery as u32;
        /// See [`Capability::Pairing`].
        const PAIRING = 1 << Capability::Pairing as u32;
        /// See [`Capability::WatchAdvertisements`].
        const WATCH_ADVERTISEMENTS = 1 << Capability::WatchAdvertisements as u32;
        /// See [`Capability::RequestDevicePicker`].
        const REQUEST_DEVICE_PICKER = 1 << Capability::RequestDevicePicker as u32;
        /// See [`Capability::BackgroundScan`].
        const BACKGROUND_SCAN = 1 << Capability::BackgroundScan as u32;
        /// See [`Capability::AutoConnect`].
        const AUTO_CONNECT = 1 << Capability::AutoConnect as u32;
        /// See [`Capability::AdapterPowerControl`].
        const ADAPTER_POWER_CONTROL = 1 << Capability::AdapterPowerControl as u32;
        /// See [`Capability::SystemConnectedPeripherals`].
        const SYSTEM_CONNECTED_PERIPHERALS =
            1 << Capability::SystemConnectedPeripherals as u32;
        /// See [`Capability::ConnectionPriority`].
        const CONNECTION_PRIORITY = 1 << Capability::ConnectionPriority as u32;
    }
}

impl CapabilitySet {
    /// The single-bit set for `capability`.
    #[must_use]
    pub const fn bit(capability: Capability) -> Self {
        Self::from_bits_truncate(1 << capability as u32)
    }

    /// Returns `true` when the backend reports this capability.
    #[must_use]
    pub const fn has(self, capability: Capability) -> bool {
        self.bits() & (1 << capability as u32) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_and_has_agree() {
        let caps =
            CapabilitySet::bit(Capability::ActiveScan) | CapabilitySet::bit(Capability::GattClient);
        assert!(caps.has(Capability::ActiveScan));
        assert!(caps.has(Capability::GattClient));
        assert!(!caps.has(Capability::Pairing));
    }

    #[test]
    fn default_set_has_nothing() {
        let caps = CapabilitySet::default();
        assert_eq!(caps, CapabilitySet::empty());
        assert!(!caps.has(Capability::GattClient));
    }

    #[test]
    fn connection_priority_uses_bit_16() {
        assert_eq!(Capability::BackgroundScan as u32, 12);
        assert_eq!(Capability::ConnectionPriority as u32, 16);
        let caps = CapabilitySet::bit(Capability::ConnectionPriority);
        assert_eq!(caps.bits(), 1 << 16);
        assert!(caps.has(Capability::ConnectionPriority));
        assert!(CapabilitySet::all().has(Capability::ConnectionPriority));
        // ConnectionPriority must not alias AutoConnect, AdapterPowerControl,
        // or SystemConnectedPeripherals (bits 13-15).
        assert_eq!(caps.bits() & (1 << 13 | 1 << 14 | 1 << 15), 0);
    }

    #[test]
    fn system_connected_peripherals_uses_bit_15() {
        assert_eq!(Capability::BackgroundScan as u32, 12);
        assert_eq!(Capability::SystemConnectedPeripherals as u32, 15);
        let caps = CapabilitySet::bit(Capability::SystemConnectedPeripherals);
        assert_eq!(caps.bits(), 1 << 15);
        assert!(caps.has(Capability::SystemConnectedPeripherals));
        assert!(CapabilitySet::all().has(Capability::SystemConnectedPeripherals));
        // This single bit must not alias AutoConnect (13) or AdapterPowerControl (14).
        assert_eq!(caps.bits() & (1 << 13 | 1 << 14), 0);
    }

    #[test]
    fn existing_bits_keep_their_positions_after_u32_widening() {
        // Widening the storage must not rotate previously published bits.
        assert_eq!(CapabilitySet::PASSIVE_SCAN.bits(), 1 << 0);
        assert_eq!(CapabilitySet::BACKGROUND_SCAN.bits(), 1 << 12);
        assert_eq!(CapabilitySet::AUTO_CONNECT.bits(), 1 << 13);
        let _: u32 = CapabilitySet::all().bits();
        assert!(CapabilitySet::all().has(Capability::AutoConnect));
        assert!(!CapabilitySet::bit(Capability::GattClient).has(Capability::AutoConnect));
        assert_eq!(Capability::AutoConnect as u32, 13);
        assert_eq!(Capability::AdapterPowerControl as u32, 14);
        assert_eq!(CapabilitySet::ADAPTER_POWER_CONTROL.bits(), 1 << 14);
        assert!(CapabilitySet::all().has(Capability::AdapterPowerControl));
        assert!(!CapabilitySet::bit(Capability::GattClient).has(Capability::AdapterPowerControl));
    }
}
