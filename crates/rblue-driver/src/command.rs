//! Control-plane types: handshake, requests, responses, and events.
//!
//! All types are serde-serializable; the JSON representation is the wire
//! format for the low-frequency control plane. Command and field names are
//! camelCase to match the host-language (`ArkTS`/Kotlin) interface shape.
//! Nested `rblue-api` payload types (`ScanFilter`, `CharacteristicId`, ...)
//! keep their own serde representation.

use serde::{Deserialize, Serialize};

use rblue_api::{
    AdapterState, AdvertisementData, CharacteristicId, DescriptorId, DeviceId, DisconnectReason,
    Error, ScanFilter, ServiceSet,
};

/// Correlation id carried by every request and its response.
///
/// `u64` by protocol decision: `u32` wraps around on high-frequency,
/// long-lived connections.
pub type RequestId = u64;

/// Protocol version of this crate. A driver whose
/// [`DriverHello::protocol_version`] differs from [`PROTOCOL_VERSION`] is
/// rejected during the handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// Returns `true` when `version` speaks this protocol version.
#[must_use]
pub const fn is_protocol_compatible(version: u32) -> bool {
    version == PROTOCOL_VERSION
}

/// Handshake sent by the Rust side when the driver registers
/// (`rblue.registerDriver(...)` in ArkTS terms).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverHello {
    /// Protocol version the Rust side speaks; see [`PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// Human-readable driver name (e.g. `"ohos-ble-driver"`), for diagnostics.
    pub driver_name: String,
    /// Driver implementation version, for diagnostics.
    pub driver_version: String,
}

impl DriverHello {
    /// Returns `true` when this hello can be accepted.
    #[must_use]
    pub const fn is_compatible(&self) -> bool {
        is_protocol_compatible(self.protocol_version)
    }
}

/// Handshake reply. A rejected driver must not receive further requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverHelloAck {
    /// Protocol version the Rust core runs.
    pub protocol_version: u32,
    /// Whether the driver was accepted.
    pub accepted: bool,
}

impl DriverHelloAck {
    /// Accepts the driver after [`DriverHello::is_compatible`] passed.
    #[must_use]
    pub const fn accept() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            accepted: true,
        }
    }

    /// Rejects the driver after a version mismatch.
    #[must_use]
    pub const fn reject() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            accepted: false,
        }
    }
}

/// One request to the driver. `req_id` correlates the eventual
/// [`DriverResponse`]; the response may arrive in any order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverRequest {
    /// Correlation id, echoed verbatim by the response.
    pub req_id: RequestId,
    /// The command to execute.
    #[serde(flatten)]
    pub command: BleDriverCommand,
}

/// Commands the Rust core sends to the host-language driver. One command per
/// request; the driver completes each with exactly one [`DriverResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BleDriverCommand {
    /// Prepare the driver (request Bluetooth permissions on the host side,
    /// wire callbacks). Must complete before any other command.
    Init,
    /// Report the adapter power/availability state. Source on OHOS is
    /// `@ohos.bluetooth.access`, not the `ble` module.
    GetAdapterState,
    /// Start scanning with the given filter.
    StartScan {
        /// Filter to apply; hardware-pushed-down where the platform allows.
        filter: ScanFilter,
    },
    /// Stop the active scan.
    StopScan,
    /// Connect to a device. The device must have been observed by a scan
    /// first on platforms with that restriction (Windows, effectively OHOS).
    Connect {
        /// Device to connect to.
        device: DeviceId,
    },
    /// Disconnect from a device. Idempotent.
    Disconnect {
        /// Device to disconnect.
        device: DeviceId,
    },
    /// Discover the GATT tree of a connected device. Must run on every
    /// connection; host-side caches may be stale.
    DiscoverServices {
        /// Connected device.
        device: DeviceId,
    },
    /// Read a characteristic value.
    Read {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped characteristic address.
        characteristic: CharacteristicId,
    },
    /// Write a characteristic value. The payload does **not** travel in this
    /// JSON command: it rides the binary data-plane frame with the same
    /// `req_id` (see [`crate::frame`]).
    Write {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped characteristic address.
        characteristic: CharacteristicId,
        /// `true` for write-with-response, `false` for write-without-response.
        with_response: bool,
    },
    /// Read a descriptor value.
    ReadDescriptor {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped descriptor address.
        descriptor: DescriptorId,
    },
    /// Write a descriptor value. The payload rides the binary data-plane
    /// frame with the same `req_id`, as with [`BleDriverCommand::Write`].
    WriteDescriptor {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped descriptor address.
        descriptor: DescriptorId,
    },
    /// Subscribe to value changes. The driver picks notify vs. indicate from
    /// the characteristic properties (OHOS exposes distinct APIs).
    Subscribe {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped characteristic address.
        characteristic: CharacteristicId,
    },
    /// Cancel a subscription. Idempotent.
    Unsubscribe {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped characteristic address.
        characteristic: CharacteristicId,
    },
    /// Read the RSSI of a connected device.
    ReadRssi {
        /// Connected device.
        device: DeviceId,
    },
    /// Request an MTU change (`setBLEMtuSize` on OHOS, API 10+). Completes
    /// with the negotiated MTU.
    RequestMtu {
        /// Connected device.
        device: DeviceId,
        /// Requested ATT MTU in bytes.
        mtu: u16,
    },
}

