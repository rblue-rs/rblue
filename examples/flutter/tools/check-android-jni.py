#!/usr/bin/env python3
"""Check that an APK retains every Java helper loaded by the Rust JNI bridge.

Usage: check-android-jni.py APK (apkanalyzer must be on PATH).
"""

from pathlib import Path
import re
import subprocess
import sys

repo = Path(__file__).resolve().parents[3]
package = repo / "examples/flutter/rblue_flutter"
sources = package / "android/src/main/java"
expected = {"com.rblue.RblueNative"}
for source in sources.rglob("*.java"):
    package = re.search(r"^package\s+([\w.]+);", source.read_text(), re.MULTILINE)
    assert package, f"Missing package: {source}"
    expected.add(f"{package[1]}.{source.stem}")

output = subprocess.check_output(
    ["apkanalyzer", "dex", "packages", "--defined-only", sys.argv[1]], text=True
)
classes = {
    line.split()[-1] for line in output.splitlines() if line.startswith("C ")
}
missing = expected - classes
assert not missing, f"APK is missing JNI classes: {sorted(missing)}"
print(f"Verified {len(expected)} JNI helper classes in {sys.argv[1]}")
