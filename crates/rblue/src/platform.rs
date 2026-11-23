//! Platform/backend selection, in the `btleplug::platform` pattern.
//!
//! Every backend crate exposes the same two entry points,
//! `default_adapter() -> Result<Arc<dyn Adapter>, Error>` and
//! `backend_version() -> &'static str`. This module picks which one the
//! crate exports at compile time:
//!
//! - if a backend matching the current target is compiled in, it is selected
//!   (the automatic `cfg(target_os)`-style choice),
//! - otherwise `mock`, the universal no-hardware fallback,
//! - otherwise the first enabled backend in the fixed order `web`, `ohos`,
//!   `winrt`, `corebluetooth`, `bluez`, `android`. Those runtimes fail with
//!   [`crate::Error::Unsupported`] at run time when away from their native target;
//!   the web and ohos submodules are host-testable placeholders by design.
//!
//! Regardless of the selection, every compiled-in backend remains reachable
//! under its own submodule, so a test or an embedder can address a
//! non-selected backend explicitly (or construct backend-specific types such
//! as `mock::MockAdapter`).
//!
//! The `ohos` backend is the Driver track: its adapter is driven by an
//! ArkTS `BleDriver` implementation injected through a `DriverRegistry`
//! (`ohos::DriverRegistry`; see the `ohos` submodule docs). `ohos::
//! default_adapter` builds an adapter over a fresh, driver-less registry -
//! usable only for host tests; a real OHOS app obtains the adapter wired to
//! the Node-API facade instead.

#![warn(missing_docs)]

cfg_if::cfg_if! {
    // Automatic choice first: the backend native to the target OS, when its
    // feature is enabled.
    if #[cfg(all(feature = "corebluetooth", target_vendor = "apple"))] {
        pub use self::corebluetooth::{backend_version, default_adapter};
    } else if #[cfg(all(feature = "winrt", target_os = "windows"))] {
        pub use self::winrt::{backend_version, default_adapter};
    } else if #[cfg(all(feature = "android", target_os = "android"))] {
        pub use self::android::{backend_version, default_adapter};
    } else if #[cfg(all(feature = "bluez", target_os = "linux", not(target_env = "ohos")))] {
        pub use self::bluez::{backend_version, default_adapter};
    } else if #[cfg(all(feature = "web", target_arch = "wasm32"))] {
        pub use self::web::{backend_version, default_adapter};
    } else if #[cfg(all(feature = "ohos", target_os = "linux", target_env = "ohos"))] {
        pub use self::ohos::{backend_version, default_adapter};

    // Explicit non-native fallbacks. `mock` comes first: it is the default
    // feature and the only backend that functions on every target.
    } else if #[cfg(feature = "mock")] {
        pub use self::mock::{backend_version, default_adapter};
    } else if #[cfg(feature = "web")] {
        pub use self::web::{backend_version, default_adapter};
    } else if #[cfg(feature = "ohos")] {
        pub use self::ohos::{backend_version, default_adapter};
    } else if #[cfg(feature = "winrt")] {
        pub use self::winrt::{backend_version, default_adapter};
    } else if #[cfg(feature = "corebluetooth")] {
        pub use self::corebluetooth::{backend_version, default_adapter};
    } else if #[cfg(feature = "bluez")] {
        pub use self::bluez::{backend_version, default_adapter};
    } else if #[cfg(feature = "android")] {
        pub use self::android::{backend_version, default_adapter};
    } else {
        compile_error!(
            "rblue: no backend feature is enabled; enable one of \
             `mock` (default), `winrt`, `corebluetooth`, `bluez`, \
             `android`, `web`, `ohos`"
        );
    }
}

/// In-memory mock backend (feature `mock`, on by default).
///
/// Everything here is host-usable on every target: the mock is the baseline
/// for tests, doctests, and prototyping.
///
/// # Examples
///
/// Scripted scans and GATT against the mock, through the aggregate surface:
///
/// ```
/// use std::sync::Arc;
///
/// use futures::StreamExt;
/// use rblue::platform::mock::MockAdapter;
/// use rblue::prelude::*;
///
/// # fn main() -> Result<(), Error> {
/// # futures::executor::block_on(async {
/// let mock = MockAdapter::new();
/// let device = mock.add_device(
///     DeviceId::Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01], AddressType::RandomStatic),
///     "simulated-sensor",
/// );
/// device.set_advertisement_script(vec![AdvertisementData {
///     local_name: Some("simulated-sensor".to_owned()),
///     services: vec![uuid::Uuid::from_u128(0x180F)],
///     rssi: Some(-40),
///     ..AdvertisementData::default()
/// }]);
///
/// let adapter: Arc<dyn Adapter> = Arc::new(mock);
/// adapter.initialize().await?;
/// let mut events = adapter.events();
/// adapter.start_scan(&ScanFilter::default()).await?;
///
/// let AdapterEvent::DeviceDiscovered(discovered) =
///     events.next().await.expect("discovery")
/// else {
///     panic!("expected a discovery");
/// };
/// assert_eq!(discovered.id, *device.id());
///
/// // Advancing the script re-advertises with the scripted RSSI.
/// assert!(device.advance_script());
/// let AdapterEvent::DeviceUpdated(_, rssi) = events.next().await.expect("update")
/// else {
///     panic!("expected an update");
/// };
/// assert_eq!(rssi.as_i16(), -40);
///
/// adapter.stop_scan().await?;
/// # Ok::<(), Error>(())
/// # })?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "mock")]
pub mod mock {
    use std::sync::Arc;

