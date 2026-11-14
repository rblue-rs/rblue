//! Session restoration after automatic reconnection.
//!
//! Reconnecting restores the *link*, not the *session*: every
//! [`CharacteristicId`] is scoped to one connection and invalid afterwards
//! (see [`crate::CharacteristicId`]), subscriptions are gone, and
//! operations that were in flight when the link dropped never completed.
//! [`SessionRestorer`] is the post-reconnect pipeline that turns a saved
//! session back into a working one:
//!
//! 1. **Re-discover** (default: always - iOS requires it per connection and
//!    Android's stack cache can be stale; a cache-validated mode consults
//!    the opt-in [`ServiceCache`](crate::cache::ServiceCache)).
//! 2. **Re-index** the saved characteristic ids onto the fresh tree.
//! 3. **Re-subscribe** every surviving characteristic (notify/indicate
//!    selection is the backend's job, driven by the characteristic
//!    properties).
//! 4. **Replay or drop** the operations that were unfinished when the link
//!    dropped, per [`ReplayPolicy`].
//!
//! # Composition with the supervisor
//!
//! The [`crate::supervisor::ConnectionSupervisor`] owns reconnection; the
//! caller consumes its `SupervisorEvent::Connected` events and runs
//! [`SessionRestorer::restore`] for the affected device, updating the
//! [`SavedSession`] it keeps per device (add to `subscriptions` on every
//! successful subscribe, append to `pending` when an operation is submitted
//! and remove it when its result arrives).
//!
//! # Examples
//!
//! ```no_run
//! use std::sync::Arc;
//! use rblue_api::prelude::*;
//! use rblue_api::session::{ReplayPolicy, SavedSession, SessionRestoreConfig, SessionRestorer};
//!
//! # async fn demo(peripheral: Arc<dyn Peripheral>) -> Result<(), Error> {
//! let restorer = SessionRestorer::new(SessionRestoreConfig {
//!     rediscovery: rblue_api::session::RediscoveryMode::Always,
//!     replay: ReplayPolicy::Replay { max_ops: 16 },
//! });
//! let saved = SavedSession {
//!     subscriptions: Vec::new(),
//!     pending: Default::default(),
//! };
//! let restored = restorer.restore(peripheral, saved).await?;
//! for id in &restored.subscriptions {
//!     // Persist the fresh ids for the *next* restore; they die with this
//!     // connection.
//!     let _ = id;
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::VecDeque;
use std::sync::Arc;

use crate::cache::{SharedServiceCache, ValidationEvidence};
use crate::error::Error;
use crate::gatt::{CharacteristicId, ServiceId, ServiceSet};
use crate::peripheral::Peripheral;
use crate::queue::{GattOperation, OpOutcome};

/// How `restore` obtains the fresh GATT tree.
#[derive(Debug, Clone, Default)]
pub enum RediscoveryMode {
    /// Always run `discover_services` after a reconnect. The default and the
    /// only mode valid on iOS/macOS (CoreBluetooth mandates per-connection
    /// discovery).
    #[default]
    Always,
    /// Consult the opt-in [`ServiceCache`](crate::cache::ServiceCache) first:
    /// with valid Database Hash /
    /// Service Changed evidence the discovery is skipped, otherwise a full
    /// discovery runs and the cache is refreshed. The caller supplies the
    /// evidence (it owns the 0x2B2A read and the pending 0x2A05 tracking).
    ///
    /// Never enable this mode on iOS/macOS; on Android pair it with the
    /// backend's forced-refresh option (`BluetoothGatt.refresh()`) whenever
    /// validation fails. Note the cache refresh stores the evidence's hash;
    /// a stricter implementation re-reads 0x2B2A after the fresh discovery.
    IfCacheValidates {
        /// The application-wide cache.
        cache: SharedServiceCache,
        /// Evidence gathered for this reconnect.
        evidence: ValidationEvidence,
    },
}

/// What happens to operations that were queued but never completed when the
/// connection dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPolicy {
    /// Re-issue up to `max_ops` pending operations against the new
    /// connection (drop the remainder). Writes replay verbatim: only
    /// replay idempotent writes, or make the payload idempotent.
    Replay {
        /// Upper bound on replayed operations; the rest are dropped.
        max_ops: usize,
    },
    /// Discard all unfinished operations.
    Drop,
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self::Replay { max_ops: 16 }
    }
}

