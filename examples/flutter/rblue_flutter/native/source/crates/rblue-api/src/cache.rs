//! Opt-in service cache with Database Hash / Service Changed validation.
//!
//! The default policy of rblue is to run [`crate::Peripheral::
//! discover_services`] on every connection: discovery usually costs only
//! 50-200 ms, iOS CoreBluetooth requires a fresh discovery per connection
//! anyway, and session-scoped identifiers are never reused across
//! connections (see [`crate::CharacteristicId`]). This module is the
//! opt-in escape hatch for applications that connect frequently to
//! well-behaved peripherals and want to skip redundant work.
//!
//! # Validation premise
//!
//! Caching is only sound when the peripheral lets us detect a changed GATT
//! database. Two Standard Characteristics carry that signal:
//!
//! - **Database Hash (0x2B2A)**: a hash of the server's attribute table.
//!   The cached entry is reused only when the hash read now equals the hash
//!   observed at discovery time; any difference invalidates the entry.
//! - **Service Changed (0x2A05)**: an indication announcing that attribute
//!   handles changed. A pending Service Changed indication always
//!   invalidates the cached entry (conservatively: the unified GATT model
//!   carries no ATT handles, so the affected range cannot be narrowed).
//!
//! Entries cached without a Database Hash value can never be validated and
//! are therefore never reused - without 0x2B2A the premise of this cache
//! does not hold.
//!
//! # Platform facts (why this stays opt-in)
//!
//! - **iOS/macOS**: CoreBluetooth rediscovering on every connection is
//!   mandatory; do not enable this cache on Apple platforms.
//! - **Android**: after a firmware GATT-table change the platform stack
//!   keeps serving its own cache; rediscovery needs the hidden
//!   `BluetoothGatt.refresh()` call, which rblue backends expose as a
//!   forced-refresh option on `discover_services`. Enable it whenever the
//!   Database Hash invalidated an entry.
//! - Devices without 0x2B2A/0x2A05: no validation is possible, the cache
//!   silently degrades to always-rediscover.
//!
//! The cache is a plain synchronous structure (no awaits inside), safe to
//! share across tasks via `Arc`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::device::DeviceId;
use crate::gatt::ServiceSet;

/// GATT Service Changed standard characteristic UUID (0x2A05).
#[must_use]
pub fn service_changed_uuid() -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0000_2A05_0000_1000_8000_0080_5F9B_34FB)
}

/// GATT Database Hash standard characteristic UUID (0x2B2A).
#[must_use]
pub fn database_hash_uuid() -> uuid::Uuid {
    uuid::Uuid::from_u128(0x0000_2B2A_0000_1000_8000_0080_5F9B_34FB)
}

/// Current validation evidence collected from the peripheral before reusing
/// a cache entry.
///
/// Phase C facades gather this by reading the Database Hash characteristic
/// (when the previously discovered tree contained one) and by tracking
/// pending Service Changed indications; `session::SessionRestorer` drives
/// the same check in its cache-validated rediscovery mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValidationEvidence {
    /// Value read from the Database Hash characteristic (0x2B2A), when the
    /// device exposes it. `None` means "could not be read".
    pub database_hash: Option<[u8; 16]>,
    /// `Some((start_handle, end_handle))` while an unacknowledged Service
    /// Changed indication (0x2A05) is pending for this device.
    pub service_changed_range: Option<(u16, u16)>,
}

#[derive(Debug)]
struct CacheEntry {
    services: ServiceSet,
    /// Database Hash observed when the tree was discovered.
    database_hash: Option<[u8; 16]>,
}

/// Configuration for [`ServiceCache`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceCacheConfig {
    /// Maximum number of device entries; inserting beyond this evicts in
    /// unspecified order (devices are few; the bound only stops unbounded
    /// growth when scanning thousands of devices).
    pub max_entries: usize,
}

impl Default for ServiceCacheConfig {
    fn default() -> Self {
        Self { max_entries: 16 }
    }
}

/// Opt-in GATT service cache keyed by [`DeviceId`].
///
/// Insert after a full discovery; consult [`ServiceCache::reuse`] on the
/// next connection and fall back to a fresh discovery whenever it returns
/// `None`. See the module docs for the validation premise and the platform
/// caveats that keep this disabled by default.
#[derive(Debug)]
pub struct ServiceCache {
    config: ServiceCacheConfig,
    entries: Mutex<HashMap<DeviceId, CacheEntry>>,
}

