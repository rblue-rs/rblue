# Rebuilding the packaged native libraries

Normal users do **not** run these steps. Pub consumers build against the native
artifacts already included for Android, iOS, macOS, and Windows. This document
records how to reproduce them from the self-contained source snapshot shipped in
the package.

## Source provenance

- Upstream repository: <https://github.com/rblue-rs/rblue>
- Rebuild source commit: `416f04568622e3802aa444e7d2668e8256add7c3`
- Package-local source Git tree: `8489a606289f143c84ded8054f6fe44541b59289`
- All seven binaries were rebuilt from that self-contained snapshot in
  [hosted rebuild run 45157867742](https://github.com/rblue-rs/rblue/actions/runs/45157867742).
  No external working tree or old packaged binary was used as a build input.
- Pinned btleplug fork revision:
  `1af8299c291f24fca68dfb6b7ef1b5c0475892e1`
- Rust compiler: `rustc 1.100.0 (0ef1bc386 2026-11-09)`
- Cargo: `cargo 1.100.0 (3acc79c0c 2026-11-05)`
- Android NDK: `30.0.16248370` (r30); link API: 24
- Apple Xcode: 26.6 (build 17F113)
- Windows: native `x86_64-pc-windows-msvc`, using the MSVC tools from
  Visual Studio Enterprise 2026, installation `18.12.12405.243` (18.12.0).
  The hosted record captures the Visual Studio installation, not a separate
  `cl.exe` patch version; this rebuild did not use `cargo-xwin`.

`provenance/android.json`, `provenance/apple.json`, and
`provenance/windows.json` retain the source commit/tree, all 58 source-file
SHA-256 values, build-input identities, actual host/toolchain records, run URL,
and seven output checksums. They were recorded by the rebuild jobs themselves.
Android and Apple checkouts use canonical LF bytes. The Windows checkout used
Git's CRLF conversion: its 58 recorded source hashes match the exact bytes
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
d88ad3d37ff9b992423b025ac038b7a61285dfcc7fada9298271d7a0deddcb30  android/src/main/jniLibs/arm64-v8a/librblue_c.so
98a33b234c9c5293286791c79d464c89f3add5b2d7e9f0e7cd98f17b3a6b31a4  android/src/main/jniLibs/armeabi-v7a/librblue_c.so
9efe58cecaaf29d156aa6bca81c985455667a8f11d828f36fdad2be0d7f271dd  android/src/main/jniLibs/x86_64/librblue_c.so
d707b26d98da5edc616e8905366416887c9544a02748088618ef8d51896c7906  ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a
703eb428ad9ac01fdda32b62cf0e3eb280204367393b5a6899556a5f1dd9cbd3  ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a
69223037e54ca197f7985fb489b860d01e36e022e8ed1d54e62ef0e728b7fc76  macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a
5613cef2cec392ff8ca37b12e6d6f74687a8b448b17e78fc901fbb4398eae053  windows/x64/rblue_c.dll
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