/// Configuration for [`SessionRestorer`].
#[derive(Debug, Clone, Default)]
pub struct SessionRestoreConfig {
    /// Fresh-tree strategy; see [`RediscoveryMode`].
    pub rediscovery: RediscoveryMode,
    /// Unfinished-operation strategy; see [`ReplayPolicy`].
    pub replay: ReplayPolicy,
}

/// The session state an application keeps per device while connected.
///
/// The application updates it as it goes: append to `subscriptions` after a
/// successful subscribe, append to `pending` when an operation is submitted
/// and remove it once its result arrives. On reconnect the saved state
/// drives [`SessionRestorer::restore`].
#[derive(Debug, Clone, Default)]
pub struct SavedSession {
    /// Characteristics that were subscribed on the previous connection,
    /// identified by UUID triples (session-scoped ids of that connection).
    pub subscriptions: Vec<CharacteristicId>,
    /// Operations submitted but not completed before the disconnect.
    pub pending: VecDeque<GattOperation>,
}

/// Outcome of a successful restore.
#[derive(Debug)]
pub struct RestoredSession {
    /// The fresh GATT tree this connection's ids refer to.
    pub services: ServiceSet,
    /// Characteristics successfully re-subscribed, with their *new*
    /// session-scoped ids.
    pub subscriptions: Vec<CharacteristicId>,
    /// Saved subscriptions that no longer exist in the fresh tree.
    pub missing_subscriptions: Vec<CharacteristicId>,
    /// Subscriptions that exist but failed to re-subscribe (CCCD write
    /// failure, ...). The session stays usable; callers decide whether to
    /// surface or retry.
    pub failed_subscriptions: Vec<(CharacteristicId, Error)>,
    /// Outcomes of replayed operations, in replay order.
    pub replayed: Vec<Result<OpOutcome, Error>>,
    /// How many pending operations were dropped (by policy, by the
    /// `max_ops` bound, or because their target characteristic vanished).
    pub dropped_ops: usize,
}

/// Reconnect-aware pipeline builder: cheap to clone, share one per app.
#[derive(Debug, Clone)]
pub struct SessionRestorer {
    config: SessionRestoreConfig,
}

impl SessionRestorer {
    /// Creates a restorer with `config`.
    #[must_use]
    pub fn new(config: SessionRestoreConfig) -> Self {
        Self { config }
    }

    /// Runs the restore pipeline for a freshly (re)connected peripheral.
    ///
    /// # Errors
    ///
    /// Returns the discovery error when the fresh tree cannot be obtained
    /// (the connection is then useless anyway). Per-subscription and
    /// per-operation failures are *not* fatal: they are reported inside
    /// [`RestoredSession`].
    pub async fn restore(
        &self,
        peripheral: Arc<dyn Peripheral>,
        saved: SavedSession,
    ) -> Result<RestoredSession, Error> {
        let services = self.fresh_services(&peripheral).await?;
        let reindex = |old: CharacteristicId| reindex_characteristic(old, &services);

        let mut subscriptions = Vec::with_capacity(saved.subscriptions.len());
        let mut missing_subscriptions = Vec::new();
        let mut failed_subscriptions = Vec::new();
        for old in saved.subscriptions {
            match reindex(old) {
                Some(new) => {
                    if let Err(err) = peripheral.subscribe(&new).await {
                        failed_subscriptions.push((new, err));
                    } else {
                        subscriptions.push(new);
                    }
                }
                None => missing_subscriptions.push(old),
            }
        }

        let mut replayed = Vec::new();
        let mut dropped_ops = saved.pending.len();
        if let ReplayPolicy::Replay { max_ops } = self.config.replay {
            dropped_ops = saved.pending.len().saturating_sub(max_ops);
            for mut op in saved.pending.into_iter().take(max_ops) {
                op.reindex(reindex);
                if op_references_missing(&op, &reindex) {
                    dropped_ops += 1;
                    continue;
                }
                replayed.push(op.execute(peripheral.as_ref()).await);
            }
        }

        Ok(RestoredSession {
            services,
            subscriptions,
            missing_subscriptions,
            failed_subscriptions,
            replayed,
            dropped_ops,
        })
    }

