//! Unified error model.
//!
//! Every backend maps its platform errors into [`Error`] at the boundary;
//! raw `HRESULT`/`NSError`/D-Bus/JNI error values must never leak through the
//! public API. The model distinguishes *expected capability gaps* (callers
//! should degrade gracefully, e.g. [`Error::Unsupported`]) from *runtime
//! failures*.
//!
//! Each backend must also ship an error-mapping table document
//! (`crates/rblue-*/docs/error-mapping.md`).

use crate::device::DeviceId;

/// The unified BLE error type.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Error {
    /// The platform or current state does not support this capability.
    /// Expected, not a bug - UI layers should degrade. Examples: calling
    /// `start_scan` on the web, or requesting the real MAC on iOS.
    #[error("unsupported on {platform}: {feature}")]
    Unsupported {
        /// Platform that rejected the operation (e.g. `"web"`, `"ios"`).
        platform: &'static str,
        /// Capability or operation that is not supported there.
        feature: &'static str,
    },

    /// A required permission is missing: Bluetooth radio off, location
    /// permission (Android API <= 30 scanning), runtime Bluetooth permission,
    /// or an OS-level authorization denial.
    #[error("permission denied: {0:?}")]
    Permission(PermissionReason),

    /// The device is not known to this adapter (never scanned, or the scan
    /// cache entry expired).
    #[error("device not found: {0:?}")]
    NotFound(DeviceId),

    /// The operation requires a connection but the peripheral is not connected.
    #[error("not connected")]
    NotConnected,

    /// The operation (or an awaited event) exceeded its deadline.
    #[error("operation timed out")]
    Timeout,

    /// Pairing or bonding failed; the platform status is normalized into
    /// [`PairingStatus`].
    #[error("pairing/authentication failed: {0:?}")]
    Pairing(PairingStatus),

    /// A GATT procedure over ATT failed. The ATT error code (0x01..=0x7F) is
    /// preserved in [`AttError`], normalized as far as platforms allow.
    #[error("GATT procedure failed: {0:?}")]
    Att(AttError),

    /// The platform returned a structured status code that must be preserved
    /// verbatim: Android GATT status 133 (the leading connect-failure cause),
    /// OHOS error codes, and so on. Supervisors match on the raw code, so
    /// backends must never demote these into [`Error::Backend`].
    #[error("platform error {code}: {message}")]
    Platform {
        /// Raw platform status code (e.g. Android `BluetoothGatt` status).
        code: i32,
        /// Human-readable context for the code.
        message: String,
    },

    /// Unified write semantics: data longer than
    /// [`crate::Peripheral::max_write_len`] is always rejected. It is never
    /// silently truncated (iOS native behavior) and never hiddenly chunked
    /// (the web does not chunk either - oversize simply fails).
    #[error("data length {len} exceeds max {max}")]
    InvalidLength {
        /// Length of the rejected payload, in bytes.
        len: usize,
        /// Maximum accepted payload length, in bytes.
        max: usize,
    },

    /// The device disconnected while an operation was in flight. Drives the
    /// supervisor's reconnect flow.
    #[error("device disconnected during operation")]
    Disconnected,

    /// The adapter or peripheral is in a state that forbids the operation,
    /// e.g. connecting before stopping an exclusive scan mode.
    #[error("invalid state: {0}")]
    InvalidState(String),

    /// Fallback for backend failures with no structured status code. When a
    /// numeric platform code exists, use [`Error::Platform`] instead.
    #[error("backend error: {0}")]
    Backend(String),
}

/// Why a permission-gated operation failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PermissionReason {
    /// The system Bluetooth radio is switched off.
    BluetoothOff,
    /// Location permission missing. Required for scanning on Android API 30
    /// and below.
    LocationDenied,
    /// A Bluetooth runtime permission was not granted (`BLUETOOTH_SCAN` /
    /// `BLUETOOTH_CONNECT` on Android API 31+, `ACCESS_BLUETOOTH` on OHOS).
    BluetoothPermissionDenied,
    /// The platform denied access for policy reasons (e.g. OS-level
    /// authorization state on Apple platforms).
    Unauthorized,
}

/// Normalized pairing/bonding failure status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PairingStatus {
    /// The peer rejected the pairing request.
    Rejected,
    /// The pairing procedure did not complete within its deadline.
    TimedOut,
    /// Authentication failed: PIN/passkey mismatch or MITM protection failure.
    AuthenticationFailed,
    /// A pairing procedure is already in progress for this device.
    InProgress,
    /// The peer or transport cannot pair (IO capability mismatch).
    NotCapable,
    /// The platform returned a status code that does not normalize; the raw
    /// code is preserved.
    Other(i32),
}

/// A failed GATT (ATT) procedure, carrying the ATT protocol error code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AttError {
    /// ATT error code as defined by the Bluetooth Core Spec (0x01..=0x7F for
    /// protocol-defined codes; higher values are application/profile codes).
    pub code: u8,
    /// Handle of the attribute that caused the error, when the platform
    /// reports it.
    pub handle: Option<u16>,
}

impl AttError {
    /// Spec name of a well-known ATT error code (`"invalid handle"`,
    /// `"insufficient authentication"`, ...). Returns `None` for unknown or
    /// application-defined codes.
    #[must_use]
    pub fn description(&self) -> Option<&'static str> {
        Some(match self.code {
            0x01 => "invalid handle",
            0x02 => "read not permitted",
            0x03 => "write not permitted",
            0x04 => "invalid pdu",
            0x05 => "insufficient authentication",
            0x06 => "request not supported",
            0x07 => "invalid offset",
            0x08 => "insufficient authorization",
            0x09 => "prepare queue full",
            0x0A => "attribute not found",
            0x0B => "attribute not long",
            0x0C => "insufficient encryption key size",
            0x0D => "invalid attribute value length",
            0x0E => "unlikely error",
            0x0F => "insufficient encryption",
            0x10 => "unsupported group type",
            0x11 => "insufficient resources",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_att_codes_have_descriptions() {
        let err = AttError {
            code: 0x05,
            handle: Some(0x002A),
        };
        assert_eq!(err.description(), Some("insufficient authentication"));
    }

    #[test]
    fn unknown_att_code_has_no_description() {
        let err = AttError {
            code: 0xE0,
            handle: None,
        };
        assert_eq!(err.description(), None);
    }

    #[test]
    fn error_display_preserves_platform_codes() {
        let err = Error::Platform {
            code: 133,
            message: "GATT_ERROR".to_owned(),
        };
        assert_eq!(err.to_string(), "platform error 133: GATT_ERROR");
    }

    #[test]
    fn error_display_states_unsupported_pair() {
        let err = Error::Unsupported {
            platform: "web",
            feature: "start_scan",
        };
        assert_eq!(err.to_string(), "unsupported on web: start_scan");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn error_serializes_with_platform_code() {
        let err = Error::Platform {
            code: 133,
            message: "GATT_ERROR".to_owned(),
        };
        let json = serde_json::to_value(&err).expect("serialize");
        assert!(json.to_string().contains("GATT_ERROR"));
    }
}
