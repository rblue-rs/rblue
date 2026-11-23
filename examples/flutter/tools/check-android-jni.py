#!/usr/bin/env python3
"""Check that an APK retains every Java helper loaded by the Rust JNI bridge.

Usage: check-android-jni.py APK

Prefers `apkanalyzer` when it is on PATH or under the Android SDK. If that
tool is missing, falls back to reading defined class names from the APK's
DEX files so CI does not depend on cmdline-tools being on PATH.
"""

from __future__ import annotations

from pathlib import Path
import os
import re
import struct
import subprocess
import sys
import zipfile

repo = Path(__file__).resolve().parents[3]
package = repo / "examples/flutter/rblue_flutter"
sources = package / "android/src/main/java"
expected = {"com.rblue.RblueNative", "com.rblue.AdapterStateMonitor"}
for source in sources.rglob("*.java"):
    package_match = re.search(r"^package\s+([\w.]+);", source.read_text(), re.MULTILINE)
    assert package_match, f"Missing package: {source}"
    expected.add(f"{package_match[1]}.{source.stem}")


def sdk_roots() -> list[Path]:
    roots: list[Path] = []
    for key in ("ANDROID_SDK_ROOT", "ANDROID_HOME"):
        value = os.environ.get(key)
        if value:
            roots.append(Path(value))
    roots.extend(
        [
            Path("/usr/local/lib/android/sdk"),
            Path.home() / "Android/Sdk",
            Path.home() / "Library/Android/sdk",
            Path("/opt/android-sdk"),
            Path("/opt/homebrew/share/android-commandlinetools"),
        ]
    )
    unique: list[Path] = []
    seen: set[Path] = set()
    for root in roots:
        try:
            resolved = root.resolve()
        except OSError:
            resolved = root
        if resolved not in seen:
            seen.add(resolved)
            unique.append(root)
    return unique


def find_apkanalyzer() -> str | None:
    from shutil import which

    found = which("apkanalyzer")
    if found:
        return found
    candidates: list[Path] = []
    for root in sdk_roots():
        candidates.append(root / "cmdline-tools/latest/bin/apkanalyzer")
        candidates.extend(sorted(root.glob("cmdline-tools/*/bin/apkanalyzer")))
        candidates.append(root / "tools/bin/apkanalyzer")
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return None


def _uleb128(data: bytes, offset: int) -> tuple[int, int]:
    result = 0
    shift = 0
    while True:
        if offset >= len(data):
            raise ValueError("truncated ULEB128 in DEX")
        byte = data[offset]
        offset += 1
        result |= (byte & 0x7F) << shift
        if byte & 0x80 == 0:
            return result, offset
        shift += 7
        if shift > 28:
            raise ValueError("ULEB128 too large in DEX")


def _dex_string(data: bytes, string_ids_off: int, index: int) -> str:
    offset = struct.unpack_from("<I", data, string_ids_off + index * 4)[0]
    _, offset = _uleb128(data, offset)
    end = data.index(b"\x00", offset)
    return data[offset:end].decode("utf-8", "surrogateescape")


def dex_defined_classes(data: bytes) -> set[str]:
    if len(data) < 104 or not data.startswith(b"dex\n"):
        raise ValueError("not a DEX file")
    endian = struct.unpack_from("<I", data, 40)[0]
    if endian != 0x12345678:
        raise ValueError(f"unsupported DEX endian tag: {endian:#x}")
    string_ids_off = struct.unpack_from("<I", data, 60)[0]
    type_ids_off = struct.unpack_from("<I", data, 68)[0]
    class_defs_size, class_defs_off = struct.unpack_from("<II", data, 96)
    classes: set[str] = set()
    for i in range(class_defs_size):
        class_idx = struct.unpack_from("<I", data, class_defs_off + i * 32)[0]
        string_idx = struct.unpack_from("<I", data, type_ids_off + class_idx * 4)[0]
        descriptor = _dex_string(data, string_ids_off, string_idx)
        if descriptor.startswith("L") and descriptor.endswith(";"):
            classes.add(descriptor[1:-1].replace("/", "."))
    return classes


def classes_from_apk_dex(apk: str) -> set[str]:
    classes: set[str] = set()
    with zipfile.ZipFile(apk) as archive:
        dex_names = [
            name
            for name in archive.namelist()
            if name == "classes.dex" or re.fullmatch(r"classes\d+\.dex", name)
        ]
        if not dex_names:
            raise AssertionError(f"APK has no classes.dex: {apk}")
        for name in dex_names:
            classes.update(dex_defined_classes(archive.read(name)))
    return classes


def classes_from_apkanalyzer(tool: str, apk: str) -> set[str]:
    output = subprocess.check_output(
        [tool, "dex", "packages", "--defined-only", apk], text=True
    )
    return {line.split()[-1] for line in output.splitlines() if line.startswith("C ")}


apk = sys.argv[1]
tool = find_apkanalyzer()
if tool:
    try:
        classes = classes_from_apkanalyzer(tool, apk)
        source = f"apkanalyzer ({tool})"
    except (OSError, subprocess.CalledProcessError) as exc:
        print(f"warning: apkanalyzer failed ({exc}); falling back to DEX parsing", file=sys.stderr)
        classes = classes_from_apk_dex(apk)
        source = "DEX class_defs"
else:
    classes = classes_from_apk_dex(apk)
    source = "DEX class_defs"

missing = expected - classes
assert not missing, f"APK is missing JNI classes: {sorted(missing)}"
print(f"Verified {len(expected)} JNI helper classes in {apk} via {source}")
