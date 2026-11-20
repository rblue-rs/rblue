# rblue_flutter

> Early experimental preview. The API, wire contract, native binaries, and
> supported targets may change before a stable release.

`rblue_flutter` packages the experimental Flutter binding maintained in
[`rblue-rs/rblue`](https://github.com/rblue-rs/rblue). It wraps rblue's
message-passing C ABI with a typed Dart client for Bluetooth Low Energy.

## Current support

| Platform | Packaged target | Minimum | Packaging validation |
| --- | --- | --- | --- |
| Android | arm64-v8a, armeabi-v7a, x86_64 | API 24 | Debug and release APK builds |
| iOS device | arm64 | iOS 15 | CocoaPods and SwiftPM unsigned builds |
| iOS simulator | arm64 (Apple Silicon) | iOS 15 | Packaging/UI only; no BLE radio acceptance |
| macOS | arm64 and x86_64 universal | macOS 12 | Debug/release Flutter and SwiftPM builds |
| Windows | x64 MSVC | Windows 10 1809 | Rust DLL cross-build, exports, and CMake packaging |

Linux, web, Intel iOS simulators, Windows ARM64, and HarmonyOS are **not**
declared by this Flutter package. The Rust project has additional backends, but
platform support is declared here only after Flutter packaging exists.

Implemented operations include adapter initialization, scan/start/stop,
peripheral lookup, connect/disconnect, service discovery, characteristic
read/write, subscribe/unsubscribe, RSSI, MTU query, maximum write length,
capabilities, connection state, Apple system-connected peripheral lookup, and connection priority.

## Installation

```yaml
dependencies:
  rblue_flutter: ^0.0.1-dev.2
```

The Pub package contains the native libraries. Consumers do not need Rust,
Cargo, or a checkout of the rblue monorepo.

## App setup

### Android

The plugin manifest contributes the BLE permissions. Request runtime permission
before creating the client:

```dart
final granted = await RbluePluginChannel.instance.requestBlePermissions();
if (!granted) {
  throw StateError('Bluetooth permission was not granted');
}
final client = await RblueClient.create();
await client.initialize();
```

The consuming app must use `minSdk >= 24`. On Android 12+ the plugin requests
`BLUETOOTH_SCAN` and `BLUETOOTH_CONNECT`; on older Android it requests fine
location. The bundled manifest uses Android's `neverForLocation` assertion by
default, which can filter some BLE beacons. Apps deriving physical location or
scanning those beacons must override that manifest policy and manage the
additional location permission themselves.

### iOS

Set an honest Bluetooth explanation in the consuming app's `Info.plist`:

```xml
<key>NSBluetoothAlwaysUsageDescription</key>
<string>This app uses Bluetooth to communicate with nearby devices.</string>
```

The plugin supports CocoaPods and Swift Package Manager. Flutter selects the
configured dependency manager; both package the same arm64 device and Apple
Silicon simulator C ABI. Intel simulator builds are unsupported.

### macOS

Set `NSBluetoothAlwaysUsageDescription` in the app's `Info.plist`. Sandboxed
apps also need this entitlement:

```xml
<key>com.apple.security.device.bluetooth</key>
<true/>
```

The packaged static library is universal (`arm64` and `x86_64`). CocoaPods and
Swift Package Manager are both supported.

### Windows

The package bundles an x64 MSVC `rblue_c.dll` beside the Flutter runner. It
requires Windows 10 version 1809 (build 17763) or newer. Full-trust desktop apps
do not need a runtime permission prompt. MSIX/AppContainer packaging must add a
Bluetooth device capability to its manifest, for example:

```xml
<DeviceCapability Name="bluetooth" />
```

## Minimal scan

```dart
final client = await RblueClient.create();
final subscription = client.events.listen((event) {
  // Inspect AdapterEvent values here.
});

await client.initialize();
await client.startScan();

// Later:
await client.stopScan();
await subscription.cancel();
client.dispose();
```

Always call `dispose()`. Native callback listeners keep the client reachable, so
it cannot rely on garbage collection for teardown. Disposal stops native
producers, drains every queued ownership transfer, and then releases the
callback listeners and native handle. Subscribe to event and notification
streams before starting operations if the first event must not be missed.

A complete four-platform app is under [`example/`](example/). It demonstrates
scan, GATT read/write/notify, and host-managed reconnect behavior.

## Native packaging and provenance

This release packages prebuilt `rblue-c` binaries so an installed Pub package
works without reaching outside itself:

- Android: `android/src/main/jniLibs/<abi>/librblue_c.so`;
- iOS CocoaPods and SwiftPM: `ios/rblue_flutter/RblueC.xcframework`;
- macOS CocoaPods and SwiftPM: `macos/rblue_flutter/RblueC.xcframework`;
- Windows: `windows/x64/rblue_c.dll`;
- Android btleplug Java/JNI helpers: `android/src/main/java/`.

The corresponding Rust source snapshot, pinned lockfile, exact build
instructions, checksums, and exported-symbol checks are included under
[`native/`](native/). This is for reproducibility and license compliance;
normal Flutter builds consume the packaged binaries rather than compiling Rust.

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) for dependency
attribution and license texts. The package itself is dual-licensed under MIT or
Apache-2.0; the complete texts are embedded in [`LICENSE`](LICENSE) so automated
package analysis can recognize them.

## Known limitations

- CI and local checks validate packaging and linking; physical BLE acceptance is
  still required on Android, iOS, macOS, and Windows hardware.
- Reconnect policy lives in Dart. Rust `ConnectionSupervisor` and GATT queue
  behavior are not exported by the current C ABI.
- Notification streams are broadcast streams without built-in bounded
  backpressure. Production consumers must define an aggregation/drop policy.
- GATT characteristic instance IDs are connection-scoped and must be
  rediscovered after reconnecting.
- Pairing, descriptor read/write, and explicit Android MTU requests are not
  exposed yet.
- Some Android failures are collapsed by the pinned btleplug fork to
  `NotConnected`; raw GATT status is preserved only when the backend supplies it.

Please report reproducible issues at
<https://github.com/rblue-rs/rblue/issues>.
