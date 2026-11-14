## 0.0.1-dev.2

Expands the experimental preview and fixes Pub.dev package scoring.

- Adds complete MIT and Apache-2.0 license texts in the package `LICENSE`.
- Adds iOS Swift Package Manager support while retaining CocoaPods.
- Adds universal macOS 12+ support through CoreBluetooth for arm64 and x86_64.
- Adds Windows 10 1809+ x64 support through the WinRT backend.
- Migrates the Android plugin away from legacy Kotlin declarations while
  retaining Flutter 3.41/AGP 8 compatibility.
- Adds platform-specific packaging checks, reproducible native artifacts,
  examples, and CI coverage.
- Adds an explicit native shutdown/drain barrier so Dart releases every queued
  callback payload and event string before closing FFI listeners.

This remains an experimental release. Native packaging is build-verified, but
BLE behavior must still be accepted on physical hardware for each platform.

## 0.0.1-dev.1

Initial early experimental Pub.dev preview.

- Exposes the rblue message-passing C ABI as a typed Dart client.
- Supports Android API 24+ on arm64-v8a, armeabi-v7a, and x86_64.
- Supports arm64 iOS 15+ devices and Apple Silicon iOS simulators.
- Packages rebuilt native Rust libraries, required Android JNI Java helpers,
  reproducible source snapshots, checksums, and third-party notices so the
  package has no monorepo-relative runtime or build dependencies.
- Includes an Android/iOS example for scan, GATT read/write/notifications, and
  host-managed reconnect behavior.

This is not a stable or production-readiness release. The Dart API, native ABI,
and platform coverage may change, and BLE behavior still requires acceptance
on physical devices.
