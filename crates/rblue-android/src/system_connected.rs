//! System-connected peripheral lookup for Android.
//!
//! The forked droidplug backend does not implement `retrieve_peripherals`.
//! The shell queries `BluetoothManager.getConnectedDevices(GATT)` over JNI,
//! filters by `BluetoothDevice.getUuids()`, and then asks the fork to
//! `add_peripheral` so the caller can `connect()` locally. Vendored fork
//! Java is left untouched.

use uuid::Uuid;

/// A GATT-connected device reported by the Android system, with any cached
/// service UUIDs from `BluetoothDevice.getUuids()`.
///
/// Used by the Android adapter (and unit tests). Host builds of this crate
/// compile the helpers so the matching rules stay testable.
#[allow(dead_code)] // constructed by the Android JNI query
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SystemConnectedDevice {
    pub address: [u8; 6],
    pub name: Option<String>,
    pub services: Vec<Uuid>,
}

/// Parses a colon-separated Android MAC (`AA:BB:CC:DD:EE:FF`).
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) fn parse_android_address(address: &str) -> Option<[u8; 6]> {
    let mut bytes = [0u8; 6];
    let mut parts = address.split(':');
    for byte in &mut bytes {
        let part = parts.next()?;
        if part.len() != 2 {
            return None;
        }
        *byte = u8::from_str_radix(part, 16).ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(bytes)
}

/// Strict service match: a device is included only when its cached UUID
/// list is nonempty and intersects `requested`. Devices with no cached
/// UUIDs are excluded rather than returned as false positives.
#[cfg_attr(not(any(test, target_os = "android")), allow(dead_code))]
pub(crate) fn matches_requested_services(cached: &[Uuid], requested: &[Uuid]) -> bool {
    !cached.is_empty() && cached.iter().any(|uuid| requested.contains(uuid))
}

#[cfg(target_os = "android")]
pub(crate) fn query_gatt_connected() -> Result<Vec<SystemConnectedDevice>, rblue_api::Error> {
    use rblue_api::error::{Error, PermissionReason};

    const GATT_PROFILE: i32 = 7; // BluetoothProfile.GATT

    let vm = jni::JavaVM::singleton()
        .map_err(|err| Error::Backend(format!("android system-connected: no Java VM: {err}")))?;
    vm.attach_current_thread(|env| {
        Ok::<_, jni::errors::Error>(match query_gatt_connected_with_env(env, GATT_PROFILE) {
            Ok(devices) => Ok(devices),
            Err(QueryError::Permission) => Err(Error::Permission(
                PermissionReason::BluetoothPermissionDenied,
            )),
            Err(QueryError::BluetoothOff) => Err(Error::Permission(PermissionReason::BluetoothOff)),
            Err(QueryError::Failed(message)) => Err(Error::Backend(message)),
        })
    })
    .map_err(|err| Error::Backend(format!("android system-connected: JNI attach: {err}")))?
}

#[cfg(target_os = "android")]
enum QueryError {
    Permission,
    BluetoothOff,
    Failed(String),
}

#[cfg(target_os = "android")]
fn query_gatt_connected_with_env(
    env: &mut jni::Env<'_>,
    profile: i32,
) -> Result<Vec<SystemConnectedDevice>, QueryError> {
    use jni::objects::JValue;
    use jni::{jni_sig, jni_str};

    let context = match env.call_static_method(
        jni_str!("android/app/ActivityThread"),
        jni_str!("currentApplication"),
        jni_sig!("()Landroid/app/Application;"),
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "ActivityThread.currentApplication"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };
    if context.is_null() {
        return Err(QueryError::Failed(
            "android system-connected: application context is unavailable".into(),
        ));
    }

    let service_name = env
        .new_string("bluetooth")
        .map_err(|err| QueryError::Failed(err.to_string()))?;
    let manager = match env.call_method(
        &context,
        jni_str!("getSystemService"),
        jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
        &[(&service_name).into()],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "Context.getSystemService"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };
    if manager.is_null() {
        return Err(QueryError::BluetoothOff);
    }

    let list = match env.call_method(
        &manager,
        jni_str!("getConnectedDevices"),
        jni_sig!("(I)Ljava/util/List;"),
        &[JValue::from(profile)],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(
                env,
                "BluetoothManager.getConnectedDevices",
            ));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };
    if list.is_null() {
        return Ok(Vec::new());
    }

    let size = match env.call_method(&list, jni_str!("size"), jni_sig!("()I"), &[]) {
        Ok(value) => value
            .i()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "List.size"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };

    let mut devices = Vec::new();
    for index in 0..size {
        let device = match env.call_method(
            &list,
            jni_str!("get"),
            jni_sig!("(I)Ljava/lang/Object;"),
            &[JValue::from(index)],
        ) {
            Ok(value) => value
                .l()
                .map_err(|err| QueryError::Failed(err.to_string()))?,
            Err(jni::errors::Error::JavaException) => {
                return Err(map_java_exception(env, "List.get"));
            }
            Err(err) => return Err(QueryError::Failed(err.to_string())),
        };
        if device.is_null() {
            continue;
        }
        if let Some(parsed) = read_bluetooth_device(env, &device)? {
            devices.push(parsed);
        }
    }
    Ok(devices)
}

