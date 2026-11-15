#!/usr/bin/env bash
#
# Build and stage the rblue C ABI artifacts distributed by rblue_flutter.
#
#   Android: arm64-v8a, armeabi-v7a, x86_64 shared libraries
#   iOS:     arm64 device and Apple Silicon simulator static libraries + XCFramework
#   macOS:   arm64/x86_64 universal static library + XCFramework
#
# With no option, the historical Android+iOS set is built. Use --all to include
# macOS, or one of the platform-specific options below.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
OUT_DIR="$REPO_ROOT/bindings/c"
PLUGIN_ROOT="$REPO_ROOT/examples/flutter/rblue_flutter"
SNAPSHOT_ROOT="$PLUGIN_ROOT/native/source"
CRATE_DIR="$SNAPSHOT_ROOT/crates/rblue-c"
TARGET_DIR="${CARGO_TARGET_DIR:-$SNAPSHOT_ROOT/target}"
PLUGIN_ANDROID_JNILIBS="$PLUGIN_ROOT/android/src/main/jniLibs"
PLUGIN_IOS_LIBS="$PLUGIN_ROOT/ios/Libs"
PLUGIN_IOS_XCFRAMEWORK="$PLUGIN_ROOT/ios/rblue_flutter/RblueC.xcframework"
PLUGIN_MACOS_LIBS="$PLUGIN_ROOT/macos/Libs"
PLUGIN_MACOS_XCFRAMEWORK="$PLUGIN_ROOT/macos/rblue_flutter/RblueC.xcframework"
NATIVE_HEADERS="$PLUGIN_ROOT/native/include"

BUILD_ANDROID=1
BUILD_IOS=1
BUILD_MACOS=0
case "${1:-}" in
  "") ;;
  --android-only) BUILD_IOS=0 ;;
  --ios-only) BUILD_ANDROID=0 ;;
  --macos-only) BUILD_ANDROID=0; BUILD_IOS=0; BUILD_MACOS=1 ;;
  --apple-only) BUILD_ANDROID=0; BUILD_MACOS=1 ;;
  --all) BUILD_MACOS=1 ;;
  *)
    echo "usage: $0 [--android-only|--ios-only|--macos-only|--apple-only|--all]" >&2
    exit 2
    ;;