    async fn fresh_services(&self, peripheral: &Arc<dyn Peripheral>) -> Result<ServiceSet, Error> {
        match &self.config.rediscovery {
            RediscoveryMode::Always => peripheral.discover_services(&[]).await,
            RediscoveryMode::IfCacheValidates { cache, evidence } => {
                if let Some(cached) = cache.reuse(peripheral.id(), evidence) {
                    return Ok(cached);
                }
                let services = peripheral.discover_services(&[]).await?;
                let hash = evidence.database_hash;
                cache.insert(peripheral.id().clone(), services.clone(), hash);
                Ok(services)
            }
        }
    }
}

/// Maps a characteristic id from a previous connection onto a freshly
/// discovered tree.
///
/// A [`CharacteristicId`] is `(service uuid, service instance, characteristic
/// uuid, characteristic instance)`; the mapping succeeds when the same
/// UUID/instance coordinates still exist in `services` - discovery order
/// defines the instance indices, so this is a validity check plus a rebuild
/// of the id. Returns `None` when the characteristic (or its service) is
/// gone, which the caller reports as missing.
#[must_use]
pub fn reindex_characteristic(
    old: CharacteristicId,
    services: &ServiceSet,
) -> Option<CharacteristicId> {
    let service = services
        .services
        .iter()
        .filter(|service| service.uuid == old.service.uuid)
        .nth(old.service.instance as usize)?;
    let characteristic = service
        .characteristics
        .iter()
        .filter(|characteristic| characteristic.uuid == old.uuid)
        .nth(old.instance as usize)?;

    // ServiceId.instance is the index among same-UUID services, so it carries
    // over verbatim once existence is established.
    Some(CharacteristicId {
        service: ServiceId {
            uuid: old.service.uuid,
            instance: old.service.instance,
        },
        uuid: characteristic.uuid,
        instance: old.instance,
    })
}

