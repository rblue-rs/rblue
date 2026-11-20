//! Radio-state helpers that bypass the fork's constant `CentralState::Unknown`.
//!
//! `BluetoothAdapter.getState()` and `ACTION_STATE_CHANGED` are queried from
//! the shell so `Adapter::state` and live `StateChanged` events report the
//! real radio. Programmatic enable/disable is not offered: modern Android
//! rejects or deprecates those APIs for ordinary apps.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use rblue_api::event::{AdapterEvent, AdapterState};

use crate::mapping::map_android_bluetooth_state;

/// Sentinel stored before the first successful query or broadcast.
const UNSET: i32 = i32::MIN;

static LAST: AtomicI32 = AtomicI32::new(UNSET);
static PUBLISHERS: OnceLock<Mutex<Vec<Box<dyn Fn(AdapterEvent) + Send + Sync>>>> = OnceLock::new();

fn publishers() -> &'static Mutex<Vec<Box<dyn Fn(AdapterEvent) + Send + Sync>>> {
    PUBLISHERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Registers a fan-out target for live `StateChanged` events.
pub(crate) fn register_publisher(publish: impl Fn(AdapterEvent) + Send + Sync + 'static) {
    publishers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(Box::new(publish));
}

/// Records a native `BluetoothAdapter` state code and publishes a change.
pub(crate) fn apply_native_state(code: i32) {
    let previous = LAST.swap(code, Ordering::AcqRel);
    if previous == code {
        return;
    }
    let event = AdapterEvent::StateChanged(map_android_bluetooth_state(code));
    for publish in publishers()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
    {
        publish(event.clone());
    }
}

/// Live radio state: a JNI query when the VM is available, else the last
/// broadcast, else [`AdapterState::Unknown`].
pub(crate) fn current_state() -> AdapterState {
    match query_adapter_state() {
        Ok(code) => {
            LAST.store(code, Ordering::Release);
            map_android_bluetooth_state(code)
        }
        Err(()) => match LAST.load(Ordering::Acquire) {
            UNSET => AdapterState::Unknown,
            code => map_android_bluetooth_state(code),
        },
    }
}

fn query_adapter_state() -> Result<i32, ()> {
    let vm = jni::JavaVM::singleton().map_err(|_| ())?;
    vm.attach_current_thread(|env| {
        let adapter = env
            .call_static_method(
                jni::jni_str!("android/bluetooth/BluetoothAdapter"),
                jni::jni_str!("getDefaultAdapter"),
                jni::jni_sig!("()Landroid/bluetooth/BluetoothAdapter;"),
                &[],
            )
            .map_err(|_| ())?
            .l()
            .map_err(|_| ())?;
        if adapter.is_null() {
            return Ok(-2);
        }
        match env.call_method(
            &adapter,
            jni::jni_str!("getState"),
            jni::jni_sig!("()I"),
            &[],
        ) {
            Ok(value) => value.i().map_err(|_| ()),
            Err(jni::errors::Error::JavaException) => {
                let _ = env.exception_occurred();
                env.exception_clear().ok();
                Ok(-3)
            }
            Err(_) => Err(()),
        }
    })
    .map_err(|_| ())
}
