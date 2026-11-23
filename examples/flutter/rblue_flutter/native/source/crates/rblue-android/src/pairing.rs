//! Shell-side Android pairing via `BluetoothDevice.createBond` /
//! hidden `removeBond`.
//!
//! The forked droidplug backend does not expose these APIs on the
//! `Peripheral` trait. The shell talks to the platform adapter over JNI
//! and leaves vendored fork Java untouched. Address formatting and
//! bond-state outcomes are host-testable; the JNI calls compile only for
//! Android targets.

use rblue_api::error::Error;

/// `BluetoothDevice.BOND_NONE`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) const BOND_NONE: i32 = 10;
/// `BluetoothDevice.BOND_BONDING`.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) const BOND_BONDING: i32 = 11;
/// `BluetoothDevice.BOND_BONDED`.
pub(crate) const BOND_BONDED: i32 = 12;

/// Formats a MAC as Android's colon-separated uppercase hex.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) fn format_android_address(address: [u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        address[0], address[1], address[2], address[3], address[4], address[5]
    )
}

/// Outcome of waiting for a bond-state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) enum BondWait {
    /// Reached `BOND_BONDED`.
    Bonded,
    /// Reached `BOND_NONE`.
    None,
    /// Still bonding (or unknown) when the deadline expired.
    TimedOut,
}

/// Maps a finished `pair` wait onto the unified error surface.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) fn interpret_pair_wait(outcome: BondWait) -> Result<(), Error> {
    use rblue_api::error::PairingStatus;
    match outcome {
        BondWait::Bonded => Ok(()),
        BondWait::None => Err(Error::Pairing(PairingStatus::Rejected)),
        BondWait::TimedOut => Err(Error::Pairing(PairingStatus::TimedOut)),
    }
}

/// Maps a finished `unpair` wait onto the unified error surface.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) fn interpret_unpair_wait(outcome: BondWait) -> Result<(), Error> {
    use rblue_api::error::PairingStatus;
    match outcome {
        BondWait::None => Ok(()),
        BondWait::Bonded => Err(Error::Pairing(PairingStatus::Other(BOND_BONDED))),
        BondWait::TimedOut => Err(Error::Pairing(PairingStatus::TimedOut)),
    }
}

#[cfg(target_os = "android")]
pub(crate) async fn pair(address: [u8; 6]) -> Result<(), Error> {
    spawn_blocking("pair", move || pair_blocking(address)).await
}

#[cfg(target_os = "android")]
pub(crate) async fn unpair(address: [u8; 6]) -> Result<(), Error> {
    spawn_blocking("unpair", move || unpair_blocking(address)).await
}

#[cfg(target_os = "android")]
async fn spawn_blocking(
    feature: &'static str,
    work: impl FnOnce() -> Result<(), Error> + Send + 'static,
) -> Result<(), Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| Error::Backend(format!("android {feature}: join: {err}")))?
}

#[cfg(target_os = "android")]
fn pair_blocking(address: [u8; 6]) -> Result<(), Error> {
    let device = remote_device(address)?;
    match device.bond_state()? {
        BOND_BONDED => return Ok(()),
        BOND_BONDING => return interpret_pair_wait(device.wait_until_settled()),
        _ => {}
    }
    if !device.create_bond()? {
        // createBond is documented as returning false when a bond already
        // exists or a procedure is already running; re-check state.
        return interpret_pair_wait(match device.bond_state()? {
            BOND_BONDED => BondWait::Bonded,
            BOND_BONDING => device.wait_until_settled(),
            _ => BondWait::None,
        });
    }
    interpret_pair_wait(device.wait_until_settled())
}

#[cfg(target_os = "android")]
fn unpair_blocking(address: [u8; 6]) -> Result<(), Error> {
    let device = remote_device(address)?;
    match device.bond_state()? {
        BOND_NONE => return Ok(()),
        BOND_BONDING => {
            // Wait for the in-flight procedure before removing the bond.
            match device.wait_until_settled() {
                BondWait::None => return Ok(()),
                BondWait::TimedOut => {
                    return Err(Error::Pairing(rblue_api::error::PairingStatus::TimedOut));
                }
                BondWait::Bonded => {}
            }
        }
        _ => {}
    }
    if !device.remove_bond()? {
        return interpret_unpair_wait(match device.bond_state()? {
            BOND_NONE => BondWait::None,
            _ => BondWait::Bonded,
        });
    }
    interpret_unpair_wait(device.wait_until_settled())
}

