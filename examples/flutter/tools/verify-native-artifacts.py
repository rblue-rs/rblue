#!/usr/bin/env python3
"""Verify packaged native checksums and their hosted-build source provenance."""
from hashlib import sha256
import json
from pathlib import Path
import re
import sys
import subprocess

ARTIFACTS = {
    "android": [f"android/src/main/jniLibs/{abi}/librblue_c.so"
                for abi in ("arm64-v8a", "armeabi-v7a", "x86_64")],
    "apple": [
        "ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a",
        "ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a",
        "macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a",
    ],
    "windows": ["windows/x64/rblue_c.dll"],
}


def source_checksums(root):
    return {
        path.relative_to(root).as_posix(): sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob("*"))
        if path.is_file() and "target" not in path.relative_to(root).parts
    }


def matches_windows_git_checkout(package, report):
    """Match Windows bytes through Git's real clean/smudge rules, never ignore drift."""
    package = package.resolve()
    source = package / "native/source"
    if set(report.get("source_files", {})) != set(source_checksums(source)):
        return False
    def git(*args):
        return subprocess.check_output(
            ["git", "-C", str(package), *args], stderr=subprocess.DEVNULL)
    try:
        repo = Path(git("rev-parse", "--show-toplevel").decode().strip())
        relative = source.relative_to(repo).as_posix()
        if git("rev-parse", f"HEAD:{relative}").decode().strip() != report.get("source_tree"):
            return False
        for path, expected in report["source_files"].items():
            tracked_path = f"{relative}/{path}"
            committed_blob = git("rev-parse", f"HEAD:{tracked_path}").strip()
            actual_blob = git("hash-object", f"--path={tracked_path}", str(source / path)).strip()
            if committed_blob != actual_blob:
                return False
            windows_bytes = git("-c", "core.autocrlf=true", "cat-file", "--filters", f"HEAD:{tracked_path}")
            if sha256(windows_bytes).hexdigest() != expected:
                return False
        return True
    except (OSError, subprocess.CalledProcessError, ValueError):
        return False


def verify_provenance(package):
    errors = []
    sources = source_checksums(package / "native/source")
    for platform, paths in ARTIFACTS.items():
        report_path = package / f"native/provenance/{platform}.json"
        if not report_path.is_file():
            errors.append(f"missing provenance: {platform}")
            continue
        report = json.loads(report_path.read_text())
        source_matches = bool(sources) and report.get("source_files") == sources
        if not source_matches and platform == "windows":
            source_matches = matches_windows_git_checkout(package, report)
        if not source_matches:
            errors.append(f"source provenance mismatch: {platform}")
        if report.get("platform") != platform or set(report.get("sha256", {})) != set(paths):
            errors.append(f"unexpected artifact set: {platform}")
            continue
        if not all(re.fullmatch(r"[0-9a-f]{40}", report.get(field, ""))
                   for field in ("source_commit", "source_tree")):
            errors.append(f"missing source revision: {platform}")
        if not re.fullmatch(r"https://github.com/rblue-rs/rblue/actions/runs/[0-9]+",
                            report.get("workflow_run", "")):
            errors.append(f"missing hosted rebuild run: {platform}")
        for relative in paths:
            artifact = package / relative
            if not artifact.is_file() or sha256(artifact.read_bytes()).hexdigest() != report["sha256"][relative]:
                errors.append(f"binary provenance mismatch: {relative}")
    return errors


def main():
    package = Path(__file__).resolve().parents[1] / "rblue_flutter"
    building = package / "native/BUILDING.md"
    entries = re.findall(r"^([0-9a-f]{64})  (\S+)$", building.read_text(), re.MULTILINE)
    expected_paths = sum(ARTIFACTS.values(), [])
    if sorted(relative for _, relative in entries) != sorted(expected_paths):
        raise SystemExit("BUILDING.md must list exactly the seven packaged native artifacts")
    errors = verify_provenance(package)
    windows_metadata = (package / "windows/x64/ARTIFACT.txt").read_text()
    metadata_hashes = re.findall(r"^    ([0-9a-f]{64})$", windows_metadata, re.MULTILINE)
    windows_hashes = [digest for digest, relative in entries if relative == "windows/x64/rblue_c.dll"]
    if metadata_hashes != windows_hashes:
        errors.append("Windows ARTIFACT.txt checksum differs from BUILDING.md")
    for expected, relative in entries:
        artifact = package / relative
        if not artifact.is_file():
            errors.append(f"missing: {relative}")
            continue
        actual = sha256(artifact.read_bytes()).hexdigest()
        if actual != expected:
            errors.append(f"checksum mismatch: {relative}\n  expected {expected}\n  actual   {actual}")
        else:
            print(f"verified {relative}: {actual}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        raise SystemExit(1)
    print(f"Verified {len(entries)} native artifacts, source manifests, and hosted rebuild provenance.")


if __name__ == "__main__":
    main()
