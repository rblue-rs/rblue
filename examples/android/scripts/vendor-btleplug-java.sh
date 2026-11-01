#!/usr/bin/env bash
#
# Vendors the btleplug fork's Java helper classes into bindings/kotlin/btleplug-java.
#
# The rblue Android backend shells over the forked btleplug's droidplug Java
# classes (com.nonpolynomial.btleplug.android.impl.*) and their support
# classes (io.github.gedgygedgy.rust.*). They are a hard runtime dependency of
# the backend (see crates/rblue-android/docs/platform-notes.md) and are
# packaged as plain source in the AAR project.
#
# Sources are copied from the fork checkout cargo already materialized for
# `crates/rblue-android` (pinned to rev 1af8299 in its Cargo.toml), located
# through `cargo metadata` so no checkout path is hard-coded. Each file gets a
# provenance header; the BSD-3 license text ships in NOTICE.md.
#
# The vendored tree is committed; re-run only when the pinned revision
# changes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SRC_JAVA="$ROOT/bindings/kotlin/btleplug-java/src/main/java"
NOTICE="$ROOT/bindings/kotlin/btleplug-java/NOTICE.md"

command -v cargo >/dev/null 2>&1 || { echo "error: cargo is required" >&2; exit 1; }
command -v python3 >/dev/null 2>&1 || { echo "error: python3 is required" >&2; exit 1; }

echo "==> locating btleplug fork checkout via cargo metadata"
read -r FORK_DIR REV < <(cargo metadata --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
  | python3 -c '
import json, os, sys
pkgs = [p for p in json.load(sys.stdin)["packages"] if p["name"] == "btleplug"]
if not pkgs:
    sys.exit("btleplug not found in cargo metadata")
print(os.path.dirname(pkgs[0]["manifest_path"]), pkgs[0]["source"])
')
REV="${REV#*rev=}"
REV="${REV%%#*}"
if [[ -z "$REV" || "$REV" == "None" ]]; then
  # Fall back to the revision rblue-android pins (kept in sync manually).
  REV="1af8299c291f24fca68dfb6b7ef1b5c0475892e1"
fi
JAVA_DIR="$FORK_DIR/src/droidplug/java"
[[ -d "$JAVA_DIR/src/main/java" ]] || { echo "error: java sources not found under $JAVA_DIR" >&2; exit 1; }
echo "    fork: $FORK_DIR"
echo "    rev:  $REV"

echo "==> copying java sources"
rm -rf "$SRC_JAVA"
mkdir -p "$SRC_JAVA"
cp -R "$JAVA_DIR/src/main/java/." "$SRC_JAVA/"

echo "==> adding provenance headers"
HEADER_1="// Vendored from https://github.com/rblue-rs/btleplug (src/droidplug/java) at"
HEADER_2="// rev $REV. BSD-3-Clause; see NOTICE.md in this directory for the license"
HEADER_3="// text and the vendoring rules. Do not edit by hand."
while IFS= read -r -d '' file; do
  { printf '%s\n%s\n%s\n\n' "$HEADER_1" "$HEADER_2" "$HEADER_3"; cat "$file"; } > "$file.tmp"
  mv "$file.tmp" "$file"
done < <(find "$SRC_JAVA" -type f -name '*.java' -print0)

echo "==> writing NOTICE.md"
{
  echo "# NOTICE - vendored btleplug Java classes"
  echo
  echo "The Java sources under \`src/main/java\` are vendored from"
  echo "[btleplug](https://github.com/rblue-rs/btleplug) at rev \`$REV\`"
  echo "(\`src/droidplug/java\`), the fork the rblue Android backend is a thin"
  echo "shell over. They are copied by"
  echo "\`examples/android/scripts/vendor-btleplug-java.sh\`, which re-runs"
  echo "whenever the pinned revision changes; do not edit them by hand."
  echo
  echo "Upstream license (BSD-3-Clause):"
  echo
  cat "$FORK_DIR/LICENSE.md"
} > "$NOTICE"

echo "==> done"
echo "    vendored: $SRC_JAVA"
echo "    notice:   $NOTICE"
