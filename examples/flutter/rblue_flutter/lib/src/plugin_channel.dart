import 'package:flutter/services.dart';

/// The plugin's method channel: host-side setup the C ABI cannot do itself.
///
/// Android: the JNI bridge of the `rblue-android` backend requires a Java-side
/// initialization before the first adapter operation. The plugin ships the
/// Kotlin class `com.rblue.RblueNative` (its `external fun nativeInit()`
/// resolves against `Java_com_rblue_RblueNative_nativeInit`, linked into
/// `librblue_c.so`). The plugin calls it when the engine attaches and this
/// channel re-asserts it before the first Dart FFI call - the engine may start
/// Dart before `onAttachedToEngine` runs, so the client must not rely on the
/// plugin-side call alone. The call is idempotent.
///
/// iOS: no initialization step exists; every call returns success.
class RbluePluginChannel {
  /// The channel used for the initialization handshake and the Android BLE
  /// runtime-permission request.
  static const methodChannel = MethodChannel('rblue_flutter');

  RbluePluginChannel._();

  static final RbluePluginChannel instance = RbluePluginChannel._();

  /// Runs the platform initialization; throws when it reports failure.
  Future<void> initialize() async {
    final ok = await methodChannel.invokeMethod<bool>('initialize');
    if (ok != true) {
      throw StateError(
        'rblue_flutter: platform initialization failed; on Android this means '
        'RblueNative.nativeInit() returned false - check logcat for the '
        'rblue-android error record',
      );
    }
  }

  /// Requests the Android BLE runtime permissions (BLUETOOTH_SCAN/CONNECT on
  /// API 31+, ACCESS_FINE_LOCATION on API 30 and below). Returns true when
  /// every requested permission was granted; on iOS always true.
  Future<bool> requestBlePermissions() async {
    final granted = await methodChannel.invokeMethod<bool>(
      'requestBlePermissions',
    );
    return granted ?? false;
  }
}