#[cfg(target_os = "android")]
fn read_bluetooth_device(
    env: &mut jni::Env<'_>,
    device: &jni::objects::JObject<'_>,
) -> Result<Option<SystemConnectedDevice>, QueryError> {
    use jni::{jni_sig, jni_str};

    let address = match env.call_method(
        device,
        jni_str!("getAddress"),
        jni_sig!("()Ljava/lang/String;"),
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothDevice.getAddress"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };
    if address.is_null() {
        return Ok(None);
    }
    let address = jstring_to_string(env, address)?;
    let Some(address) = parse_android_address(&address) else {
        return Ok(None);
    };

    let name = match env.call_method(
        device,
        jni_str!("getName"),
        jni_sig!("()Ljava/lang/String;"),
        &[],
    ) {
        Ok(value) => {
            let value = value
                .l()
                .map_err(|err| QueryError::Failed(err.to_string()))?;
            if value.is_null() {
                None
            } else {
                Some(jstring_to_string(env, value)?)
            }
        }
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothDevice.getName"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };

    let services = read_cached_uuids(env, device)?;
    Ok(Some(SystemConnectedDevice {
        address,
        name,
        services,
    }))
}

#[cfg(target_os = "android")]
fn read_cached_uuids(
    env: &mut jni::Env<'_>,
    device: &jni::objects::JObject<'_>,
) -> Result<Vec<Uuid>, QueryError> {
    use jni::objects::JObjectArray;
    use jni::{jni_sig, jni_str};

    let uuids = match env.call_method(
        device,
        jni_str!("getUuids"),
        jni_sig!("()[Landroid/os/ParcelUuid;"),
        &[],
    ) {
        Ok(value) => value
            .l()
            .map_err(|err| QueryError::Failed(err.to_string()))?,
        Err(jni::errors::Error::JavaException) => {
            return Err(map_java_exception(env, "BluetoothDevice.getUuids"));
        }
        Err(err) => return Err(QueryError::Failed(err.to_string())),
    };
    if uuids.is_null() {
        return Ok(Vec::new());
    }
    let array = env
        .cast_local::<JObjectArray<jni::objects::JObject>>(uuids)
        .map_err(|err| QueryError::Failed(err.to_string()))?;
    let len = array
        .len(env)
        .map_err(|err| QueryError::Failed(err.to_string()))?;
    let mut services = Vec::with_capacity(len);
    for index in 0..len {
        let parcel = array
            .get_element(env, index)
            .map_err(|err| QueryError::Failed(err.to_string()))?;
        if parcel.is_null() {
            continue;
        }
        let uuid_obj = match env.call_method(
            &parcel,
            jni_str!("getUuid"),
            jni_sig!("()Ljava/util/UUID;"),
            &[],
        ) {
            Ok(value) => value
                .l()
                .map_err(|err| QueryError::Failed(err.to_string()))?,
            Err(jni::errors::Error::JavaException) => {
                return Err(map_java_exception(env, "ParcelUuid.getUuid"));
            }
            Err(err) => return Err(QueryError::Failed(err.to_string())),
        };
        if uuid_obj.is_null() {
            continue;
        }
        let uuid_str = match env.call_method(
            &uuid_obj,
            jni_str!("toString"),
            jni_sig!("()Ljava/lang/String;"),
            &[],
        ) {
            Ok(value) => value
                .l()
                .map_err(|err| QueryError::Failed(err.to_string()))?,
            Err(jni::errors::Error::JavaException) => {
                return Err(map_java_exception(env, "UUID.toString"));
            }
            Err(err) => return Err(QueryError::Failed(err.to_string())),
        };
        if uuid_str.is_null() {
            continue;
        }
        let uuid_str = jstring_to_string(env, uuid_str)?;
        if let Ok(uuid) = Uuid::parse_str(&uuid_str) {
            services.push(uuid);
        }
    }
    Ok(services)
}

#[cfg(target_os = "android")]
fn jstring_to_string(
    env: &mut jni::Env<'_>,
    value: jni::objects::JObject<'_>,
) -> Result<String, QueryError> {
    use jni::objects::JString;
    let jstr = env
        .cast_local::<JString>(value)
        .map_err(|err| QueryError::Failed(err.to_string()))?;
    jstr.mutf8_chars(env)
        .map(String::from)
        .map_err(|err| QueryError::Failed(err.to_string()))
}

#[cfg(target_os = "android")]
fn map_java_exception(env: &mut jni::Env<'_>, what: &str) -> QueryError {
    use jni::jni_str;
    let occurred = env.exception_occurred();
    env.exception_clear();
    let Some(exception) = occurred else {
        return QueryError::Failed(format!("{what}: Java exception"));
    };
    if exception.is_null() {
        return QueryError::Failed(format!("{what}: Java exception"));
    }
    let is_security = env
        .is_instance_of(&exception, jni_str!("java/lang/SecurityException"))
        .unwrap_or(false);
    if is_security {
        QueryError::Permission
    } else {
        QueryError::Failed(format!("{what}: Java exception"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::uuid;

    const SENSOR: Uuid = uuid!("0000180f-0000-1000-8000-00805f9b34fb");
    const BATTERY: Uuid = uuid!("0000180a-0000-1000-8000-00805f9b34fb");

    #[test]
    fn parse_android_address_accepts_colon_hex() {
        assert_eq!(
            parse_android_address("AA:BB:0C:DD:EE:FF"),
            Some([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0xFF])
        );
        assert_eq!(
            parse_android_address("aa:bb:0c:dd:ee:ff"),
            Some([0xAA, 0xBB, 0x0C, 0xDD, 0xEE, 0xFF])
        );
        assert_eq!(parse_android_address("AA:BB:CC:DD:EE"), None);
        assert_eq!(parse_android_address("AABBCCDDEEFF"), None);
        assert_eq!(parse_android_address("AA:BB:CC:DD:EE:FFF"), None);
    }

    #[test]
    fn matches_requested_services_is_strict() {
        assert!(matches_requested_services(&[SENSOR, BATTERY], &[SENSOR]));
        assert!(!matches_requested_services(&[], &[SENSOR]));
        assert!(!matches_requested_services(&[BATTERY], &[SENSOR]));
    }
}