impl ServiceCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new(config: ServiceCacheConfig) -> Self {
        Self {
            config,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Stores the discovery result of one connection, together with the
    /// Database Hash read from the (just-discovered) tree, if any.
    pub fn insert(&self, id: DeviceId, services: ServiceSet, database_hash: Option<[u8; 16]>) {
        let mut entries = lock(&self.entries);
        if entries.len() >= self.config.max_entries && !entries.contains_key(&id) {
            entries.clear();
        }
        entries.insert(
            id,
            CacheEntry {
                services,
                database_hash,
            },
        );
    }

    /// Returns the cached tree when validation passes:
    ///
    /// 1. an entry exists for `id`,
    /// 2. no Service Changed indication is pending,
    /// 3. both the entry and the fresh evidence carry a Database Hash and
    ///    they are equal.
    ///
    /// Any failure invalidates the entry first, so a failed check always
    /// forces a fresh discovery on the next connection too.
    pub fn reuse(&self, id: &DeviceId, evidence: &ValidationEvidence) -> Option<ServiceSet> {
        let mut entries = lock(&self.entries);
        let entry = entries.get(id)?;
        let valid = evidence.service_changed_range.is_none()
            && evidence.database_hash.is_some()
            && entry.database_hash.is_some()
            && entry.database_hash == evidence.database_hash;
        if valid {
            return Some(entry.services.clone());
        }
        entries.remove(id);
        None
    }

    /// Explicitly drops the entry for `id` (e.g. after an unexpected GATT
    /// error that suggests a changed tree). Returns `true` when an entry
    /// was removed.
    pub fn invalidate(&self, id: &DeviceId) -> bool {
        lock(&self.entries).remove(id).is_some()
    }

    /// Drops every entry (e.g. after a firmware upgrade session).
    pub fn clear(&self) {
        lock(&self.entries).clear();
    }

    /// Number of cached devices (diagnostics).
    #[must_use]
    pub fn len(&self) -> usize {
        lock(&self.entries).len()
    }

    /// Returns `true` when nothing is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Shared-handle alias used by `session::RediscoveryMode::IfCacheValidates`.
pub type SharedServiceCache = Arc<ServiceCache>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::AddressType;
    use crate::gatt::{Characteristic, Service};

    fn device(byte: u8) -> DeviceId {
        DeviceId::Mac([byte, 0, 0, 0, 0, 1], AddressType::RandomStatic)
    }

    fn tree() -> ServiceSet {
        ServiceSet {
            services: vec![Service {
                uuid: uuid::Uuid::from_u128(0x180F),
                primary: true,
                characteristics: vec![Characteristic {
                    uuid: uuid::Uuid::from_u128(0x2B19),
                    properties: crate::CharacteristicProperties::READ,
                    descriptors: Vec::new(),
                }],
            }],
        }
    }

    const HASH_A: [u8; 16] = [1; 16];
    const HASH_B: [u8; 16] = [2; 16];

    #[test]
    fn matching_hash_reuses_entry() {
        let cache = ServiceCache::new(ServiceCacheConfig::default());
        cache.insert(device(1), tree(), Some(HASH_A));

        let reused = cache.reuse(
            &device(1),
            &ValidationEvidence {
                database_hash: Some(HASH_A),
                service_changed_range: None,
            },
        );
        assert!(reused.is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn hash_mismatch_invalidates() {
        let cache = ServiceCache::new(ServiceCacheConfig::default());
        cache.insert(device(1), tree(), Some(HASH_A));

        let reused = cache.reuse(
            &device(1),
            &ValidationEvidence {
                database_hash: Some(HASH_B),
                service_changed_range: None,
            },
        );
        assert!(reused.is_none());
        assert!(cache.is_empty(), "invalid entry must be dropped");
    }

    #[test]
    fn pending_service_changed_invalidates() {
        let cache = ServiceCache::new(ServiceCacheConfig::default());
        cache.insert(device(1), tree(), Some(HASH_A));

        let reused = cache.reuse(
            &device(1),
            &ValidationEvidence {
                database_hash: Some(HASH_A),
                service_changed_range: Some((0x0001, 0x0010)),
            },
        );
        assert!(reused.is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn unverifiable_entries_are_never_reused() {
        let cache = ServiceCache::new(ServiceCacheConfig::default());
        // Cached without a hash, or hash unreadable this time: no way to
        // verify the premise, so the cache must yield.
        cache.insert(device(1), tree(), None);
        assert!(cache
            .reuse(
                &device(1),
                &ValidationEvidence {
                    database_hash: Some(HASH_A),
                    service_changed_range: None,
                }
            )
            .is_none());

        cache.insert(device(2), tree(), Some(HASH_A));
        assert!(cache
            .reuse(&device(2), &ValidationEvidence::default())
            .is_none());
    }

    #[test]
    fn invalidate_and_clear() {
        let cache = ServiceCache::new(ServiceCacheConfig::default());
        cache.insert(device(1), tree(), Some(HASH_A));
        cache.insert(device(2), tree(), Some(HASH_A));
        assert!(cache.invalidate(&device(1)));
        assert!(!cache.invalidate(&device(1)));
        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn max_entries_is_bounded() {
        let cache = ServiceCache::new(ServiceCacheConfig { max_entries: 2 });
        cache.insert(device(1), tree(), Some(HASH_A));
        cache.insert(device(2), tree(), Some(HASH_A));
        cache.insert(device(3), tree(), Some(HASH_A));
        assert!(cache.len() <= 2);
    }

    #[test]
    fn standard_uuids_match_spec() {
        assert_eq!(
            database_hash_uuid().to_string(),
            "00002b2a-0000-1000-8000-00805f9b34fb"
        );
        assert_eq!(
            service_changed_uuid().to_string(),
            "00002a05-0000-1000-8000-00805f9b34fb"
        );
    }
}
