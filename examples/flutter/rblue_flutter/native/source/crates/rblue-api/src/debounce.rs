//! Scan-result debouncing: RSSI sliding windows and `DeviceUpdated` rate
//! limiting.
//!
//! Advertisements arrive at tens of milliseconds per device; surfacing every
//! sighting spams consumers and hides the signal (the strongest sightings)
//! behind noise. The debouncer:
//!
//! - groups sightings of the *same name* - the plan-mandated grouping, which
//!   also collapses RPA rotations ([`crate::AddressType::ResolvablePrivate`]
//!   devices change `DeviceId` but keep their advertised name);
//! - keeps a sliding RSSI window per group and emits the window mean;
//! - rate-limits [`DebouncedEvent::Updated`] to one per
//!   [`DebounceConfig::update_interval`] per group (default 1 s).
//!
//! The core is time-injected (every method takes `now`), so tests and
//! consumers drive it deterministically from any clock. Wire it into the
//! adapter event stream:
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::time::Instant;
//! use futures::StreamExt;
//! use rblue_api::debounce::{DebounceConfig, ScanDebouncer};
//! use rblue_api::prelude::*;
//!
//! # async fn demo(adapter: Arc<dyn Adapter>) {
//! let mut debouncer = ScanDebouncer::new(DebounceConfig::default());
//! let mut events = adapter.events();
//! while let Some(event) = events.next().await {
//!     match event {
//!         AdapterEvent::DeviceDiscovered(device) => {
//!             if let Some(debounced) = debouncer.observe(&device, Instant::now()) {
//!                 let _ = debounced; // surface to the application
//!             }
//!         }
//!         AdapterEvent::DeviceUpdated(id, rssi) => {
//!             if let Some(debounced) = debouncer.observe_update(&id, rssi, Instant::now()) {
//!                 let _ = debounced;
//!             }
//!         }
//!         _ => {}
//!     }
//! }
//! # }
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::device::{DeviceId, Rssi};
use crate::scan::DiscoveredDevice;

/// Configuration for [`ScanDebouncer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebounceConfig {
    /// Length of the RSSI sliding window; samples older than this are
    /// dropped when the group is next observed.
    pub window: Duration,
    /// Minimum spacing between surfaced `Updated` events per group
    /// (the `DeviceUpdated` rate limit).
    pub update_interval: Duration,
    /// Maximum RSSI samples retained per group; bounds memory under
    /// millisecond-rate advertisers. The window length is the other bound.
    pub max_samples: usize,
}

impl Default for DebounceConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(4),
            update_interval: Duration::from_secs(1),
            max_samples: 8,
        }
    }
}

/// An event the debouncer decided to surface.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum DebouncedEvent {
    /// First sighting of a group; mirrors `AdapterEvent::DeviceDiscovered`.
    Discovered(DiscoveredDevice),
    /// Rate-limited update with the window-mean RSSI; mirrors
    /// `AdapterEvent::DeviceUpdated`.
    Updated {
        /// Identifier of the sighting that triggered this event.
        id: DeviceId,
        /// Mean RSSI over the sliding window.
        rssi: Rssi,
    },
}

/// Grouping key: advertised name when present, device id otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum GroupKey {
    Name(String),
    Id(DeviceId),
}

#[derive(Debug)]
struct Group {
    /// Instant of the last surfaced event; the rate-limit clock.
    last_emit: Instant,
    /// `(timestamp, rssi)` samples inside the sliding window.
    samples: Vec<(Instant, i16)>,
}

/// RSSI sliding window and rate limiter over the scan event stream.
///
/// State is bounded: samples per group by [`DebounceConfig::max_samples`],
/// groups by lazy expiry ([`ScanDebouncer::sweep`]) - a group is dropped
/// once it stayed silent for a full window.
#[derive(Debug)]
pub struct ScanDebouncer {
    config: DebounceConfig,
    groups: HashMap<GroupKey, Group>,
    /// Maps every seen device id to its group so `DeviceUpdated` events
    /// (which carry no name) land in the right window.
    id_to_group: HashMap<DeviceId, GroupKey>,
}

impl ScanDebouncer {
    /// Creates a debouncer with `config`.
    #[must_use]
    pub fn new(config: DebounceConfig) -> Self {
        Self {
            config,
            groups: HashMap::new(),
            id_to_group: HashMap::new(),
        }
    }

