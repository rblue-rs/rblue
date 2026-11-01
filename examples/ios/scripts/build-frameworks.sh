#!/usr/bin/env bash
#
# Builds the rblue XCFramework and the Swift bindings for the local SPM
# package (bindings/swift/Rblue).
#
# Artifacts:
#   target/<triple>/release/librblue_ffi.a
#       Static libraries for the device and simulator slices. One artifact per
#       slice is enough although the plan sketches `librblue_ffi.a +
#       librblue.a`: rustc bundles every rlib dependency (the aggregate crate,
#       the CoreBluetooth backend, tokio, ...) into the top-level staticlib,
#       so `librblue_ffi.a` already carries the whole stack. The UniFFI
#       scaffolding lives in rblue-ffi, which is why rblue-ffi (not the
#       aggregate `rblue`) is the crate built here; the `corebluetooth`
#       feature is forwarded to the aggregate crate, which selects the
#       CoreBluetooth backend on Apple targets.
#   bindings/swift/generated/
#       Canonical uniffi-bindgen output, committed for review:
#       rblue_ffi.swift / rblue_ffiFFI.h / rblue_ffiFFI.modulemap.
#   bindings/swift/Rblue/Sources/Rblue/Rblue.swift
#       Script-synced copy of the generated Swift module. Committed so the
#       SPM package resolves on a fresh clone; refreshed on every run.
#   bindings/swift/Rblue/Rblue.xcframework
#       Static XCFramework (device + simulator slices, each with the FFI
#       header and a module.modulemap). Gitignored; rebuilt by this script.
#
# The script is idempotent: previous xcframework/headers staging is removed
# before each run.
#
# Usage: scripts/build-frameworks.sh [--mock]
#   --mock  Build the mock backend instead of CoreBluetooth. Builds without
#           any Bluetooth hardware usage; useful to smoke-test the packaging
#           pipeline, but the app cannot talk to real devices.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
IOS_TRIPLE="aarch64-apple-ios"
SIM_TRIPLE="aarch64-apple-ios-sim"
FEATURE="corebluetooth"
if [[ "${1:-}" == "--mock" ]]; then
  FEATURE="mock"
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--mock]" >&2
  exit 2
fi

GENERATED="$ROOT/bindings/swift/generated"
PACKAGE="$ROOT/bindings/swift/Rblue"
XCFRAMEWORK="$PACKAGE/Rblue.xcframework"
STAGING="$ROOT/examples/ios/build/xcframework-headers"

for tool in cargo rustup xcodebuild; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: $tool is required (Xcode and Rust toolchains)" >&2
    exit 1
  fi
done

echo "==> rustup target add $IOS_TRIPLE $SIM_TRIPLE"
rustup target add "$IOS_TRIPLE" "$SIM_TRIPLE"

staticlib() {
  echo "$ROOT/target/$1/release/librblue_ffi.a"
}

build_slice() {
  local triple="$1"
  echo "==> cargo build --release -p rblue-ffi --lib --features $FEATURE --target $triple"
  (cd "$ROOT" && cargo build --release -p rblue-ffi --lib --features "$FEATURE" --target "$triple")
  if [[ ! -f "$(staticlib "$triple")" ]]; then
    echo "error: static library was not produced at $(staticlib "$triple")" >&2
    exit 1
  fi
}

build_slice "$IOS_TRIPLE"
build_slice "$SIM_TRIPLE"

echo "==> uniffi-bindgen generate --language swift --out-dir bindings/swift/generated"
rm -rf "$GENERATED"
(cd "$ROOT" && cargo run -q -p rblue-ffi --bin uniffi-bindgen -- \
  generate --library "$(staticlib "$IOS_TRIPLE")" \
  --language swift \
  --out-dir "$GENERATED")
for file in rblue_ffi.swift rblue_ffiFFI.h rblue_ffiFFI.modulemap; do
  if [[ ! -f "$GENERATED/$file" ]]; then
    echo "error: uniffi-bindgen did not produce $file" >&2
    exit 1
  fi
done

echo "==> sync generated Swift module into the SPM package"
mkdir -p "$PACKAGE/Sources/Rblue"
cp "$GENERATED/rblue_ffi.swift" "$PACKAGE/Sources/Rblue/Rblue.swift"

stage_headers() {
  local triple="$1" dir="$2"
  rm -rf "$dir"
  mkdir -p "$dir"
  cp "$GENERATED/rblue_ffiFFI.h" "$dir/"
  # SPM's clang importer scans include paths for a file named exactly
  # `module.modulemap`; the module *name* inside stays `rblue_ffiFFI`, which
  # is what the generated Swift code imports.
  cp "$GENERATED/rblue_ffiFFI.modulemap" "$dir/module.modulemap"
  echo "    staged headers for $triple in $dir"
}

stage_headers "$IOS_TRIPLE" "$STAGING/$IOS_TRIPLE"
stage_headers "$SIM_TRIPLE" "$STAGING/$SIM_TRIPLE"

echo "==> xcodebuild -create-xcframework"
rm -rf "$XCFRAMEWORK"
xcodebuild -create-xcframework \
  -library "$(staticlib "$IOS_TRIPLE")" -headers "$STAGING/$IOS_TRIPLE" \
  -library "$(staticlib "$SIM_TRIPLE")" -headers "$STAGING/$SIM_TRIPLE" \
  -output "$XCFRAMEWORK" \
  >/dev/null

echo "==> done"
echo "    xcframework: $XCFRAMEWORK"
echo "    generated:   $GENERATED"
echo "next: xcodegen generate (in examples/ios), then build the RblueExample scheme."
