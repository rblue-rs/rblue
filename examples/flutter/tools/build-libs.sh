#!/usr/bin/env bash
#
# build-libs.sh - cross-builds the rblue C ABI (crates/rblue-c) for the
# Flutter example and stages the artifacts for both plugin packagers:
#
#   Android: 4 ABI cdylibs  -> bindings/c/android/<abi>/librblue_c.so
#            (canonical copy; staged into
#             rblue_flutter/android/src/main/jniLibs/<abi>/ for gradle)
#   iOS:     2 static slices -> bindings/c/ios{,-sim}/librblue_c.a
#            (canonical copy; staged into rblue_flutter/ios/Libs/, which the
#             podspec force_loads per SDK)
#
# Everything under bindings/c/{android,ios} and rblue_flutter/ios/Libs is a
# build artifact and gitignored; rerun this script to refresh it.
#
# Environment overrides:
#   ANDROID_NDK_HOME        NDK root (default: Homebrew commandline tools 27)
#   ANDROID_API             linker API level (default 24, the rblue-android floor)
#   SKIP_ANDROID / SKIP_IOS set to 1 to build only one platform
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
CRATE_DIR="$REPO_ROOT/crates/rblue-c"
OUT_DIR="$REPO_ROOT/bindings/c"
PLUGIN_IOS_LIBS="$REPO_ROOT/examples/flutter/rblue_flutter/ios/Libs"
PLUGIN_ANDROID_JNILIBS="$REPO_ROOT/examples/flutter/rblue_flutter/android/src/main/jniLibs"

ANDROID_API="${ANDROID_API:-24}"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  NDK_BASE="/opt/homebrew/share/android-commandlinetools/ndk"
  if [[ -d "$NDK_BASE/28.2.13676358" ]]; then
    ANDROID_NDK_HOME="$NDK_BASE/28.2.13676358"
  else
    ANDROID_NDK_HOME="$NDK_BASE/27.0.12077973"
  fi
fi

HOST_TAG="darwin-x86_64"
if [[ ! -d "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG" ]]; then
  echo "error: NDK toolchain not found at" \
    "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG" >&2
  echo "       set ANDROID_NDK_HOME to your NDK root" >&2
  exit 1
fi
NDK_BIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$HOST_TAG/bin"

echo "== rblue-c cross build (api $ANDROID_API, ndk $ANDROID_NDK_HOME) =="

build_android_abi() {
  local triple="$1" abi="$2" linker_triple="$3"
  export CARGO_TARGET_"$(echo "$triple" | tr 'a-z-' 'A-Z_')"_LINKER="$NDK_BIN/$linker_triple-clang"
  cargo build --release --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target "$triple" --no-default-features --features android
  mkdir -p "$OUT_DIR/android/$abi" "$PLUGIN_ANDROID_JNILIBS/$abi"
  cp "$CRATE_DIR/target/$triple/release/librblue_c.so" "$OUT_DIR/android/$abi/"
  cp "$CRATE_DIR/target/$triple/release/librblue_c.so" "$PLUGIN_ANDROID_JNILIBS/$abi/"
  echo "   android/$abi <- librblue_c.so"
}

if [[ "${SKIP_ANDROID:-0}" != "1" ]]; then
  # rblue-android needs API 24+; the linker triples embed it.
  build_android_abi aarch64-linux-android   arm64-v8a   "aarch64-linux-android$ANDROID_API"
  build_android_abi armv7-linux-androideabi armeabi-v7a "armv7a-linux-androideabi$ANDROID_API"
  build_android_abi i686-linux-android      x86         "i686-linux-android$ANDROID_API"
  build_android_abi x86_64-linux-android    x86_64      "x86_64-linux-android$ANDROID_API"
fi

build_ios_slice() {
  local triple="$1" dir="$2"
  cargo build --release --manifest-path "$CRATE_DIR/Cargo.toml" \
    --target "$triple" --no-default-features --features corebluetooth
  mkdir -p "$OUT_DIR/$dir" "$PLUGIN_IOS_LIBS/$dir"
  cp "$CRATE_DIR/target/$triple/release/librblue_c.a" "$OUT_DIR/$dir/"
  cp "$CRATE_DIR/target/$triple/release/librblue_c.a" "$PLUGIN_IOS_LIBS/$dir/"
  echo "   $dir <- librblue_c.a"
}

if [[ "${SKIP_IOS:-0}" != "1" ]]; then
  build_ios_slice aarch64-apple-ios ios
  build_ios_slice aarch64-apple-ios-sim ios-sim
fi

echo "== done: artifacts in $OUT_DIR; staged into $PLUGIN_ANDROID_JNILIBS and $PLUGIN_IOS_LIBS =="