    /// Feeds one full advertisement sighting.
    ///
    /// Returns [`DebouncedEvent::Discovered`] for the first sighting of a
    /// group, and a rate-limited [`DebouncedEvent::Updated`] (window-mean
    /// RSSI) afterwards once [`DebounceConfig::update_interval`] has
    /// elapsed; otherwise `None` (the sighting was absorbed into its
    /// group's window).
    #[must_use]
    pub fn observe(&mut self, sighting: &DiscoveredDevice, now: Instant) -> Option<DebouncedEvent> {
        let key = match &sighting.advertisement.local_name {
            Some(name) => GroupKey::Name(name.clone()),
            None => GroupKey::Id(sighting.id.clone()),
        };
        self.id_to_group.insert(sighting.id.clone(), key.clone());
        if !self.groups.contains_key(&key) {
            self.groups.insert(
                key.clone(),
                Group {
                    last_emit: now,
                    samples: Vec::new(),
                },
            );
            self.push_sample(&key, sighting.advertisement.rssi, now);
            return Some(DebouncedEvent::Discovered(sighting.clone()));
        }
        self.emit_updated(&key, sighting.id.clone(), sighting.advertisement.rssi, now)
    }

    /// Feeds a nameless `AdapterEvent::DeviceUpdated(id, rssi)`. Devices
    /// previously seen via [`ScanDebouncer::observe`] are routed into their
    /// name group; unknown ids form their own group and surface
    /// immediately.
    #[must_use]
    pub fn observe_update(
        &mut self,
        id: &DeviceId,
        rssi: Rssi,
        now: Instant,
    ) -> Option<DebouncedEvent> {
        let key = self
            .id_to_group
            .get(id)
            .cloned()
            .unwrap_or_else(|| GroupKey::Id(id.clone()));
        if !self.groups.contains_key(&key) {
            self.groups.insert(
                key.clone(),
                Group {
                    last_emit: now,
                    samples: Vec::new(),
                },
            );
            self.push_sample(&key, Some(rssi.as_i16()), now);
            return Some(DebouncedEvent::Updated {
                id: id.clone(),
                rssi,
            });
        }
        self.emit_updated(&key, id.clone(), Some(rssi.as_i16()), now)
    }

    /// Drops groups silent for a full window; returns how many were removed.
    /// Call periodically (e.g. once per scan batch) to bound memory when
    /// devices churn.
    pub fn sweep(&mut self, now: Instant) -> usize {
        let window = self.config.window;
        let before = self.groups.len();
        self.groups.retain(|_, group| {
            group
                .samples
                .last()
                .is_some_and(|(last_seen, _)| now.duration_since(*last_seen) < window)
        });
        let removed = before - self.groups.len();
        if removed > 0 {
            self.id_to_group
                .retain(|_, key| self.groups.contains_key(key));
        }
        removed
    }

    /// Number of tracked groups (diagnostics).
    #[must_use]
    pub fn tracked_groups(&self) -> usize {
        self.groups.len()
    }

    fn push_sample(&mut self, key: &GroupKey, rssi: Option<i16>, now: Instant) {
        let config = self.config.clone();
        let Some(group) = self.groups.get_mut(key) else {
            return;
        };
        let Some(dbm) = rssi else {
            return;
        };
        group.samples.push((now, dbm));
        group
            .samples
            .retain(|(seen, _)| now.duration_since(*seen) < config.window);
        let excess = group.samples.len().saturating_sub(config.max_samples);
        group.samples.drain(..excess);
    }

    fn emit_updated(
        &mut self,
        key: &GroupKey,
        id: DeviceId,
        rssi: Option<i16>,
        now: Instant,
    ) -> Option<DebouncedEvent> {
        self.push_sample(key, rssi, now);
        let group = self.groups.get(key)?;
        if group.samples.is_empty() {
            return None;
        }
        if now.duration_since(group.last_emit) < self.config.update_interval {
            return None;
        }
        let mean = mean_rssi(&group.samples);
        let group = self.groups.get_mut(key).expect("group checked above");
        group.last_emit = now;
        Some(DebouncedEvent::Updated {
            id,
            rssi: Rssi::new(mean),
        })
    }
}

