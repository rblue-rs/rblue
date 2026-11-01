//! UniFFI error model: the `rblue-api` error set, re-shaped for the FFI
//! boundary.
//!
//! Every variant mirrors [`rblue_api::Error`] one to one so no information is
//! lost crossing the boundary - in particular [`RblueError::Platform`] keeps
//! the raw platform status code (Android GATT status 133, OHOS error codes,
//! ...) as a structured field instead of a string, because supervisors and
//! application retry policies match on the code itself.

use rblue_api::error::{Error as ApiError, PairingStatus as ApiPairingStatus};
use rblue_api::Error;

/// Why a permission-gated operation failed.
///
/// Mirrors `rblue_api::PermissionReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionReason {
    /// The system Bluetooth radio is switched off.
    BluetoothOff,
    /// Location permission missing (required for scanning on Android API 30
    /// and below).
    LocationDenied,
    /// A Bluetooth runtime permission was not granted (`BLUETOOTH_SCAN` /
    /// `BLUETOOTH_CONNECT` on Android API 31+, `ACCESS_BLUETOOTH` on OHOS).
    BluetoothPermissionDenied,
    /// The platform denied access for policy reasons (OS-level authorization
    /// state on Apple platforms).
    Unauthorized,
}

/// Normalized pairing/bonding failure status.
///
/// Mirrors `rblue_api::PairingStatus`; [`PairingStatus::Other`] preserves the
/// raw platform code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PairingStatus {
    /// The peer rejected the pairing request.
    Rejected,
    /// The pairing procedure did not complete within its deadline.
    TimedOut,
    /// PIN/passkey mismatch or MITM protection failure.
    AuthenticationFailed,
    /// A pairing procedure is already in progress for this device.
    InProgress,
    /// The peer or transport cannot pair (IO capability mismatch).
    NotCapable,
    /// The platform returned a status code that does not normalize.
    Other {
        /// Raw platform pairing status code.
        code: i32,
    },
}

impl From<rblue_api::PermissionReason> for PermissionReason {
    fn from(reason: rblue_api::PermissionReason) -> Self {
        match reason {
            rblue_api::PermissionReason::BluetoothOff => Self::BluetoothOff,
            rblue_api::PermissionReason::LocationDenied => Self::LocationDenied,
            rblue_api::PermissionReason::BluetoothPermissionDenied => {
                Self::BluetoothPermissionDenied
            }
            rblue_api::PermissionReason::Unauthorized => Self::Unauthorized,
        }
    }
}

impl From<ApiPairingStatus> for PairingStatus {
    fn from(status: ApiPairingStatus) -> Self {
        match status {
            ApiPairingStatus::Rejected => Self::Rejected,
            ApiPairingStatus::TimedOut => Self::TimedOut,
            ApiPairingStatus::AuthenticationFailed => Self::AuthenticationFailed,
            ApiPairingStatus::InProgress => Self::InProgress,
            ApiPairingStatus::NotCapable => Self::NotCapable,
            ApiPairingStatus::Other(code) => Self::Other { code },
        }
    }
}

