//! Lock helper shared by the event bus and the wasm backend.

use std::sync::{Mutex, MutexGuard};

/// Locks `mutex`, recovering from poisoning.
///
/// The guarded data is plain queue/map state that stays structurally valid
/// across a panic (an unwinding publisher cannot leave a `VecDeque` or
/// `HashMap` half-mutated), so continuing after a poisoned lock is sound.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
