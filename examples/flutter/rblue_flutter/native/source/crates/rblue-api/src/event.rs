//! Adapter-level event model.

use crate::device::{DeviceId, Rssi};
use crate::scan::DiscoveredDevice;

/// Radio/permission state of the adapter, as reported by
/// [`crate::AdapterEvent::StateChanged`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AdapterState {
    /// The Bluetooth radio is powered off.
    PoweredOff,
    /// The radio is on and the adapter is usable.
    PoweredOn,
    /// Permission to use Bluetooth has not been granted; see
    /// [`crate::Error::Permission`].
    Unauthorized,
    /// No Bluetooth hardware is present or the platform has no support.
    Unavailable,
    /// The state is not (yet) known; the default before the first event.
    #[default]
    Unknown,
}

/// Why a connection dropped. Platforms that do not surface a structured
/// reason report [`DisconnectReason::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DisconnectReason {
    /// The local side closed the connection (`Peripheral::disconnect`).
    Local,
    /// The peer dropped the connection or the link was lost.
    Remote,
    /// The connection supervision timeout elapsed.
    TimedOut,
    /// The platform reported a raw status code (e.g. Android GATT status);
    /// supervisors match on it directly.
    Platform(i32),
    /// The platform did not provide a reason.
    Unknown,
}

/// Streamed adapter-level events.
///
/// Deliveries are ordered per source but the interleaving of independent
/// events (scan results vs. connection state) is unspecified.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AdapterEvent {
    /// Adapter power/permission state changed.
    StateChanged(AdapterState),
    /// A new device was seen. Carries the parsed advertisement; raw
    /// advertisement bytes are not part of the model.
    DeviceDiscovered(DiscoveredDevice),
    /// A known device re-advertised: RSSI or payload updated.
    DeviceUpdated(DeviceId, Rssi),
    /// A device connection was established.
    DeviceConnected(DeviceId),
    /// A device connection dropped, with the reason when known.
    DeviceDisconnected(DeviceId, Option<DisconnectReason>),
}