#[cfg(target_os = "android")]
fn remote_device(address: [u8; 6]) -> Result<BondedDevice, Error> {
    if !crate::jni_initialized() {
        return Err(Error::InvalidState(
            "backend not initialized; call rblue_android::init() with the application JNIEnv first"
                .into(),
        ));
    }

    // The JNI work has to stay inside attach_current_thread. BondedDevice
    // re-attaches for each JNI burst (state / createBond / removeBond).
    Ok(BondedDevice { address })
}

#[cfg(target_os = "android")]
struct BondedDevice {
    address: [u8; 6],
}

#[cfg(target_os = "android")]
impl BondedDevice {
    fn with_device<T>(
        &self,
        what: &'static str,
        f: impl for<'a> FnOnce(&mut jni::Env<'a>, &jni::objects::JObject<'a>) -> Result<T, PairingJni>,
    ) -> Result<T, Error> {
        let vm = jni::JavaVM::singleton()
            .map_err(|err| Error::Backend(format!("android pairing: no Java VM: {err}")))?;
        vm.attach_current_thread(|env| {
            Ok::<_, jni::errors::Error>(
                (|| {
                    let device = lookup_remote_device(env, self.address)?;
                    f(env, &device)
                })()
                .map_err(|err| err.into_error(what)),
            )
        })
        .map_err(|err| Error::Backend(format!("android pairing: JNI attach: {err}")))?
    }

    fn bond_state(&self) -> Result<i32, Error> {
        self.with_device("BluetoothDevice.getBondState", |env, device| {
            use jni::{jni_sig, jni_str};
            match env.call_method(device, jni_str!("getBondState"), jni_sig!("()I"), &[]) {
                Ok(value) => value.i().map_err(|err| PairingJni::Failed(err.to_string())),
                Err(jni::errors::Error::JavaException) => {
                    Err(map_java_exception(env, "BluetoothDevice.getBondState"))
                }
                Err(err) => Err(PairingJni::Failed(err.to_string())),
            }
        })
    }

    fn create_bond(&self) -> Result<bool, Error> {
        self.with_device("BluetoothDevice.createBond", |env, device| {
            use jni::{jni_sig, jni_str};
            match env.call_method(device, jni_str!("createBond"), jni_sig!("()Z"), &[]) {
                Ok(value) => value.z().map_err(|err| PairingJni::Failed(err.to_string())),
                Err(jni::errors::Error::JavaException) => {
                    Err(map_java_exception(env, "BluetoothDevice.createBond"))
                }
                Err(err) => Err(PairingJni::Failed(err.to_string())),
            }
        })
    }

    fn remove_bond(&self) -> Result<bool, Error> {
        self.with_device("BluetoothDevice.removeBond", |env, device| {
            use jni::{jni_sig, jni_str};
            match env.call_method(device, jni_str!("removeBond"), jni_sig!("()Z"), &[]) {
                Ok(value) => value.z().map_err(|err| PairingJni::Failed(err.to_string())),
                Err(jni::errors::Error::JavaException) => {
                    Err(map_java_exception(env, "BluetoothDevice.removeBond"))
                }
                Err(err) => Err(PairingJni::Failed(err.to_string())),
            }
        })
    }

    fn wait_until_settled(&self) -> BondWait {
        const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        const POLL: std::time::Duration = std::time::Duration::from_millis(200);
        let deadline = std::time::Instant::now() + TIMEOUT;
        loop {
            match self.bond_state() {
                Ok(BOND_BONDED) => return BondWait::Bonded,
                Ok(BOND_NONE) => return BondWait::None,
                Ok(_) => {}
                Err(_) => {}
            }
            if std::time::Instant::now() >= deadline {
                return BondWait::TimedOut;
            }
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(target_os = "android")]
enum PairingJni {
    Permission,
    BluetoothOff,
    Unsupported,
    Failed(String),
}

#[cfg(target_os = "android")]
impl PairingJni {
    fn into_error(self, what: &str) -> Error {
        use rblue_api::error::PermissionReason;
        match self {
            Self::Permission => Error::Permission(PermissionReason::BluetoothPermissionDenied),
            Self::BluetoothOff => Error::Permission(PermissionReason::BluetoothOff),
            Self::Unsupported => Error::Unsupported {
                platform: crate::mapping::PLATFORM,
                feature: if what.contains("removeBond") {
                    "unpair"
                } else {
                    "pair"
                },
            },
            Self::Failed(message) => Error::Backend(format!("{what}: {message}")),
        }
    }
}

#[cfg(target_os = "android")]
fn lookup_remote_device<'a>(
    env: &mut jni::Env<'a>,
    address: [u8; 6],
) -> Result<jni::objects::JObject<'a>, PairingJni> {
    use jni::{jni_sig, jni_str};