/// The unified BLE error, as raised by the facade.
///
/// Mirrors `rblue_api::Error` variant by variant; see there for the
/// per-variant semantics and each backend's `error-mapping.md` for the
/// platform-code provenance. [`RblueError::InvalidArgument`] is the one
/// facade-only variant: it reports malformed JSON method arguments, which
/// never reach the backend.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum RblueError {
    /// The platform or current state does not support this capability.
    /// Expected, not a bug - UI layers should degrade.
    #[error("unsupported on {platform}: {feature}")]
    Unsupported {
        /// Platform that rejected the operation (e.g. `"web"`, `"ios"`).
        platform: String,
        /// Capability or operation that is not supported there.
        feature: String,
    },
    /// A required permission is missing.
    #[error("permission denied")]
    Permission {
        /// Why the operation was denied.
        reason: PermissionReason,
    },
    /// The device is not known to this adapter (never scanned, or the scan
    /// cache entry expired).
    #[error("device not found")]
    NotFound {
        /// The unknown device, as `DeviceId` JSON.
        device: String,
    },
    /// The operation requires a connection but the peripheral is not
    /// connected.
    #[error("not connected")]
    NotConnected,
    /// The operation (or an awaited event) exceeded its deadline.
    #[error("operation timed out")]
    Timeout,
    /// Pairing or bonding failed.
    #[error("pairing/authentication failed")]
    Pairing {
        /// Normalized failure status; `Other` carries the raw platform code.
        status: PairingStatus,
    },
    /// A GATT procedure over ATT failed.
    #[error("GATT procedure failed: code {code}")]
    Att {
        /// ATT error code (Bluetooth Core Spec: 0x01..=0x7F are
        /// protocol-defined codes, higher values are application/profile
        /// codes).
        code: u8,
        /// Handle of the attribute that caused the error, when reported.
        handle: Option<u16>,
    },
    /// The platform returned a structured status code that must be preserved
    /// verbatim - Android GATT status 133, OHOS error codes, and so on.
    /// Retry policies match on the raw code; it is never demoted to a string.
    #[error("platform error {code}: {detail}")]
    Platform {
        /// Raw platform status code (e.g. Android `BluetoothGatt` status).
        code: i32,
        /// Human-readable context for the code. Named `detail` because foreign
        /// exception classes already inherit a `message` property.
        detail: String,
    },
    /// Unified write semantics: the payload exceeded `max_write_len` and was
    /// rejected - never truncated, never chunked.
    #[error("data length {len} exceeds max {max}")]
    InvalidLength {
        /// Length of the rejected payload, in bytes.
        len: u64,
        /// Maximum accepted payload length, in bytes.
        max: u64,
    },
    /// The device disconnected while an operation was in flight.
    #[error("device disconnected during operation")]
    Disconnected,
    /// The adapter or peripheral is in a state that forbids the operation.
    #[error("invalid state: {detail}")]
    InvalidState {
        /// What state was expected vs. found.
        detail: String,
    },
    /// Backend failure with no structured status code. When a numeric
    /// platform code exists it arrives as [`RblueError::Platform`] instead.
    #[error("backend error: {detail}")]
    Backend {
        /// Free-form backend detail.
        detail: String,
    },
    /// A method argument could not be parsed (malformed JSON for a
    /// `DeviceId` / `ScanFilter` / `CharacteristicId` / options payload, or a
    /// malformed UUID). The operation never reached the backend.
    #[error("invalid argument: {detail}")]
    InvalidArgument {
        /// What failed to parse and why.
        detail: String,
    },
}

impl From<Error> for RblueError {
    fn from(error: Error) -> Self {
        match error {
            ApiError::Unsupported { platform, feature } => Self::Unsupported {
                platform: platform.to_owned(),
                feature: feature.to_owned(),
            },
            ApiError::Permission(reason) => Self::Permission {
                reason: reason.into(),
            },
            ApiError::NotFound(device) => Self::NotFound {
                device: serde_json::to_string(&device).unwrap_or_else(|_| device.to_string()),
            },
            ApiError::NotConnected => Self::NotConnected,
            ApiError::Timeout => Self::Timeout,
            ApiError::Pairing(status) => Self::Pairing {
                status: status.into(),
            },
            ApiError::Att(att) => Self::Att {
                code: att.code,
                handle: att.handle,
            },
            ApiError::Platform { code, message } => Self::Platform {
                code,
                detail: message,
            },
            ApiError::InvalidLength { len, max } => Self::InvalidLength {
                len: len as u64,
                max: max as u64,
            },
            ApiError::Disconnected => Self::Disconnected,
            ApiError::InvalidState(message) => Self::InvalidState { detail: message },
            ApiError::Backend(message) => Self::Backend { detail: message },
        }
    }
}
