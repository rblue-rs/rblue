import 'dart:io' show Platform;

import 'package:flutter/services.dart';

/// The plugin's method channel for Android-only host setup.
///
/// The Android backend requires Java-side JNI initialization before the first
/// Dart FFI call and runtime BLE permission handling. Apple and Windows builds
/// call the C ABI directly, so their setup and permission methods are no-ops.
class RbluePluginChannel {
  /// The channel used for the initialization handshake and the Android BLE
  /// runtime-permission request.
  static const methodChannel = MethodChannel('rblue_flutter');

  RbluePluginChannel._();

  static final RbluePluginChannel instance = RbluePluginChannel._();

  /// Runs the Android JNI initialization handshake.
  Future<void> initialize() async {
    if (!Platform.isAndroid) return;
    final ok = await methodChannel.invokeMethod<bool>('initialize');
    if (ok != true) {
      throw StateError(
        'rblue_flutter: Android JNI initialization failed; '
        'RblueNative.nativeInit() returned false - check logcat for the '
        'rblue-android error record',
      );
    }
  }

  /// Requests Android BLE runtime permissions. Other native platforms grant
  /// access through their operating-system Bluetooth flows.
  Future<bool> requestBlePermissions() async {
    if (!Platform.isAndroid) return true;
    final granted = await methodChannel.invokeMethod<bool>(
      'requestBlePermissions',
    );
    return granted ?? false;
  }
}
