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
#[repr(u16)]
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
    /// Actively requesting an MTU (`requestMtu` / `setBLEMtuSize`). Windows,
    /// macOS, and Linux negotiate automatically and have no request API.
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
    pub struct CapabilitySet: u16 {
        /// See [`Capability::PassiveScan`].
        const PASSIVE_SCAN = 1 << Capability::PassiveScan as u16;
        /// See [`Capability::ActiveScan`].
        const ACTIVE_SCAN = 1 << Capability::ActiveScan as u16;
        /// See [`Capability::ScanResponse`].
        const SCAN_RESPONSE = 1 << Capability::ScanResponse as u16;
        /// See [`Capability::GattClient`].
        const GATT_CLIENT = 1 << Capability::GattClient as u16;
        /// See [`Capability::GattServer`].
        const GATT_SERVER = 1 << Capability::GattServer as u16;
        /// See [`Capability::Advertise`].
        const ADVERTISE = 1 << Capability::Advertise as u16;
        /// See [`Capability::RssiDuringConnection`].
        const RSSI_DURING_CONNECTION = 1 << Capability::RssiDuringConnection as u16;
        /// See [`Capability::MtuNegotiation`].
        const MTU_NEGOTIATION = 1 << Capability::MtuNegotiation as u16;
        /// See [`Capability::MtuQuery`].
        const MTU_QUERY = 1 << Capability::MtuQuery as u16;
        /// See [`Capability::Pairing`].
        const PAIRING = 1 << Capability::Pairing as u16;
        /// See [`Capability::WatchAdvertisements`].
        const WATCH_ADVERTISEMENTS = 1 << Capability::WatchAdvertisements as u16;
        /// See [`Capability::RequestDevicePicker`].
        const REQUEST_DEVICE_PICKER = 1 << Capability::RequestDevicePicker as u16;
        /// See [`Capability::BackgroundScan`].
        const BACKGROUND_SCAN = 1 << Capability::BackgroundScan as u16;
    }
}

impl CapabilitySet {
    /// The single-bit set for `capability`.
    #[must_use]
    pub const fn bit(capability: Capability) -> Self {
        Self::from_bits_truncate(1 << capability as u16)
    }

    /// Returns `true` when the backend reports this capability.
    #[must_use]
    pub const fn has(self, capability: Capability) -> bool {
        self.bits() & (1 << capability as u16) != 0
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
}
