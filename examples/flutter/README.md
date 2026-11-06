# rblue Flutter example

One Flutter codebase for Android and iOS, backed by the `rblue-c` message-passing
C ABI. It demonstrates:

- BLE permission and adapter initialization;
- scanning with an RSSI-sorted, identity-deduplicated device list;
- connecting and rediscovering the per-connection GATT tree;
- reads, writes with/without response, and the unified `InvalidLength` behavior;
- notification subscriptions and a value console;
- visible host-side reconnect backoff, including Android GATT status 133;
- platform capability and privacy notes.

Use `fvm flutter` / `fvm dart` from `examples/flutter`; `.fvmrc` pins the SDK
without changing the global Flutter installation. VS Code uses `.fvm/flutter_sdk`.

The app talks to the local `rblue_flutter` plugin. The plugin uses ffigen bindings
over [`bindings/c/include/rblue.h`](../../bindings/c/include/rblue.h) and packages
prebuilt `rblue-c` libraries for four Android ABIs plus iOS device/simulator.

## Prerequisites

- Rust stable and the mobile targets listed below;
- FVM with the project-pinned Flutter 3.47.9 (Dart 3.13.7);
- Android SDK + NDK 27/28 and Java 17;
- Xcode on macOS for iOS builds;
- a physical Android/iOS device for Bluetooth testing (simulators do not expose
  real BLE hardware).

```bash
rustup target add \
  aarch64-linux-android armv7-linux-androideabi \
  i686-linux-android x86_64-linux-android \
  aarch64-apple-ios aarch64-apple-ios-sim
```

## Build the native libraries

From the repository root:

```bash
# Both platforms (macOS only):
examples/flutter/tools/build-libs.sh

# One platform, as used by CI:
examples/flutter/tools/build-libs.sh --android-only
examples/flutter/tools/build-libs.sh --ios-only
```

The script auto-detects common NDK locations. Override with
`ANDROID_NDK_HOME=/path/to/ndk`; `ANDROID_API` defaults to 24. Build artifacts
are staged into the plugin and `bindings/c/` but are gitignored.

When the C header changes, regenerate the checked-in Dart binding:

```bash
cd examples/flutter
fvm flutter pub get
fvm dart run ffigen --config ffigen.yaml
```

Never edit `rblue_flutter/lib/src/ffi_bindings.dart` by hand.

## Mobile M3 Expressive UI

The native Flutter UI implements the mobile prototype: five persistent destinations
(设备 / 数据流 / 重连 / 环境 / 日志), device drill-in with search state retained,
light/dark ColorScheme roles, selective emphasized typography, spring-driven button
shape changes and adjacent-button squish. Reduced-motion preferences apply changes
without a spring delay; large text uses independent stacked actions.

Scan, connect, GATT discovery, HEX / UTF-8 writes, RSSI / MTU / max-write-length
queries, notifications and reconnect use the existing rblue-c client. There are no
hardcoded demo devices or simulated GATT errors in the application. Reconnection
can restore the subscriptions captured when monitoring starts, matching fresh IDs
from the rediscovered service tree.

Notifications retain 200 packets and operations retain 300 logs. **复制会话 JSON**
exports a snapshot through the system clipboard (not a saved file). The snapshot
may include device identifiers and payloads; storage is in memory only.

## Run and test

```bash
cd examples/flutter
fvm use 3.47.9
fvm flutter --version
fvm flutter analyze
fvm flutter test
fvm flutter run                    # choose a connected physical device
fvm flutter build apk --debug
fvm flutter build ios --no-codesign
```

On Android, the plugin calls `com.rblue.RblueNative.nativeInit()` from the
engine thread before `rblue_new`; this gives the btleplug Java helpers the
correct application ClassLoader. The call is idempotent. The manifest requests
`BLUETOOTH_SCAN`/`BLUETOOTH_CONNECT` on API 31+ and legacy Bluetooth/location
permissions through API 30.

On iOS, `NSBluetoothAlwaysUsageDescription` is present. The static Rust library
is force-loaded by the podspec, which also links CoreBluetooth, Foundation,
`libobjc`, and `libc++` into the consuming Runner target.

## GitHub Release assets

An `examples-v*` tag runs `.github/workflows/release-examples.yml` and publishes:

- a debug-signed Android APK;
- an Apple Silicon iOS simulator `.app.zip`;
- an unsigned arm64 device `.app.zip` (a build artifact, not an installable IPA).

The Android debug key may change between GitHub runners; uninstall the previous
example if an in-place upgrade is rejected. Signed store distribution is out of
scope for the example workflow.

## Physical-device acceptance

1. Install the app on a phone and grant Bluetooth permissions.
2. Start a scan and confirm discoveries are RSSI-sorted without duplicate ids.
3. Open a device, connect, discover its services, and read a readable value.
4. Write a normal payload, then run the oversize write validation (max write length + 1 when available) and verify
   it reports `InvalidLength` rather than truncating.
5. Subscribe and verify notifications appear in the console.
6. Open **Reconnect**, power the peripheral off/on, and observe exponential
   retries; on Android, status 133 remains visible as the original platform
   code.

No automated test claims real radio coverage. The CI workflow validates Rust,
Dart, Android packaging, and the no-codesign iOS link; hardware acceptance is a
manual/self-hosted step.

See [KNOWN-GAPS.md](KNOWN-GAPS.md) for deliberate limitations.
