# Rebuilding the packaged native libraries

Normal users do **not** run these steps. Pub consumers build against the native
artifacts already included for Android, iOS, macOS, and Windows. This document
records how to reproduce them from the self-contained source snapshot shipped in
the package.

## Source provenance

- Upstream repository: <https://github.com/rblue-rs/rblue>
- Rebuild source commit: `4d8b356ae47cf5bc2d6f6c6a6f9bca12bbeed80a`
- Package-local source Git tree: `9d46425e979fea71554df771b7f21263224e7469`
- All seven binaries were rebuilt from that self-contained snapshot in
  [hosted rebuild run 45165332315](https://github.com/rblue-rs/rblue/actions/runs/45165332315).
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
`provenance/windows.json` retain the source commit/tree, all 59 source-file
SHA-256 values, build-input identities, actual host/toolchain records, run URL,
and seven output checksums. They were recorded by the rebuild jobs themselves.
Android and Apple checkouts use canonical LF bytes. The Windows checkout used
Git's CRLF conversion: its 59 recorded source hashes match the exact bytes
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
01a06021c07398d116073dd02ca659d8a75ced82d43f4d275ce992ee664e13e9  android/src/main/jniLibs/arm64-v8a/librblue_c.so
f938c79a3707556c9ef1d43352b7e86883cad874e549102501c93830f8c785f9  android/src/main/jniLibs/armeabi-v7a/librblue_c.so
96ef6bd9d8b07c27033450a6ca4a136bc5e409cdc72975d11916d4f4d67008fd  android/src/main/jniLibs/x86_64/librblue_c.so
96381f2d7049775100348a1adc2e60a4b418c7feaeee747924eea82aecf84f9c  ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a
339e4e4a6bb132c692a696180cd1e7dc952180ee6708146e9df5ea99556c01c9  ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a
5711f97f3d949b47770c681baa40d806b8b597a9160a50294dd8e57503c3c639  macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a
740a65ea2551b539ddd5f7296e8899449e9de3da6302316caf1a401945ab8e2a  windows/x64/rblue_c.dll
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
