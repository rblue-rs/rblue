# Rebuilding the packaged native libraries

Normal users do **not** run these steps. Pub consumers build against the native
artifacts already included for Android, iOS, macOS, and Windows. This document
records how to reproduce them from the self-contained source snapshot shipped in
the package.

## Source provenance

- Upstream repository: <https://github.com/rblue-rs/rblue>
- Rebuild source commit: `5c8583c04e348ea60398e45452bdc166b74d5851`
- Package-local source Git tree: `45243a7f9a32bd8c6e988606d96e37fb5ee5d4a3`
- All seven binaries were rebuilt from that self-contained snapshot in
  [hosted rebuild run 45199804552](https://github.com/rblue-rs/rblue/actions/runs/45199804552).
  No external working tree or old packaged binary was used as a build input.
- Pinned btleplug fork revision:
  `983f3f1346bb1646281e0faf345a21c4a787833d`
- Rust compiler: `rustc 1.100.0 (0ef1bc386 2026-11-09)`
- Cargo: `cargo 1.100.0 (3acc79c0c 2026-11-05)`
- Android NDK: `30.0.16248370` (r30); link API: 24
- Apple Xcode: 26.6 (build 17F113)
- Windows: native `x86_64-pc-windows-msvc`, using the MSVC tools from
  Visual Studio Enterprise 2026, installation `18.12.12405.243` (18.12.0).
  The hosted record captures the Visual Studio installation, not a separate
  `cl.exe` patch version; this rebuild did not use `cargo-xwin`.

`provenance/android.json`, `provenance/apple.json`, and
`provenance/windows.json` retain the source commit/tree, all 61 source-file
SHA-256 values, build-input identities, actual host/toolchain records, run URL,
and seven output checksums. They were recorded by the rebuild jobs themselves.
Android and Apple checkouts use canonical LF bytes. The Windows checkout used
Git's CRLF conversion: its 61 recorded source hashes match the exact bytes
produced by `git -c core.autocrlf=true cat-file --filters HEAD:<path>` for the
same committed source tree. The validator checks those Git clean/smudge rules
and each current source blob identity; it does not ignore arbitrary source edits
or relax binary checksums.

The optional `rebuild_native` workflow-dispatch input on the CI repair branch
runs only standard hosted Ubuntu/macOS/Windows runners. It records allowlisted
native binaries and provenance with non-force Git commits, without Actions
artifact/cache storage, releases, or package publishing. Normal CI is read-only.
After a rebuild, update this checksum table and run both validators:

```bash
python3 examples/flutter/tools/verify-native-artifacts.py
bash examples/flutter/tools/check-snapshot-sync.sh
```

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
`cargo xwin build`. The packaged DLL was built natively from this snapshot on the hosted MSVC runner.

## Packaged checksums

SHA-256:

```text
5896f98c5957459a07558ce3eda1b6ad48f383434ed9ffe21c1a4f9763255cef  android/src/main/jniLibs/arm64-v8a/librblue_c.so
2c21d43bdc40c808fe9797224b95176ba72a7dddaf9930d6951aed6345919954  android/src/main/jniLibs/armeabi-v7a/librblue_c.so
0393ac3a8fd373b7c4c9dc88dac9ed5fb0cb598cc199b6145195f62cca9331bd  android/src/main/jniLibs/x86_64/librblue_c.so
5ebd3ecd94bc55460ad7c8f3a84e4de35d4a0029e11fe16b72cab4fea3c3f2f3  ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a
bbe12d3efd04bcd643e8a1b2fbfa9e2e1555ea0529ab23f5cf715741cf29b694  ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a
7ff13512634f136feac6f05c8d96cc5e8c7064a6bcebc1e87568b152b27e9445  macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a
481676e7f7b8854d8b0552937fadfd3c9ac1320f1de8ae87340a54dd06bbc657  windows/x64/rblue_c.dll
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
