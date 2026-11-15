#!/usr/bin/env bash
#
# Builds the rblue Android bindings: the four ABI .so files consumed by the
# AAR module (bindings/kotlin/rblue) and the UniFFI Kotlin bindings.
#
# Artifacts:
#   bindings/kotlin/rblue/src/real/jniLibs/<abi>/librblue_ffi.so
#       Release cdylib of `rblue-ffi` built with the `android` feature, i.e.
#       the real JNI backend (rblue-android shell over the btleplug fork).
#       arm64-v8a, armeabi-v7a, x86, x86_64.
#   bindings/kotlin/rblue/src/mock/jniLibs/<abi>/librblue_ffi.so
#       Same cdylib built with the default feature set (mock backend) for
#       arm64-v8a and x86_64: runs on emulators and CI with no Bluetooth
#       hardware. The `mock` product flavor of `:rblue` picks these up.
#   bindings/kotlin/generated/
#       Canonical `uniffi-bindgen --language kotlin` output (uniffi/rblue_ffi/
#       rblue_ffi.kt), committed for review; CI rejects drift against it.
#
# Notes:
#   - The crate built is `rblue-ffi`, not the aggregate `rblue`: the UniFFI
#     scaffolding lives in rblue-ffi, and the feature is forwarded to the
#     aggregate crate, which selects the Android backend on android targets.
#   - `cargo` needs the NDK clang wrappers as linkers; this script exports the
#     CARGO_TARGET_*_LINKER variables itself (cargo-ndk is not required).
#   - Generated Kotlin uses `--no-format` so output is deterministic and the
#     committed copy is drift-checkable without ktlint.
#
# Usage: scripts/build-bindings.sh [--real|--mock]
#   --real  build only the real-backend .so pipeline
#   --mock  build only the mock-backend .so pipeline (plus the Kotlin bindings)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
GENERATED="$ROOT/bindings/kotlin/generated"
RBLUE_MODULE="$ROOT/bindings/kotlin/rblue"

MODE="all"
case "${1:-}" in
  "") ;;
  --real) MODE="real" ;;
  --mock) MODE="mock" ;;
  *) echo "usage: $0 [--real|--mock]" >&2; exit 2 ;;
esac

for tool in cargo rustup python3; do
  command -v "$tool" >/dev/null 2>&1 || { echo "error: $tool is required (Rust toolchain)" >&2; exit 1; }
done

# --- Android SDK / NDK -------------------------------------------------------

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [[ -z "$SDK" ]]; then
  for candidate in /opt/homebrew/share/android-commandlinetools "$HOME/Library/Android/sdk" "$HOME/Android/Sdk"; do
    if [[ -d "$candidate" ]]; then SDK="$candidate"; break; fi
  done
fi
[[ -n "$SDK" && -d "$SDK" ]] || { echo "error: Android SDK not found; set ANDROID_HOME" >&2; exit 1; }

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "$NDK" || ! -d "$NDK" ]]; then
  # Pick the highest complete NDK installed under the SDK.
  NDK="$(find "$SDK/ndk" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | while read -r dir; do
    # `if` (not `[[ ]] &&`): a failing short-circuit body would propagate a
    # nonzero status through pipefail and kill this script under set -e.
    if [[ -d "$dir/toolchains/llvm/prebuilt" ]]; then echo "$dir"; fi
  done | sort -V | tail -1)"
fi
[[ -n "$NDK" && -d "$NDK" ]] || { echo "error: Android NDK not found; set ANDROID_NDK_HOME" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) HOST_TAG="darwin-x86_64" ;;   # NDK binaries are universal; runs on arm64 macs
  Linux)  HOST_TAG="linux-x86_64" ;;
  *) echo "error: unsupported host OS for the NDK: $(uname -s)" >&2; exit 1 ;;
