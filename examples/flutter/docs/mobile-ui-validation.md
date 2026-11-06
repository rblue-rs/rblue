# Flutter mobile UI implementation / 2026-11-06

## Delivered

- Project FVM pin: Flutter 3.47.9, Dart 3.13.7; scoped `.fvmrc` and IDE SDK path.
- Existing Flutter CI and example release workflow use the same Flutter version.
- Actual C-ABI driven mobile flow: five persistent destinations, device search/filter,
  instance-aware GATT selection, HEX / UTF-8, both supported write modes,
  max-write-length validation, RSSI / MTU query, notifications and host reconnect.
- M3 color roles, emphasized headings, light/dark themes, native Material button
  semantics with spring shape changes and adjacent-action squish. Reduced motion
  and 2x text-size layout are covered by widget tests.
- Notification stream keeps 200 packets, logs keep 300 entries; pause freezes the
  display snapshot but not receiving. JSON export copies to the system clipboard.
- Reconnect optionally restores subscriptions captured when monitoring starts,
  using the newly discovered service/characteristic instance coordinates.

## Verification

- App: `fvm flutter analyze` passes; 13 tests (4 prior, 9 added) pass.
- Local plugin: analyze and 17 tests pass on the project SDK.
- Rust C ABI: 22 tests and `cargo clippy -p rblue-c --all-targets -- -D warnings` pass.
- `cargo check --workspace` passes.
- Native libraries rebuilt/staged for four Android ABIs and both Apple Silicon iOS slices.
- Android debug APK, iOS simulator debug and unsigned arm64 iOS device builds pass.
- iOS simulator installed and launched: UI displays the actual Unsupported adapter
  error instead of crashing or generating simulated devices. Screenshot is in
  gitignored `output/ios-simulator.png`.

## Startup fix required during verification

Native launch exposed a pre-existing C-ABI startup panic in `rblue_set_event_callback`:
`tokio::spawn` ran on the Flutter host thread without a current reactor. Entering the
facade-owned runtime before registering the callback fixes the crash and keeps the
pump on the right runtime. A plain Rust host-thread regression test exercises both
registration and callback delivery without relying on an outer Tokio test runtime.
No C signatures changed; generated bindings were not hand-edited.

## Limits

### Android JNI packaging correction (2026-11-06)

The first distributed APK omitted all 28 btleplug/support Java helper classes,
so `RblueNative.nativeInit()` returned false. The Flutter plugin now compiles
the shared `bindings/kotlin/btleplug-java` sources and consumes their JNI keep
rules, along with a keep rule for its Kotlin JNI entry point.

- `tools/check-android-jni.py APK` checks all 29 required class names through
  `apkanalyzer` on PATH. It fails against the previous debug APK and passes
  against the corrected release APK after R8 optimization.
- Release build `1.0.0+2` passes `apksigner verify` and installs on the Android
  API 36 ARM64 emulator. After granting nearby-device permission, the session
  log reports `蓝牙适配器已初始化`; no initialization error is displayed.
- Evidence: `output/distribution/android-fix-session.xml`,
  `android-fix-session.png`, `android-fix-startup.txt`.
- Pgyer CLI polling confirms build 2 is published at
  https://www.pgyer.com/rblueflutterexample . Physical BLE acceptance remains
  outstanding.

### Remaining limits

- No physical phone was available for BLE acceptance. Scan/connect/write/notify,
  reconnect recovery, keyboard/gesture ergonomics and performance still need
  real-device checks. Simulator UI launch is not radio validation.
- The requested broad WASM check remains blocked by the workspace's existing
  Tokio `rt-multi-thread` feature selection (including native C/UniFFI facades).
  This change does not add Web support to the C ABI or alter that feature matrix.
- Android build emits existing Gradle/AGP/Kotlin deprecation warnings and an
  incomplete cached NDK-27 warning; successful native cross-builds used NDK 28.2.
- The iOS plugin remains packaged with CocoaPods; Flutter warns that SPM adoption
  will be needed in a future version. Distribution signing was not changed.
- Independent Android/iOS native example UIs were not redesigned.
