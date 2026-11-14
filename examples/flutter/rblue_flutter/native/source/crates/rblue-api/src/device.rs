//! Device and adapter identity types.

use core::fmt;

/// Address kind carried by [`DeviceId::Mac`].
///
/// Modern BLE peripherals rotate resolvable private addresses roughly every
/// 15 minutes, so `ResolvablePrivate` values must never be used as a
/// persistent primary key. Reconnect across a rotation by re-scanning and
/// matching on name/advertisement fingerprints instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AddressType {
    /// Public (IEEE-registered) device address.
    Public,
    /// Static random address, stable for the device's power cycle lifetime.
    RandomStatic,
    /// Resolvable private address; rotates about every 15 minutes.
    ResolvablePrivate,
    /// The platform did not report the address kind.
    Unknown,
}

/// Platform-agnostic peripheral identifier.
///
/// iOS/macOS can never expose a real MAC (CoreBluetooth only hands out
/// peripheral UUIDs), while every other platform is MAC-first, so the union is
/// the first place platform differences are abstracted.
///
/// Application code must not assume two `DeviceId`s can be compared across
/// devices or across platforms, and must not assume `DeviceId::Mac` is stable
/// over time (RPA rotation). Persist a `DeviceId` together with the device
/// name/service fingerprint, and reconnect by re-scanning with a
/// [`crate::ScanFilter`].
///
/// Bluetooth identifiers are personal data: never log a `DeviceId` at info
/// level or above; the [`fmt::Display`] form exists for debug/trace output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DeviceId {
    /// 6-byte MAC address. Native semantics on Windows/Linux/Android/OHOS.
    /// The address type marks whether the value is a rotating RPA.
    Mac([u8; 6], AddressType),
    /// Platform-generated stable UUID (iOS/macOS). Changes when the
    /// application is reinstalled; not a permanent device identity.
    Uuid(uuid::Uuid),
    /// Web: `BluetoothDevice.id`, an opaque string.
    Web(String),
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceId::Mac(bytes, address_type) => {
                write!(f, "{:02X}", bytes[0])?;
                for byte in &bytes[1..] {
                    write!(f, ":{byte:02X}")?;
                }
                write!(f, " ({address_type:?})")
            }
            DeviceId::Uuid(uuid) => write!(f, "{uuid}"),
            DeviceId::Web(id) => write!(f, "{id}"),
        }
    }
}

/// Handle of the Bluetooth adapter.
///
/// 1.0 binds only the system default adapter; enumerating and selecting
/// multiple dongles (e.g. two BlueZ adapters) is post-1.0. This type is the
/// reserved seam for that, so adapters do not have to become un-unique later.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdapterId(String);

impl AdapterId {
    /// Identity of the system default adapter - the only adapter 1.0 binds.
    #[must_use]
    pub fn default_adapter() -> Self {
        Self("default".to_owned())
    }

    /// Stable string form of the identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for AdapterId {
    fn default() -> Self {
        Self::default_adapter()
    }
}

impl fmt::Display for AdapterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Received signal strength in dBm (typical range -127..=20).
///
/// A newtype so raw `i16` noise cannot leak into APIs that mean "an RSSI".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rssi(i16);

impl Rssi {
    /// Wraps a raw dBm value.
    #[must_use]
    pub const fn new(dbm: i16) -> Self {
        Self(dbm)
    }

    /// Raw dBm value.
    #[must_use]
    pub const fn as_i16(self) -> i16 {
        self.0
    }
}

impl From<Rssi> for i16 {
    fn from(value: Rssi) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_adapter_id_is_stable() {
        assert_eq!(AdapterId::default().as_str(), "default");
        assert_eq!(AdapterId::default_adapter(), AdapterId::default());
    }

    #[test]
    fn mac_display_is_colon_hex() {
        let id = DeviceId::Mac(
            [0x02, 0x00, 0x00, 0xCA, 0xFE, 0x01],
            AddressType::RandomStatic,
        );
        assert_eq!(id.to_string(), "02:00:00:CA:FE:01 (RandomStatic)");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn device_id_json_roundtrip() {
        let ids = [
            DeviceId::Mac([1, 2, 3, 4, 5, 6], AddressType::ResolvablePrivate),
            DeviceId::Uuid(uuid::Uuid::nil()),
            DeviceId::Web("device-id-42".to_owned()),
        ];
        for id in ids {
            let json = serde_json::to_string(&id).expect("serialize");
            let back: DeviceId = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, id);
        }
    }
}