    use rblue_api::{Adapter, Error};

    pub use rblue_mock::MockAdapter;

    /// A fresh mock adapter with an empty environment.
    ///
    /// Script the environment through [`MockAdapter`] before handing the
    /// adapter around; see the module example.
    ///
    /// # Errors
    ///
    /// Never fails; the signature matches the other backends.
    pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
        Ok(Arc::new(MockAdapter::new()))
    }

    /// Backend version probe. The mock is in-process, so there is no
    /// external component to fingerprint.
    #[must_use]
    pub fn backend_version() -> &'static str {
        "mock/in-memory"
    }
}

/// Windows WinRT backend (feature `winrt`, `target_os = "windows"`).
///
/// Shell over the forked btleplug `winrtble` implementation. See
/// [`winrt::default_adapter`] for the platform notes (Windows 10 1809+,
/// RPA devices must be observed by the watcher before connecting).
#[cfg(feature = "winrt")]
pub mod winrt {
    pub use rblue_winrt::{backend_version, default_adapter};
}

/// macOS / iOS CoreBluetooth backend (feature `corebluetooth`,
/// `target_vendor = "apple"`).
///
/// Shell over the forked btleplug corebluetooth implementation. Scanning
/// with a service-UUID filter is the only hardware-pushed-down filter;
/// iOS background scanning requires one.
#[cfg(feature = "corebluetooth")]
pub mod corebluetooth {
    pub use rblue_corebluetooth::{backend_version, default_adapter};
}

/// Linux BlueZ backend (feature `bluez`, `target_os = "linux"` outside the
/// OHOS environment).
///
/// Shell over the forked btleplug `bluez-async` implementation, talking to
/// `bluetoothd` over D-Bus.
#[cfg(feature = "bluez")]
pub mod bluez {
    pub use rblue_bluez::{backend_version, default_adapter};
}

/// Android JNI backend (feature `android`, `target_os = "android"`).
///
/// Shell over the forked btleplug droidplug implementation. Initialization
/// must run on a thread that can see the application `ClassLoader` (the
/// binding layer's `Rblue.initialize(context)` wrapper guarantees it).
#[cfg(feature = "android")]
pub mod android {
    pub use rblue_android::{backend_version, default_adapter};

    /// JNI trampoline for `com.rblue.RblueNative.nativeInit()`.
    ///
    /// Consuming cdylibs define the `#[no_mangle]` symbol and call this so
    /// rustc exports it from the shared library.
    ///
    /// # Safety
    ///
    /// `env` must be a valid, non-null `JNIEnv*` for the current thread, which
    /// must be attached to the JVM for the duration of the call.
    /// `class` is the JNI class argument and is unused.
    #[cfg(target_os = "android")]
    #[doc(hidden)]
    pub unsafe fn jni_native_init(
        env: *mut std::ffi::c_void,
        class: *mut std::ffi::c_void,
    ) -> bool {
        unsafe { rblue_android::native_init_from_raw(env, class) }
    }
}

/// Web Bluetooth backend (feature `web`, `target_arch = "wasm32"`).
///
/// On wasm32 this is the real `navigator.bluetooth` implementation; on other
/// targets the submodule compiles but [`web::default_adapter`] fails with
/// [`crate::Error::Unsupported`] — it exists so host builds of web-dependent code
/// type-check.
#[cfg(feature = "web")]
pub mod web {
    pub use rblue_web::{backend_version, default_adapter};
}

/// HarmonyOS backend (feature `ohos`, Driver track).
///
/// The adapter is driven by an ArkTS `BleDriver` injected into a
/// [`DriverRegistry`](self::DriverRegistry). [`default_adapter`](self::default_adapter)
/// builds one over a fresh, driver-less registry: every operation fails
/// (request timeout) until a driver implementation registers, which makes it
/// useful for host tests of code shaped around the adapter only. A real OHOS
/// app obtains its adapter through the Node-API facade (`rblue-ohos` `napi`
/// feature), which owns the process-wide registry wired to ArkTS.
#[cfg(feature = "ohos")]
pub mod ohos {
    use std::sync::Arc;

    use rblue_api::{Adapter, Error};

    pub use rblue_ohos::{DriverRegistry, OhosAdapter, OhosPeripheral};

    /// An adapter over a fresh [`DriverRegistry`] with no driver injected.
    ///
    /// # Errors
    ///
    /// Never fails; operations fail at run time until a `BleDriver`
    /// implementation is registered.
    pub fn default_adapter() -> Result<Arc<dyn Adapter>, Error> {
        Ok(Arc::new(OhosAdapter::new(&DriverRegistry::new())))
    }

    /// Backend version probe. The Driver track identifies itself through the
    /// protocol version handshake instead of a build fingerprint.
    #[must_use]
    pub fn backend_version() -> &'static str {
        "ohos/driver-track"
    }
}