esac
TOOLCHAIN_BIN="$NDK/toolchains/llvm/prebuilt/$HOST_TAG/bin"
NM="$TOOLCHAIN_BIN/llvm-nm"
[[ -x "$TOOLCHAIN_BIN/clang" ]] || { echo "error: NDK toolchain incomplete: $TOOLCHAIN_BIN" >&2; exit 1; }

# --- Targets -----------------------------------------------------------------
# minSdk 24: the AAR's declared floor (crates/rblue-android/docs/platform-notes.md).
MIN_API="24"
abi_target() {
  case "$1" in
    arm64-v8a)   echo "aarch64-linux-android" ;;
    armeabi-v7a) echo "armv7-linux-androideabi" ;;
    x86)         echo "i686-linux-android" ;;
    x86_64)      echo "x86_64-linux-android" ;;
  esac
}
abi_linker() {
  case "$1" in
    arm64-v8a)   echo "aarch64-linux-android${MIN_API}-clang" ;;
    armeabi-v7a) echo "armv7a-linux-androideabi${MIN_API}-clang" ;;
    x86)         echo "i686-linux-android${MIN_API}-clang" ;;
    x86_64)      echo "x86_64-linux-android${MIN_API}-clang" ;;
  esac
}
REAL_ABIS=(arm64-v8a armeabi-v7a x86 x86_64)
MOCK_ABIS=(arm64-v8a x86_64)

echo "==> rustup target add (android)"
TARGETS=""
for abi in "${REAL_ABIS[@]}"; do TARGETS+=" $(abi_target "$abi")"; done
rustup target add $TARGETS

build_abi() {
  local abi="$1" flavor="$2"
  local target; target="$(abi_target "$abi")"
  local feature_args=()
  if [[ "$flavor" == "real" ]]; then
    feature_args=(--features android)
  fi
  echo "==> cargo build --release -p rblue-ffi --lib --target $target${feature_args[*]:+ ${feature_args[*]}} ($flavor backend)"
  local linker_var="CARGO_TARGET_$(echo "$target" | tr '[:lower:]-' '[:upper:]_')_LINKER"
  (cd "$ROOT" && \
    env "$linker_var=$TOOLCHAIN_BIN/$(abi_linker "$abi")" \
    cargo build --locked --release -p rblue-ffi --lib ${feature_args[@]+"${feature_args[@]}"} --target "$target")
  local so="$ROOT/target/$target/release/librblue_ffi.so"
  [[ -f "$so" ]] || { echo "error: $so was not produced" >&2; exit 1; }
  local dest="$RBLUE_MODULE/src/$flavor/jniLibs/$abi"
  mkdir -p "$dest"
  # Replace any previous file (including a symlink) so a later mock rebuild
  # of the same target cannot clobber the real-backend copy in place.
  rm -f "$dest/librblue_ffi.so"
  cp "$so" "$dest/librblue_ffi.so"
  echo "    installed $(basename "$so") -> ${dest#"$ROOT"/}"
  if [[ "$flavor" == "real" && -x "$NM" ]]; then
    if ! check_jni_export "$dest/librblue_ffi.so"; then
      exit 1
    fi
  fi
}

# Fail if the JNI init entry is missing from a real-backend .so, and dump
# both the dynamic and the full symbol table so a false-negative `nm -D`
# (seen on some aarch64 ELFs) is visible in CI logs.
check_jni_export() {
  local so="$1"
  local symbol="Java_com_rblue_RblueNative_nativeInit"
  if [[ ! -f "$so" ]]; then
    echo "error: expected $so after the real backend build" >&2
    return 1
  fi
  if "$NM" -D --defined-only "$so" | grep -q "$symbol"; then
    echo "    $symbol exported by ${so#"$ROOT"/}"
    return 0
  fi
  echo "error: $symbol is not exported by $so" >&2
  echo "       (the AAR would fail at RblueAndroid.initialize; see crates/rblue-android/src/jni_shim.rs)" >&2
  echo "       llvm-nm -D --defined-only:" >&2
  "$NM" -D --defined-only "$so" >&2 || true
  echo "       llvm-nm --defined-only (grep $symbol / T):" >&2
  "$NM" --defined-only "$so" | grep -E "$symbol|[[:space:]]T[[:space:]]" | head -n 80 >&2 || true
  local readelf="${NM%llvm-nm}llvm-readelf"
  if [[ -x "$readelf" ]]; then
    echo "       llvm-readelf --dyn-syms (grep $symbol):" >&2
    "$readelf" --dyn-syms "$so" | grep -E "$symbol|File:|Symbol table" >&2 || true
  fi
  return 1
}