/// Returns `true` when the operation's (reindexed) target no longer resolves.
fn op_references_missing(
    op: &GattOperation,
    reindex: &dyn Fn(CharacteristicId) -> Option<CharacteristicId>,
) -> bool {
    match op {
        GattOperation::Discover { .. } => false,
        GattOperation::Read { ch }
        | GattOperation::Subscribe { ch }
        | GattOperation::Unsubscribe { ch }
        | GattOperation::Write { ch, .. } => reindex(*ch).is_none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{AddressType, DeviceId};
    use crate::gatt::{Characteristic, Service};
    use crate::queue::OpOutcome;
    use futures::StreamExt;

    const BATTERY_SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x180F);
    const LEVEL_CHAR: uuid::Uuid = uuid::Uuid::from_u128(0x2B19);
    const HEART_RATE_SERVICE: uuid::Uuid = uuid::Uuid::from_u128(0x180D);

    fn id(service: uuid::Uuid, char: uuid::Uuid) -> CharacteristicId {
        CharacteristicId {
            service: ServiceId {
                uuid: service,
                instance: 0,
            },
            uuid: char,
            instance: 0,
        }
    }

    fn tree() -> ServiceSet {
        ServiceSet {
            services: vec![Service {
                uuid: BATTERY_SERVICE,
                primary: true,
                characteristics: vec![Characteristic {
                    uuid: LEVEL_CHAR,
                    properties: crate::CharacteristicProperties::READ,
                    descriptors: Vec::new(),
                }],
            }],
        }
    }

    #[test]
    fn reindex_keeps_surviving_characteristic() {
        let mapped = reindex_characteristic(id(BATTERY_SERVICE, LEVEL_CHAR), &tree());
        assert_eq!(mapped, Some(id(BATTERY_SERVICE, LEVEL_CHAR)));
    }

    #[test]
    fn reindex_reports_missing_characteristic() {
        assert_eq!(
            reindex_characteristic(id(HEART_RATE_SERVICE, LEVEL_CHAR), &tree()),
            None,
            "service gone"
        );
        assert_eq!(
            reindex_characteristic(id(BATTERY_SERVICE, HEART_RATE_SERVICE), &tree()),
            None,
            "characteristic gone"
        );
    }

    #[test]
    fn reindex_respects_instance_coordinates() {
        let mut services = tree();
        services.services.push(Service {
            uuid: BATTERY_SERVICE,
            primary: false,
            characteristics: Vec::new(),
        });
        let second_instance = CharacteristicId {
            service: ServiceId {
                uuid: BATTERY_SERVICE,
                instance: 1,
            },
            uuid: LEVEL_CHAR,
            instance: 0,
        };
        // instance 1 of the battery service has no characteristics.
        assert_eq!(reindex_characteristic(second_instance, &services), None);
        // instance 0 still resolves.
        assert!(reindex_characteristic(id(BATTERY_SERVICE, LEVEL_CHAR), &services).is_some());
    }

    /// Minimal peripheral double that counts subscribe calls.
    struct StubPeripheral {
        id: DeviceId,
        subscribes: std::sync::atomic::AtomicUsize,
        fail_subscription: std::sync::atomic::AtomicBool,
    }

    impl StubPeripheral {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: DeviceId::Mac([7, 0, 0, 0, 0, 1], AddressType::RandomStatic),
                subscribes: std::sync::atomic::AtomicUsize::new(0),
                fail_subscription: std::sync::atomic::AtomicBool::new(false),
            })
        }
    }

    #[async_trait::async_trait]
    impl Peripheral for StubPeripheral {
        fn id(&self) -> &DeviceId {
            &self.id
        }

        fn name(&self) -> Option<String> {
            None
        }

        async fn is_connected(&self) -> Result<bool, Error> {
            Ok(true)
        }

        async fn connect(&self) -> Result<(), Error> {
            Ok(())
        }

        async fn disconnect(&self) -> Result<(), Error> {
            Ok(())
        }

        async fn discover_services(&self, _filter: &[uuid::Uuid]) -> Result<ServiceSet, Error> {
            Ok(tree())
        }

        async fn read(&self, _ch: &CharacteristicId) -> Result<Vec<u8>, Error> {
            Ok(vec![0x64])
        }

        async fn write(
            &self,
            _ch: &CharacteristicId,
            _data: &[u8],
            _mode: crate::WriteMode,
        ) -> Result<(), Error> {
            Ok(())
        }

        async fn max_write_len(
            &self,
            _ch: &CharacteristicId,
            _mode: crate::WriteMode,
        ) -> Result<usize, Error> {
            Ok(20)
        }

        async fn subscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            if self
                .fail_subscription
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err(Error::NotConnected);
            }
            self.subscribes
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(())
        }

        async fn unsubscribe(&self, _ch: &CharacteristicId) -> Result<(), Error> {
            Ok(())
        }

        fn notifications(&self) -> crate::NotificationStream {
            futures::stream::empty().boxed()
        }

        async fn pair(&self) -> Result<(), Error> {
            Err(Error::Unsupported {
                platform: "test",
                feature: "pair",
            })
        }

        async fn rssi(&self) -> Result<i16, Error> {
            Ok(-40)
        }

        async fn mtu(&self) -> Result<u16, Error> {
            Ok(23)
        }
    }

    #[tokio::test]
    async fn restore_resubscribes_and_replays() {
        let peripheral = StubPeripheral::new();
        let restorer = SessionRestorer::new(SessionRestoreConfig {
            rediscovery: RediscoveryMode::Always,
            replay: ReplayPolicy::Replay { max_ops: 4 },
        });
        let saved = SavedSession {
            subscriptions: vec![
                id(BATTERY_SERVICE, LEVEL_CHAR),
                id(HEART_RATE_SERVICE, LEVEL_CHAR),
            ],
            pending: VecDeque::from(vec![
                GattOperation::Read {
                    ch: id(BATTERY_SERVICE, LEVEL_CHAR),
                },
                GattOperation::Read {
                    ch: id(HEART_RATE_SERVICE, LEVEL_CHAR),
                },
            ]),
        };

        let restored = restorer
            .restore(Arc::clone(&peripheral) as Arc<dyn Peripheral>, saved)
            .await
            .expect("restore");

        assert_eq!(
            restored.subscriptions,
            vec![id(BATTERY_SERVICE, LEVEL_CHAR)]
        );
        assert_eq!(
            restored.missing_subscriptions,
            vec![id(HEART_RATE_SERVICE, LEVEL_CHAR)]
        );
        assert_eq!(
            peripheral
                .subscribes
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            restored.replayed.len(),
            1,
            "only the surviving read replays"
        );
        assert!(matches!(restored.replayed[0], Ok(OpOutcome::Read(_))));
        assert_eq!(
            restored.dropped_ops, 1,
            "the read against a missing char is dropped"
        );
    }

    #[tokio::test]
    async fn restore_with_drop_policy_discards_pending() {
        let peripheral = StubPeripheral::new();
        let restorer = SessionRestorer::new(SessionRestoreConfig {
            rediscovery: RediscoveryMode::Always,
            replay: ReplayPolicy::Drop,
        });
        let saved = SavedSession {
            subscriptions: Vec::new(),
            pending: VecDeque::from(vec![GattOperation::discover_all()]),
        };

        let restored = restorer
            .restore(Arc::clone(&peripheral) as Arc<dyn Peripheral>, saved)
            .await
            .expect("restore");
        assert_eq!(restored.replayed.len(), 0);
        assert_eq!(restored.dropped_ops, 1);
    }

    #[tokio::test]
    async fn failed_subscription_is_reported_not_fatal() {
        let peripheral = StubPeripheral::new();
        peripheral
            .fail_subscription
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let restorer = SessionRestorer::new(SessionRestoreConfig::default());
        let saved = SavedSession {
            subscriptions: vec![id(BATTERY_SERVICE, LEVEL_CHAR)],
            pending: VecDeque::new(),
        };

        let restored = restorer
            .restore(Arc::clone(&peripheral) as Arc<dyn Peripheral>, saved)
            .await
            .expect("restore");
        assert!(restored.subscriptions.is_empty());
        assert_eq!(restored.failed_subscriptions.len(), 1);
        assert!(matches!(
            restored.failed_subscriptions[0].1,
            Error::NotConnected
        ));
    }

    #[tokio::test]
    async fn cache_validated_mode_skips_discovery_on_hit() {
        use crate::cache::{ServiceCache, ServiceCacheConfig};
        let cache = Arc::new(ServiceCache::new(ServiceCacheConfig::default()));
        // Prime the cache with the tree the stub will (not) discover again.
        let peripheral = StubPeripheral::new();
        cache.insert(peripheral.id.clone(), tree(), Some([9; 16]));

        let restorer = SessionRestorer::new(SessionRestoreConfig {
            rediscovery: RediscoveryMode::IfCacheValidates {
                cache: Arc::clone(&cache),
                evidence: ValidationEvidence {
                    database_hash: Some([9; 16]),
                    service_changed_range: None,
                },
            },
            replay: ReplayPolicy::Drop,
        });
        let restored = restorer
            .restore(
                Arc::clone(&peripheral) as Arc<dyn Peripheral>,
                SavedSession::default(),
            )
            .await
            .expect("restore");
        assert_eq!(restored.services, tree());

        // Hash mismatch invalidates and forces a fresh discovery (same tree
        // from the stub, but the cache is refreshed with the new hash).
        let restorer = SessionRestorer::new(SessionRestoreConfig {
            rediscovery: RediscoveryMode::IfCacheValidates {
                cache: Arc::clone(&cache),
                evidence: ValidationEvidence {
                    database_hash: Some([8; 16]),
                    service_changed_range: None,
                },
            },
            replay: ReplayPolicy::Drop,
        });
        let restored = restorer
            .restore(
                Arc::clone(&peripheral) as Arc<dyn Peripheral>,
                SavedSession::default(),
            )
            .await
            .expect("restore");
        assert_eq!(restored.services, tree());
        assert!(
            cache
                .reuse(
                    peripheral.id(),
                    &ValidationEvidence {
                        database_hash: Some([8; 16]),
                        service_changed_range: None,
                    }
                )
                .is_some(),
            "cache refreshed with the new hash"
        );
    }
}
