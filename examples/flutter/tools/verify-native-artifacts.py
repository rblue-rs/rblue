#!/usr/bin/env python3
"""Verify every packaged native artifact against native/BUILDING.md."""

from hashlib import sha256
from pathlib import Path
import re
import sys

package = Path(__file__).resolve().parents[1] / "rblue_flutter"
building = package / "native" / "BUILDING.md"
entries = re.findall(r"^([0-9a-f]{64})  (\S+)$", building.read_text(), re.MULTILINE)
if not entries:
    raise SystemExit(f"No SHA-256 entries found in {building}")

failed = False
for expected, relative in entries:
    artifact = package / relative
    if not artifact.is_file():
        print(f"missing: {relative}", file=sys.stderr)
        failed = True
        continue
    actual = sha256(artifact.read_bytes()).hexdigest()
    if actual != expected:
        print(
            f"checksum mismatch: {relative}\n  expected {expected}\n  actual   {actual}",
            file=sys.stderr,
        )
        failed = True
    else:
        print(f"verified {relative}: {actual}")

if failed:
    raise SystemExit(1)
print(f"Verified {len(entries)} packaged native artifacts.")