if [[ "$MODE" == "all" || "$MODE" == "real" ]]; then
  for abi in "${REAL_ABIS[@]}"; do build_abi "$abi" real; done
fi
if [[ "$MODE" == "all" || "$MODE" == "mock" ]]; then
  for abi in "${MOCK_ABIS[@]}"; do build_abi "$abi" mock; done
fi

# --- Sanity check: the JNI init entry must be in every real .so -------------
# Individual real ABIs are already checked at install time; this pass
# re-checks every copy after the mock builds so an in-place clobber
# cannot slip through.
if [[ "$MODE" == "all" || "$MODE" == "real" ]] && [[ -x "$NM" ]]; then
  missing=0
  for abi in "${REAL_ABIS[@]}"; do
    if ! check_jni_export "$RBLUE_MODULE/src/real/jniLibs/$abi/librblue_ffi.so"; then
      missing=1
    fi
  done
  if [[ "$missing" -ne 0 ]]; then
    exit 1
  fi
  echo "==> JNI init symbol present in every real librblue_ffi.so"
fi

# --- UniFFI Kotlin bindings ---------------------------------------------------
# Generate from an already-built android artifact so the committed bindings
# always match the shipped .so surface; real arm64 is preferred, mock arm64 is
# the fallback (same API surface either way).
BINDGEN_SO=""
for candidate in "$ROOT/target/aarch64-linux-android/release/librblue_ffi.so" \
                 "$RBLUE_MODULE/src/mock/jniLibs/arm64-v8a/librblue_ffi.so"; do
  [[ -f "$candidate" ]] && { BINDGEN_SO="$candidate"; break; }
done
if [[ -z "$BINDGEN_SO" ]]; then
  echo "error: no android .so available for binding generation" >&2
  exit 1
fi

echo "==> uniffi-bindgen generate --language kotlin --out-dir bindings/kotlin/generated"
rm -rf "$GENERATED"
(cd "$ROOT" && cargo run --locked -q -p rblue-ffi --bin uniffi-bindgen -- \
  generate --library "$BINDGEN_SO" \
  --language kotlin --no-format \
  --out-dir "$GENERATED")
GENERATED_KT="$GENERATED/uniffi/rblue_ffi/rblue_ffi.kt"
[[ -f "$GENERATED_KT" ]] || { echo "error: uniffi-bindgen did not produce $GENERATED_KT" >&2; exit 1; }
# UniFFI's `--no-format` template contains trailing spaces. Strip them
# deterministically so the reviewed generated source passes `git diff --check`
# without requiring ktfmt in the release pipeline.
python3 - "$GENERATED_KT" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text().splitlines()
while lines and not lines[-1].strip():
    lines.pop()
path.write_text("\n".join(line.rstrip() for line in lines) + "\n")
PY

# Vendored upstream notices occasionally contain whitespace-only lines; keep
# their text intact while making the repository diff-clean.
python3 - "$ROOT/bindings/kotlin/btleplug-java/NOTICE.md" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
lines = path.read_text().splitlines()
while lines and not lines[-1].strip():
    lines.pop()
path.write_text("\n".join(line.rstrip() for line in lines) + "\n")
PY


echo "==> done"
echo "    jniLibs:   $RBLUE_MODULE/src/{real,mock}/jniLibs"
echo "    generated: $GENERATED_KT"
echo "next: from examples/android run ./gradlew :app:assembleRealDebug (device) or :app:assembleMockDebug (emulator)."