/// Reply to one [`DriverRequest`], carrying the same `req_id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DriverResponse {
    /// The command completed successfully.
    Success {
        /// Echoed correlation id.
        req_id: RequestId,
        /// Command-specific return value.
        value: DriverValue,
    },
    /// The command failed with a structured platform error. The raw code is
    /// preserved so supervisors can special-case values such as Android GATT
    /// status 133.
    Failure {
        /// Echoed correlation id.
        req_id: RequestId,
        /// Raw platform status code.
        code: i32,
        /// Human-readable failure context.
        message: String,
    },
}

impl DriverResponse {
    /// Maps the response onto the unified `rblue-api` error model: failures
    /// become [`Error::Platform`] so the raw code never degrades into a
    /// string.
    pub fn into_result(self) -> Result<DriverValue, Error> {
        match self {
            DriverResponse::Success { value, .. } => Ok(value),
            DriverResponse::Failure { code, message, .. } => Err(Error::Platform { code, message }),
        }
    }
}

/// Command-specific return value of a successful [`DriverResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DriverValue {
    /// Commands that return nothing (`init`, `startScan`, ...).
    None,
    /// Result of `getAdapterState`.
    AdapterState {
        /// Current adapter state.
        state: AdapterState,
    },
    /// Result of `discoverServices`.
    Services {
        /// The discovered GATT tree.
        services: ServiceSet,
    },
    /// Result of `read`. High-rate read results should use the binary
    /// data-plane frame instead; this JSON variant exists for simplicity on
    /// low-rate paths.
    Data {
        /// Value bytes.
        data: Vec<u8>,
    },
    /// Result of `readRssi`.
    Rssi {
        /// Signal strength in dBm.
        rssi: i16,
    },
    /// Result of `requestMtu`.
    Mtu {
        /// Negotiated ATT MTU in bytes.
        mtu: u16,
    },
}

/// Unsolicited events pushed by the driver to the Rust core (`emitEvent` in
/// ArkTS terms). Events carry no `req_id`; hot-path notification payloads are
/// additionally deliverable via the binary data-plane frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DriverEvent {
    /// Adapter power/availability changed.
    StateChanged {
        /// New adapter state.
        state: AdapterState,
    },
    /// A new device was seen while scanning.
    DeviceDiscovered {
        /// Device identity.
        device: DeviceId,
        /// Parsed advertisement of the sighting.
        advertisement: AdvertisementData,
    },
    /// A known device re-advertised (RSSI/payload update).
    DeviceUpdated {
        /// Device identity.
        device: DeviceId,
        /// Signal strength in dBm.
        rssi: i16,
    },
    /// A connection was established.
    DeviceConnected {
        /// Device identity.
        device: DeviceId,
    },
    /// A connection dropped.
    DeviceDisconnected {
        /// Device identity.
        device: DeviceId,
        /// Normalized reason, when the platform provides one.
        reason: Option<DisconnectReason>,
    },
    /// A subscribed characteristic delivered a value. When sent over the
    /// binary data plane, `data` is empty here and the payload rides the
    /// frame keyed by the same characteristic.
    Notification {
        /// Connected device.
        device: DeviceId,
        /// Session-scoped characteristic address.
        characteristic: CharacteristicId,
        /// Value bytes (empty when delivered via the binary frame).
        #[serde(default)]
        data: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rblue_api::{AddressType, ServiceId};

    #[test]
    fn request_json_roundtrip() {
        let request = DriverRequest {
            req_id: 42,
            command: BleDriverCommand::Write {
                device: DeviceId::Mac([1, 2, 3, 4, 5, 6], AddressType::Public),
                characteristic: CharacteristicId {
                    service: ServiceId {
                        uuid: uuid::Uuid::from_u128(0x1234),
                        instance: 0,
                    },
                    uuid: uuid::Uuid::from_u128(0x5678),
                    instance: 1,
                },
                with_response: true,
            },
        };
        let json = serde_json::to_string(&request).expect("serialize");
        assert!(json.contains("\"reqId\":42"));
        assert!(json.contains("\"type\":\"write\""));
        assert!(json.contains("\"withResponse\":true"));
        let back: DriverRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, request);
    }

    #[test]
    fn event_json_roundtrip() {
        let event = DriverEvent::DeviceDisconnected {
            device: DeviceId::Web("d1".to_owned()),
            reason: Some(DisconnectReason::Remote),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains("\"type\":\"deviceDisconnected\""));
        let back: DriverEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn version_gate() {
        assert!(is_protocol_compatible(PROTOCOL_VERSION));
        assert!(!is_protocol_compatible(PROTOCOL_VERSION + 1));
        let hello = DriverHello {
            protocol_version: PROTOCOL_VERSION,
            driver_name: "test".to_owned(),
            driver_version: "0".to_owned(),
        };
        assert!(hello.is_compatible());
        assert!(DriverHelloAck::accept().accepted);
        assert!(!DriverHelloAck::reject().accepted);
    }

    #[test]
    fn failure_maps_to_platform_error() {
        let response = DriverResponse::Failure {
            req_id: 7,
            code: 133,
            message: "GATT_ERROR".to_owned(),
        };
        match response.into_result() {
            Err(Error::Platform { code, message }) => {
                assert_eq!(code, 133);
                assert_eq!(message, "GATT_ERROR");
            }
            other => panic!("expected Error::Platform, got {other:?}"),
        }
    }
}
