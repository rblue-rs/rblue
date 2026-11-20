//! Static OHOS capability table (implementation plan section 2.3, OHOS
//! column, refined by the phase decisions recorded below).

use rblue_api::CapabilitySet;

/// Capabilities of the OHOS backend.
///
/// Reported set (code and docs must stay in sync - this table is the source
/// of truth for both):
///
/// | Capability | OHOS | Notes |
/// |---|---|---|
/// | `ActiveScan` | yes | `ble.startBLEScan` is an active scan. |
/// | `GattClient` | yes | Central role via `ble.createGattClientDevice`. |
/// | `MtuNegotiation` | yes | `GattClientDevice.setBLEMtuSize`, API 10+. |
/// | `MtuQuery` | yes | Negotiated value tracked by the registry; see `docs/platform-notes.md`. |
/// | `RssiDuringConnection` | no | `getRssiValue` needs real-device verification; not enabled yet. |
/// | `RequestDevicePicker` | no | No user-gesture picker exists on OHOS. |
/// | `Pairing` | no | Phase 2; pairing is implicit on encrypted access. |
/// | `BackgroundScan` | no | Restricted: needs additional (location-class) approval; see `docs/platform-notes.md`. |
/// | `PassiveScan` | no | The scan-duty options do not expose a passive duty. |
/// | `ScanResponse` | no | OHOS merges scan responses into the reported advertisement. |
/// | `GattServer` / `Advertise` | no | Phase 2 scope. |
/// | `WatchAdvertisements` | no | Web-only capability. |
/// | `SystemConnectedPeripherals` | no | No equivalent system-connected lookup; the method returns `Error::Unsupported`. |
#[must_use]
pub const fn ohos_capabilities() -> CapabilitySet {
    CapabilitySet::ACTIVE_SCAN
        .union(CapabilitySet::GATT_CLIENT)
        .union(CapabilitySet::MTU_NEGOTIATION)
        .union(CapabilitySet::MTU_QUERY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rblue_api::Capability;

    #[test]
    fn capability_table_matches_plan_matrix() {
        let caps = ohos_capabilities();
        assert!(caps.has(Capability::ActiveScan));
        assert!(caps.has(Capability::GattClient));
        assert!(caps.has(Capability::MtuNegotiation));
        assert!(caps.has(Capability::MtuQuery));

        // Pending real-device verification, restricted, or out of 1.0 scope:
        // none of these may be reported until the referenced verification or
        // phase lands.
        assert!(!caps.has(Capability::RssiDuringConnection));
        assert!(!caps.has(Capability::RequestDevicePicker));
        assert!(!caps.has(Capability::Pairing));
        assert!(!caps.has(Capability::BackgroundScan));
        assert!(!caps.has(Capability::PassiveScan));
        assert!(!caps.has(Capability::ScanResponse));
        assert!(!caps.has(Capability::GattServer));
        assert!(!caps.has(Capability::Advertise));
        assert!(!caps.has(Capability::WatchAdvertisements));
        assert!(!caps.has(Capability::SystemConnectedPeripherals));
    }
}
