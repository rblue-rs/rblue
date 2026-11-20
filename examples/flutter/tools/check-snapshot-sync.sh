#!/usr/bin/env bash
#
# Verify that the package-local Rust source snapshot used to rebuild the
# rblue_flutter native artifacts (examples/flutter/rblue_flutter/native/source)
# still matches the workspace crates it was copied from.
#
# For every crate present in the snapshot:
#   - every file under the workspace crate's src/ plus its build.rs (if any)
#     must exist in the snapshot and be byte-identical;
#   - every file the snapshot ships for that crate must be byte-identical to
#     the workspace copy, except the standalone manifests listed in
#     ALLOWED_DIFFS, which intentionally drop backends the package omits.
#
# Tests, docs, and examples that the snapshot intentionally leaves out are
# not required. Run from anywhere; exits non-zero on any drift.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
WORKSPACE_CRATES="$REPO_ROOT/crates"
SNAPSHOT_CRATES="$REPO_ROOT/examples/flutter/rblue_flutter/native/source/crates"

# Manifests whose standalone feature/dependency set deliberately differs.
ALLOWED_DIFFS=(
  "rblue/Cargo.toml"
  "rblue-c/Cargo.toml"
)

is_allowed_diff() {
  local candidate="$1" allowed
  for allowed in "${ALLOWED_DIFFS[@]}"; do
    [[ "$candidate" == "$allowed" ]] && return 0
  done
  return 1
}

failed=0
checked=0

compare() {
  local relative="$1"
  local workspace_file="$WORKSPACE_CRATES/$relative"
  local snapshot_file="$SNAPSHOT_CRATES/$relative"
  if [[ ! -f "$workspace_file" ]]; then
    echo "snapshot-only file (not in crates/): $relative" >&2
    failed=1
    return
  fi
  if [[ ! -f "$snapshot_file" ]]; then
    echo "missing from snapshot: $relative" >&2
    failed=1
    return
  fi
  checked=$((checked + 1))
  if is_allowed_diff "$relative"; then
    return
  fi
  if ! cmp -s "$workspace_file" "$snapshot_file"; then
    echo "out of sync: $relative" >&2
    failed=1
  fi
}

shopt -s nullglob
crates=()
for dir in "$SNAPSHOT_CRATES"/*/; do
  crates+=("$(basename "$dir")")
done
if [[ ${#crates[@]} -eq 0 ]]; then
  echo "error: no crates found under $SNAPSHOT_CRATES" >&2
  exit 1
fi

for crate in "${crates[@]}"; do
  if [[ ! -d "$WORKSPACE_CRATES/$crate" ]]; then
    echo "snapshot crate has no workspace counterpart: $crate" >&2
    failed=1
    continue
  fi
  # Workspace sources and build script must all be in the snapshot.
  while IFS= read -r -d '' file; do
    relative="${file#"$WORKSPACE_CRATES/"}"
    compare "$relative"
  done < <(
    {
      [[ -d "$WORKSPACE_CRATES/$crate/src" ]] &&
        find "$WORKSPACE_CRATES/$crate/src" -type f -print0
      [[ -f "$WORKSPACE_CRATES/$crate/build.rs" ]] &&
        printf '%s\0' "$WORKSPACE_CRATES/$crate/build.rs"
      true
    } | sort -z
  )
  # Everything the snapshot ships must match the workspace too.
  while IFS= read -r -d '' file; do
    relative="${file#"$SNAPSHOT_CRATES/"}"
    # src/ files and build.rs that exist in the workspace were compared above.
    case "${relative#"$crate/"}" in
      src/* | build.rs) [[ -f "$WORKSPACE_CRATES/$relative" ]] && continue ;;
    esac
    compare "$relative"
  done < <(find "$SNAPSHOT_CRATES/$crate" -path '*/target' -prune -o -type f -print0 | sort -z)
done

if [[ "$failed" != 0 ]]; then
  cat >&2 <<'MSG'
error: examples/flutter/rblue_flutter/native/source is out of sync with crates/.
Copy the changed workspace files into the snapshot (keep the standalone
Cargo.toml differences) so build-libs.sh rebuilds the same code.
MSG
  exit 1
fi
echo "Flutter native source snapshot in sync: $checked files across ${#crates[@]} crates."
