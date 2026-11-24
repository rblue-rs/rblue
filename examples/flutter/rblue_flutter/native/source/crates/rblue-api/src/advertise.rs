//! Peripheral advertising parameters.
//!
//! These types describe what this adapter should broadcast. They are distinct
//! from [`crate::AdvertisementData`], which is the parsed payload of someone
//! else's advertisement observed during a scan.

use std::time::Duration;

/// Parameters for [`crate::Adapter::start_advertising`].
///
/// Platforms may truncate or drop fields they cannot place in an
/// advertisement (especially iOS). The unified parameter shape stays the same.
/// Interval and transmit-power tiers are not part of this surface; backends
/// use platform defaults.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdvertiseParams {
    /// Local name to advertise when [`Self::include_name`] is `true`.
    #[cfg_attr(feature = "serde", serde(default))]
    pub local_name: Option<String>,
    /// Service UUIDs to include in the advertisement.
    #[cfg_attr(feature = "serde", serde(default))]
    pub services: Vec<uuid::Uuid>,
    /// Service data entries: service UUID and its data.
    #[cfg_attr(feature = "serde", serde(default))]
    pub service_data: Vec<(uuid::Uuid, Vec<u8>)>,
    /// Manufacturer-specific data: company ID and data.
    #[cfg_attr(feature = "serde", serde(default))]
    pub manufacturer_data: Option<(u16, Vec<u8>)>,
    /// Whether the advertisement is connectable. Defaults to `true`.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub connectable: bool,
    /// When `true`, include a transmit-power field if the platform allows it.
    #[cfg_attr(feature = "serde", serde(default))]
    pub include_tx_power: bool,
    /// When `true` (the default) and [`Self::local_name`] is set, include the
    /// name in the advertisement.
    #[cfg_attr(feature = "serde", serde(default = "default_true"))]
    pub include_name: bool,
    /// How long to advertise; `None` means until
    /// [`crate::Adapter::stop_advertising`]. Serialized as `timeout_ms`.
    #[cfg_attr(
        feature = "serde",
        serde(default, rename = "timeout_ms", with = "option_duration_millis")
    )]
    pub timeout: Option<Duration>,
}

impl Default for AdvertiseParams {
    fn default() -> Self {
        Self {
            local_name: None,
            services: Vec::new(),
            service_data: Vec::new(),
            manufacturer_data: None,
            connectable: true,
            include_tx_power: false,
            include_name: true,
            timeout: None,
        }
    }
}

#[cfg(feature = "serde")]
const fn default_true() -> bool {
    true
}

#[cfg(feature = "serde")]
mod option_duration_millis {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(duration) => {
                let millis = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                serializer.serialize_some(&millis)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Option::<u64>::deserialize(deserializer)?.map(Duration::from_millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_connectable_and_includes_name() {
        let params = AdvertiseParams::default();
        assert!(params.connectable);
        assert!(params.include_name);
        assert!(!params.include_tx_power);
        assert!(params.timeout.is_none());
        assert!(params.local_name.is_none());
        assert!(params.services.is_empty());
    }
}

#[cfg(all(test, feature = "serde"))]
mod serde_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn empty_object_uses_documented_defaults() {
        let params: AdvertiseParams = serde_json::from_str("{}").expect("default object");
        assert_eq!(params, AdvertiseParams::default());
    }

    #[test]
    fn timeout_round_trips_as_timeout_ms() {
        let params = AdvertiseParams {
            timeout: Some(Duration::from_millis(1500)),
            ..AdvertiseParams::default()
        };
        let json = serde_json::to_value(&params).expect("serialize");
        assert_eq!(json["timeout_ms"], 1500);
        let back: AdvertiseParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back.timeout, Some(Duration::from_millis(1500)));
    }
}
