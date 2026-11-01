# rblue_flutter

Local Flutter plugin for the rblue example. It packages the prebuilt
`rblue-c` C ABI (`crates/rblue-c`) and exposes a typed Dart client:

- `lib/src/ffi_bindings.dart` - ffigen output over
  [`bindings/c/include/rblue.h`](../../../bindings/c/include/rblue.h);
  regenerated with `cd examples/flutter && fvm dart run ffigen --config ffigen.yaml`,
  never hand-edited.
- `lib/src/native_bridge.dart` - loads the symbols (`DynamicLibrary.open` on
  Android, `DynamicLibrary.process()` on iOS, where the static library is
  force_load-ed into the Runner binary) and installs the two callbacks as
  `NativeCallable.listener` (they fire on Rust worker threads; `isolateLocal`
  is not allowed).
- `lib/src/client.dart` - `RblueClient`: `req_id` -> `Completer` correlation,
  `events`/`notifications` broadcast streams, error parsing, `NativeFinalizer`
  backstop plus explicit `dispose()`.
- `lib/src/models.dart` - Dart models of the serde JSON contracts
  (`DeviceId`, `AdapterEvent`, `ServiceSet`, `Notification`, `Error`, ...),
  kept in sync with the Rust tests in `crates/rblue-c`.

## Native libraries

The plugin does not build anything itself. `examples/flutter/tools/build-libs.sh`
cross-builds the libraries and stages them here:

- `android/src/main/jniLibs/<abi>/librblue_c.so` - 4 ABIs, packaged by gradle.
- `ios/Libs/ios{,-sim}/librblue_c.a` - force_load-ed by the podspec per SDK.

Both directories are gitignored; the canonical artifacts live in
`bindings/c/{android,ios}`.

## Android initialization discipline (required)

The Android backend (`rblue-android`) needs a Java-side JNI init before the
first adapter operation: the btleplug fork resolves its helper classes through
the calling thread's `ClassLoader`, which a Rust worker thread cannot do. The
symbol `Java_com_rblue_RblueNative_nativeInit` is exported by
`librblue_c.so` itself and is invoked from Kotlin:

```kotlin
// package com.rblue - must match the mangled symbol
object RblueNative {
    init { System.loadLibrary("rblue_c") }
    external fun nativeInit(): Boolean  // idempotent
}
```

`RblueFlutterPlugin` calls it in `onAttachedToEngine` and also handles the
`initialize` method of channel `rblue_flutter`. `RblueClient.create()`
awaits that method before `rblue_new`, because the Flutter engine may start
Dart before the plugin attaches; the backend init is idempotent, so the two
calls never conflict. If `nativeInit()` returns false, the client throws with
an explanation (check logcat for the `rblue-android` error record).

iOS has no such step; the same `initialize` call simply succeeds there.

## Permissions

The channel also exposes `requestBlePermissions()` (Android runtime
permissions; a no-op on iOS, where the CoreBluetooth prompt is driven by the
`NSBluetoothAlwaysUsageDescription` in the app's Info.plist). The example app
calls it before `initialize`.
