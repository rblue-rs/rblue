# Rebuilding the packaged native libraries

Normal users do **not** run these steps. Pub consumers build against the native
artifacts already included for Android, iOS, macOS, and Windows. This document
records how to reproduce them from the self-contained source snapshot shipped in
the package.

## Source provenance

- Upstream repository: <https://github.com/rblue-rs/rblue>
- Upstream base revision: `70d89cbbc3a6fb552035bb492da64fd7114cb0ea`
- The `0.0.1-dev.2` source snapshot additionally contains the callback
  shutdown/drain ABI used by this package. The complete package-local snapshot,
  rather than an external working tree, is the authoritative rebuild input.
- Pinned btleplug fork revision:
  `1af8299c291f24fca68dfb6b7ef1b5c0475892e1`
- Rust compiler used: `rustc 1.96.0 (ac68faa20 2026-05-25)`
- Cargo used: `cargo 1.96.0 (30a34c682 2026-05-25)`
- Android NDK used: `28.2.13676358`; link API: 24
- Xcode used for Apple targets: 27.1 (build 27A9275)
- Windows artifact built for `x86_64-pc-windows-msvc` with `cargo-xwin 0.20.2`

`source/` is a release-focused workspace containing the exact implementation
crates needed by the four declared Flutter platforms plus their frozen lockfile.
Tests and backends not packaged by this Flutter plugin are intentionally
omitted; selected WinRT backend notes are retained with the Windows source for
consumer requirements and diagnostics. Its aggregate Cargo manifests therefore expose
Android, CoreBluetooth, WinRT, and mock features only. It has no paths outside
the package.

## Prerequisites

Install Rust and the relevant targets:

```bash
rustup target add \
  aarch64-linux-android armv7-linux-androideabi x86_64-linux-android \
  aarch64-apple-ios aarch64-apple-ios-sim \
  aarch64-apple-darwin x86_64-apple-darwin \
  x86_64-pc-windows-msvc
```

Android needs an NDK. Apple artifacts require macOS/Xcode. Windows can be built
on an x64 MSVC host with Cargo or cross-built using `cargo-xwin`.

## Android

From this `native/` directory:

```bash
export ANDROID_NDK_HOME=/path/to/android-ndk
export NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64/bin"
export CARGO_PROFILE_RELEASE_STRIP=symbols

build_android() {
  triple="$1"; abi="$2"; clang="$3"
  linker_var="CARGO_TARGET_$(printf '%s' "$triple" | tr 'a-z-' 'A-Z_')_LINKER"
  env "$linker_var=$NDK_BIN/$clang" \
    cargo build --locked --release \
      --manifest-path source/crates/rblue-c/Cargo.toml \
      --target "$triple" --no-default-features --features android
  mkdir -p "../android/src/main/jniLibs/$abi"
  cp "source/target/$triple/release/librblue_c.so" \
    "../android/src/main/jniLibs/$abi/"
}

build_android aarch64-linux-android arm64-v8a aarch64-linux-android24-clang
build_android armv7-linux-androideabi armeabi-v7a armv7a-linux-androideabi24-clang
build_android x86_64-linux-android x86_64 x86_64-linux-android24-clang
```

The Android Java helpers under `../android/src/main/java` come from the same
pinned btleplug fork. See `../android/BLEPLUG_JAVA_NOTICE.md`.

## iOS

```bash
export CARGO_PROFILE_RELEASE_STRIP=symbols

for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo build --locked --release \
    --manifest-path source/crates/rblue-c/Cargo.toml \
    --target "$triple" --no-default-features --features corebluetooth
done

mkdir -p ../ios/Libs/ios ../ios/Libs/ios-sim
cp source/target/aarch64-apple-ios/release/librblue_c.a \
  ../ios/Libs/ios/librblue_c.a
cp source/target/aarch64-apple-ios-sim/release/librblue_c.a \
  ../ios/Libs/ios-sim/librblue_c.a

rm -rf ../ios/rblue_flutter/RblueC.xcframework
xcodebuild -create-xcframework \
  -library ../ios/Libs/ios/librblue_c.a -headers include \
  -library ../ios/Libs/ios-sim/librblue_c.a -headers include \
  -output ../ios/rblue_flutter/RblueC.xcframework
```

Add `module.modulemap` beside each XCFramework header as done by
`examples/flutter/tools/build-libs.sh`.

## macOS

```bash
export CARGO_PROFILE_RELEASE_STRIP=symbols

for triple in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --locked --release \
    --manifest-path source/crates/rblue-c/Cargo.toml \
    --target "$triple" --no-default-features --features corebluetooth
done

mkdir -p ../macos/Libs/macos
lipo -create \
  source/target/aarch64-apple-darwin/release/librblue_c.a \
  source/target/x86_64-apple-darwin/release/librblue_c.a \
  -output ../macos/Libs/macos/librblue_c.a

rm -rf ../macos/rblue_flutter/RblueC.xcframework
xcodebuild -create-xcframework \
  -library ../macos/Libs/macos/librblue_c.a -headers include \
  -output ../macos/rblue_flutter/RblueC.xcframework
```

## Windows

On Windows x64 with the MSVC build tools:

```powershell
cargo build --locked --release `
  --manifest-path source/crates/rblue-c/Cargo.toml `
  --target x86_64-pc-windows-msvc `
  --no-default-features --features winrt
Copy-Item source/target/x86_64-pc-windows-msvc/release/rblue_c.dll `
  ../windows/x64/rblue_c.dll
```

On macOS/Linux, install `cargo-xwin` and replace `cargo build` with
`cargo xwin build`. The packaged DLL was produced directly from this snapshot.

## Published checksums

SHA-256:

```text
cd0e4d09eb3a6f9b923da59f160989339b8c1dec6bcdfc86fe182e205d0a02c2  android/src/main/jniLibs/arm64-v8a/librblue_c.so
2afe0912f93c44bab44d110575b0f1316c32e527829fdacef9f76138b3991d65  android/src/main/jniLibs/armeabi-v7a/librblue_c.so
58ad549b75d8b72adcb7f1bb0e19114118e305059ab23d7041fd54c97a705aa0  android/src/main/jniLibs/x86_64/librblue_c.so
0b9a197027903a2d007dc04d7ec204350af4e0fb8daada61557fd11e8043f71d  ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a
c95aef6f0ce0978e56c0e963dccc88b3c17b6eca09f19bf31c40473d167c4a47  ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a
09910968887d3f342dcddc79e286a0fdcb31c2739dc8664945a3b61a6624a033  macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a
906b8765a302472a98764c200d63413b72a3b51e2ce2eaad35f743a9abde7421  windows/x64/rblue_c.dll
```

Every artifact must expose these nine public ABI symbols (Apple prepends `_`):

```text
rblue_new
rblue_shutdown
rblue_free
rblue_request
rblue_set_event_callback
rblue_set_owned_response_callback
rblue_set_response_callback
rblue_free_string
rblue_free_payload
```