esac
if [[ $# -gt 1 ]]; then
  echo "usage: $0 [--android-only|--ios-only|--macos-only|--apple-only|--all]" >&2
  exit 2
fi

export CARGO_PROFILE_RELEASE_STRIP="${CARGO_PROFILE_RELEASE_STRIP:-symbols}"
ANDROID_API="${ANDROID_API:-24}"

find_ndk() {
  local roots=()
  [[ -n "${ANDROID_NDK_ROOT:-}" ]] && roots+=("$ANDROID_NDK_ROOT")
  [[ -n "${ANDROID_NDK_LATEST_HOME:-}" ]] && roots+=("$ANDROID_NDK_LATEST_HOME")
  [[ -n "${ANDROID_SDK_ROOT:-}" ]] && roots+=("$ANDROID_SDK_ROOT/ndk")
  [[ -n "${ANDROID_HOME:-}" ]] && roots+=("$ANDROID_HOME/ndk")
  roots+=(
    "/opt/homebrew/share/android-commandlinetools/ndk"
    "$HOME/Library/Android/sdk/ndk"
    "$HOME/Android/Sdk/ndk"
  )
  local root latest
  for root in "${roots[@]}"; do
    [[ -d "$root/toolchains/llvm/prebuilt" ]] && { printf '%s\n' "$root"; return 0; }
    if [[ -d "$root" ]]; then
      latest="$(find "$root" -mindepth 1 -maxdepth 1 -type d -print | sort -V | tail -n 1)"
      [[ -n "$latest" && -d "$latest/toolchains/llvm/prebuilt" ]] && {
        printf '%s\n' "$latest"
        return 0
      }
    fi
  done
  return 1
}

if [[ "$BUILD_ANDROID" == 1 ]]; then
  if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    ANDROID_NDK_HOME="$(find_ndk || true)"
  fi
  if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    echo "error: Android NDK not found; set ANDROID_NDK_HOME" >&2
    exit 1
  fi

  case "$(uname -s)" in
    Darwin) HOST_TAG="darwin-x86_64" ;;
    Linux) HOST_TAG="linux-x86_64" ;;
    *) echo "error: unsupported NDK host $(uname -s)" >&2; exit 1 ;;
  esac
  if [[ ! -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG" ]]; then
    echo "error: NDK toolchain not found at $ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG" >&2
    exit 1
  fi
  NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin"
  echo "== rblue-c Android cross build (api $ANDROID_API, ndk $ANDROID_NDK_HOME) =="
fi

build_android_abi() {
  local triple="$1" abi="$2" linker_triple="$3"
  export CARGO_TARGET_"$(echo "$triple" | tr 'a-z-' 'A-Z_')"_LINKER="$NDK_BIN/$linker_triple-clang"
  cargo build --locked --release --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target "$triple" --no-default-features --features android
  mkdir -p "$OUT_DIR/android/$abi" "$PLUGIN_ANDROID_JNILIBS/$abi"
  cp "$TARGET_DIR/$triple/release/librblue_c.so" "$OUT_DIR/android/$abi/"
  cp "$TARGET_DIR/$triple/release/librblue_c.so" "$PLUGIN_ANDROID_JNILIBS/$abi/"
  echo "   android/$abi <- librblue_c.so"
}

if [[ "$BUILD_ANDROID" == 1 ]]; then
  build_android_abi aarch64-linux-android arm64-v8a "aarch64-linux-android$ANDROID_API"
  build_android_abi armv7-linux-androideabi armeabi-v7a "armv7a-linux-androideabi$ANDROID_API"
  build_android_abi x86_64-linux-android x86_64 "x86_64-linux-android$ANDROID_API"
fi

require_apple_host() {
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: Apple targets require macOS/Xcode" >&2
    exit 1
  fi
  command -v xcodebuild >/dev/null || { echo "error: xcodebuild not found" >&2; exit 1; }
  command -v lipo >/dev/null || { echo "error: lipo not found" >&2; exit 1; }
}

stage_apple_headers() {
  local platform="$1" xcframework="$2"
  cp "$NATIVE_HEADERS/rblue.h" "$PLUGIN_ROOT/$platform/Classes/include/rblue.h"
  while IFS= read -r headers; do
    cat > "$headers/module.modulemap" <<'MODULEMAP'
module RblueC {
  header "rblue.h"
  export *
}
MODULEMAP
  done < <(find "$xcframework" -type d -name Headers -print)
}

build_apple_slice() {
  local triple="$1" feature="$2" output="$3"
  cargo build --locked --release --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target "$triple" --no-default-features --features "$feature"
  mkdir -p "$(dirname "$output")"
  cp "$TARGET_DIR/$triple/release/librblue_c.a" "$output"
  xcrun strip -S -x -o "$output.stripped" "$output"
  mv "$output.stripped" "$output"
}

create_ios_xcframework() {
  rm -rf "$PLUGIN_IOS_XCFRAMEWORK"
  xcodebuild -create-xcframework \
    -library "$PLUGIN_IOS_LIBS/ios/librblue_c.a" -headers "$NATIVE_HEADERS" \
    -library "$PLUGIN_IOS_LIBS/ios-sim/librblue_c.a" -headers "$NATIVE_HEADERS" \
    -output "$PLUGIN_IOS_XCFRAMEWORK" >/dev/null
}

if [[ "$BUILD_IOS" == 1 ]]; then
  require_apple_host
  echo "== rblue-c iOS cross build =="
  build_apple_slice aarch64-apple-ios corebluetooth "$PLUGIN_IOS_LIBS/ios/librblue_c.a"
  build_apple_slice aarch64-apple-ios-sim corebluetooth "$PLUGIN_IOS_LIBS/ios-sim/librblue_c.a"
  mkdir -p "$OUT_DIR/ios" "$OUT_DIR/ios-sim"
  cp "$PLUGIN_IOS_LIBS/ios/librblue_c.a" "$OUT_DIR/ios/"
  cp "$PLUGIN_IOS_LIBS/ios-sim/librblue_c.a" "$OUT_DIR/ios-sim/"
  create_ios_xcframework
  stage_apple_headers ios "$PLUGIN_IOS_XCFRAMEWORK"
  echo "   iOS device/simulator libraries and XCFramework staged"
fi

if [[ "$BUILD_MACOS" == 1 ]]; then
  require_apple_host
  echo "== rblue-c macOS universal build =="
  build_apple_slice aarch64-apple-darwin corebluetooth "$OUT_DIR/macos-arm64/librblue_c.a"
  build_apple_slice x86_64-apple-darwin corebluetooth "$OUT_DIR/macos-x86_64/librblue_c.a"
  mkdir -p "$PLUGIN_MACOS_LIBS/macos" "$OUT_DIR/macos"
  lipo -create \
    "$OUT_DIR/macos-arm64/librblue_c.a" \
    "$OUT_DIR/macos-x86_64/librblue_c.a" \
    -output "$PLUGIN_MACOS_LIBS/macos/librblue_c.a"
  cp "$PLUGIN_MACOS_LIBS/macos/librblue_c.a" "$OUT_DIR/macos/"
  rm -rf "$PLUGIN_MACOS_XCFRAMEWORK"
  xcodebuild -create-xcframework \
    -library "$PLUGIN_MACOS_LIBS/macos/librblue_c.a" -headers "$NATIVE_HEADERS" \
    -output "$PLUGIN_MACOS_XCFRAMEWORK" >/dev/null
  stage_apple_headers macos "$PLUGIN_MACOS_XCFRAMEWORK"
  lipo "$PLUGIN_MACOS_LIBS/macos/librblue_c.a" -verify_arch arm64
  lipo "$PLUGIN_MACOS_LIBS/macos/librblue_c.a" -verify_arch x86_64
  echo "   macOS arm64/x86_64 universal library and XCFramework staged"
fi

echo "== rblue_flutter native artifacts staged successfully =="