/// Integer mean of the window samples (truncated toward zero).
fn mean_rssi(samples: &[(Instant, i16)]) -> i16 {
    let sum: i64 = samples.iter().map(|(_, dbm)| i64::from(*dbm)).sum();
    (sum / samples.len() as i64) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::AddressType;
    use crate::scan::AdvertisementData;

    fn sighting(id: DeviceId, name: Option<&str>, rssi: i16) -> DiscoveredDevice {
        DiscoveredDevice {
            id,
            advertisement: AdvertisementData {
                local_name: name.map(str::to_owned),
                rssi: Some(rssi),
                ..AdvertisementData::default()
            },
        }
    }

    fn mac(byte: u8) -> DeviceId {
        DeviceId::Mac([byte, 0, 0, 0, 0, 1], AddressType::RandomStatic)
    }

    #[test]
    fn first_sighting_surfaces_discovered() {
        let mut d = ScanDebouncer::new(DebounceConfig::default());
        let now = Instant::now();
        match d.observe(&sighting(mac(1), Some("sensor"), -50), now) {
            Some(DebouncedEvent::Discovered(s)) => {
                assert_eq!(s.id, mac(1));
                assert_eq!(s.advertisement.rssi, Some(-50));
            }
            other => panic!("expected Discovered, got {other:?}"),
        }
    }

    #[test]
    fn updates_are_rate_limited_per_group() {
        let mut d = ScanDebouncer::new(DebounceConfig::default());
        let t0 = Instant::now();
        assert!(d
            .observe(&sighting(mac(1), Some("sensor"), -50), t0)
            .is_some());

        // 600 ms later: absorbed by the 1 s limit (sample still recorded).
        assert!(d
            .observe(
                &sighting(mac(1), Some("sensor"), -60),
                t0 + Duration::from_millis(600)
            )
            .is_none());
        // 1.1 s after the first emit: surfaced with the window mean.
        match d.observe(
            &sighting(mac(1), Some("sensor"), -70),
            t0 + Duration::from_millis(1100),
        ) {
            Some(DebouncedEvent::Updated { rssi, .. }) => {
                assert_eq!(rssi.as_i16(), -60, "mean(-50, -60, -70)")
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn same_name_groups_across_rpa_rotation() {
        let mut d = ScanDebouncer::new(DebounceConfig::default());
        let t0 = Instant::now();
        assert!(d
            .observe(&sighting(mac(1), Some("sensor"), -50), t0)
            .is_some());

        // A different DeviceId advertising the same name joins the same
        // group and stays inside the rate-limit window.
        assert!(d
            .observe(
                &sighting(mac(2), Some("sensor"), -55),
                t0 + Duration::from_millis(300)
            )
            .is_none());
        // An unnamed device forms its own group and is surfaced.
        assert!(d
            .observe(
                &sighting(mac(3), None, -70),
                t0 + Duration::from_millis(400)
            )
            .is_some());
        assert_eq!(d.tracked_groups(), 2);
    }

    #[test]
    fn unknown_id_update_surfaces_immediately() {
        let mut d = ScanDebouncer::new(DebounceConfig::default());
        let event = d.observe_update(&mac(9), Rssi::new(-42), Instant::now());
        match event {
            Some(DebouncedEvent::Updated { id, rssi }) => {
                assert_eq!(id, mac(9));
                assert_eq!(rssi.as_i16(), -42);
            }
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn window_mean_smooths_flapping_rssi() {
        let config = DebounceConfig {
            update_interval: Duration::from_millis(100),
            ..DebounceConfig::default()
        };
        let mut d = ScanDebouncer::new(config);
        let t0 = Instant::now();
        assert!(d
            .observe(&sighting(mac(1), Some("sensor"), -40), t0)
            .is_some());
        match d.observe(
            &sighting(mac(1), Some("sensor"), -80),
            t0 + Duration::from_millis(150),
        ) {
            Some(DebouncedEvent::Updated { rssi, .. }) => assert_eq!(rssi.as_i16(), -60),
            other => panic!("expected Updated, got {other:?}"),
        }
    }

    #[test]
    fn sweep_expires_silent_groups() {
        let mut d = ScanDebouncer::new(DebounceConfig::default());
        let t0 = Instant::now();
        assert!(d
            .observe(&sighting(mac(1), Some("gone"), -50), t0)
            .is_some());
        assert!(d
            .observe(&sighting(mac(2), Some("alive"), -50), t0)
            .is_some());
        assert!(d
            .observe(
                &sighting(mac(2), Some("alive"), -50),
                t0 + Duration::from_secs(5)
            )
            .is_some());

        assert_eq!(d.sweep(t0 + Duration::from_secs(6)), 1);
        assert_eq!(d.tracked_groups(), 1);
    }

    #[test]
    fn max_samples_bounds_memory() {
        let config = DebounceConfig {
            window: Duration::from_secs(60),
            update_interval: Duration::from_millis(1),
            max_samples: 3,
        };
        let mut d = ScanDebouncer::new(config);
        let t0 = Instant::now();
        for ms in 0..10u64 {
            let _ = d.observe_update(
                &mac(1),
                Rssi::new(-(40 + ms as i16)),
                t0 + Duration::from_millis(ms),
            );
        }
        // Only the last three samples (-47, -48, -49) survive; the new -49
        // makes the mean -48.25, truncated to -48.
        match d.observe_update(&mac(1), Rssi::new(-49), t0 + Duration::from_millis(10)) {
            Some(DebouncedEvent::Updated { rssi, .. }) => assert_eq!(rssi.as_i16(), -48),
            other => panic!("expected Updated, got {other:?}"),
        }
    }
}