    let context = match env.call_static_method(
        jni_str!("android/app/ActivityThread"),
        jni_str!("currentApplication"),
        jni_sig!("()Landroid/app/Application;"),
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| PairingJni::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "ActivityThread.currentApplication"));
        }
        Err(err) => return Err(PairingJni::Failed(err.to_string())),
    };
    if context.is_null() {
        return Err(PairingJni::Failed(
            "android pairing: application context is unavailable".into(),
        ));
    }

    let service_name = env
        .new_string("bluetooth")
        .map_err(|err| PairingJni::Failed(err.to_string()))?;
    let manager = match env.call_method(
        &context,
        jni_str!("getSystemService"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
        &[(&service_name).into()],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| PairingJni::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "Context.getSystemService"));
        }
        Err(err) => return Err(PairingJni::Failed(err.to_string())),
    };
    if manager.is_null() {
        return Err(PairingJni::BluetoothOff);
    }

    let adapter = match env.call_method(
        &manager,
        jni_str!("getAdapter"),
        jni_sig!("()Landroid/bluetooth/BluetoothAdapter;"),
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| PairingJni::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothManager.getAdapter"));
        }
        Err(err) => return Err(PairingJni::Failed(err.to_string())),
    };
    if adapter.is_null() {
        return Err(PairingJni::BluetoothOff);
    }

    let enabled = match env.call_method(&adapter, jni_str!("isEnabled"), jni_sig!("()Z"), &[]) {
        Ok(value) => value
            .z()
            .map_err(|err| PairingJni::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothAdapter.isEnabled"));
        }
        Err(err) => return Err(PairingJni::Failed(err.to_string())),
    };
    if !enabled {
        return Err(PairingJni::BluetoothOff);
    }

    let mac = env
        .new_string(format_android_address(address))
        .map_err(|err| PairingJni::Failed(err.to_string()))?;
    let device = match env.call_method(
        &adapter,
        jni_str!("getRemoteDevice"),
        jni_sig!("(Ljava/lang/String;)Landroid/bluetooth/BluetoothDevice;"),
        &[(&mac).into()],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| PairingJni::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothAdapter.getRemoteDevice"));
        }
        Err(err) => return Err(PairingJni::Failed(err.to_string())),
    };
    if device.is_null() {
        return Err(PairingJni::Failed(
            "android pairing: getRemoteDevice returned null".into(),
        ));
    }
    Ok(device)
}

#[cfg(target_os = "android")]
fn map_java_exception(env: &mut jni::Env<'_>, what: &str) -> PairingJni {
    use jni::jni_str;
    let occurred = env.exception_occurred();
    env.exception_clear();
    let Some(exception) = occurred else {
        return PairingJni::Failed(format!("{what}: Java exception"));
    };
    if exception.is_null() {
        return PairingJni::Failed(format!("{what}: Java exception"));
    }
    if env
        .is_instance_of(&exception, jni_str!("java/lang/SecurityException"))
        .unwrap_or(false)
    {
        return PairingJni::Permission;
    }
    if env
        .is_instance_of(&exception, jni_str!("java/lang/NoSuchMethodError"))
        .unwrap_or(false)
        || env
            .is_instance_of(&exception, jni_str!("java/lang/NoSuchMethodException"))
            .unwrap_or(false)
    {
        return PairingJni::Unsupported;
    }
    PairingJni::Failed(format!("{what}: Java exception"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rblue_api::error::PairingStatus;

    #[test]
    fn format_android_address_is_colon_uppercase_hex() {
        assert_eq!(
            format_android_address([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0xFF]),
            "AA:BB:0C:DD:EE:FF"
        );
        assert_eq!(
            format_android_address([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]),
            "00:11:22:33:44:55"
        );
    }

    #[test]
    fn interpret_pair_wait_maps_bond_states() {
        assert!(interpret_pair_wait(BondWait::Bonded).is_ok());
        assert!(matches!(
            interpret_pair_wait(BondWait::None),
            Err(Error::Pairing(PairingStatus::Rejected))
        ));
        assert!(matches!(
            interpret_pair_wait(BondWait::TimedOut),
            Err(Error::Pairing(PairingStatus::TimedOut))
        ));
    }

    #[test]
    fn interpret_unpair_wait_maps_bond_states() {
        assert!(interpret_unpair_wait(BondWait::None).is_ok());
        assert!(matches!(
            interpret_unpair_wait(BondWait::Bonded),
            Err(Error::Pairing(PairingStatus::Other(BOND_BONDED)))
        ));
        assert!(matches!(
            interpret_unpair_wait(BondWait::TimedOut),
            Err(Error::Pairing(PairingStatus::TimedOut))
        ));
    }
}
