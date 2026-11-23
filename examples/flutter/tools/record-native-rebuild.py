#!/usr/bin/env python3
"""Record manual hosted rebuild outputs directly in the existing repair branch.

Uses documented Git database APIs, not billable Actions artifact storage.
Only the seven packaged binaries and per-platform provenance can be written.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import platform as host_platform
import subprocess

PACKAGE = "examples/flutter/rblue_flutter/"
BRANCH = "ci/free-standard-hosted"
REPOSITORY = "rblue-rs/rblue"
INPUTS = [
    PACKAGE + "native/source",
    PACKAGE + "native/include",
    "examples/flutter/tools/build-libs.sh",
    "examples/flutter/tools/build-windows.ps1",
    "examples/flutter/tools/record-native-rebuild.py",
]


def platform_paths(platform):
    paths = {
        "android": [f"android/src/main/jniLibs/{abi}/librblue_c.so"
                    for abi in ("arm64-v8a", "armeabi-v7a", "x86_64")],
        "apple": [
            "ios/rblue_flutter/RblueC.xcframework/ios-arm64/librblue_c.a",
            "ios/rblue_flutter/RblueC.xcframework/ios-arm64-simulator/librblue_c.a",
            "macos/rblue_flutter/RblueC.xcframework/macos-arm64_x86_64/librblue_c.a",
        ],
        "windows": ["windows/x64/rblue_c.dll"],
    }
    if platform not in paths:
        raise ValueError("Unknown platform")
    return paths[platform]


def source_checksums(root):
    return {
        path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(root.rglob("*"))
        if path.is_file() and "target" not in path.relative_to(root).parts
    }


def assert_inputs_unchanged(expected, tree):
    actual = {item["path"]: item["sha"] for item in tree}
    for path, sha in expected.items():
        if actual.get(path) != sha:
            raise RuntimeError(f"Rebuild input changed during build: {path}")


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def api(path, body=None, method=None):
    args = ["gh", "api", f"repos/{REPOSITORY}/{path}"]
    if body is not None:
        args += ["--method", method or "POST", "--input", "-"]
    return json.loads(subprocess.check_output(
        args, input=None if body is None else json.dumps(body).encode()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["android", "apple", "windows"])
    args = parser.parse_args()
    if (os.environ.get("GITHUB_REPOSITORY") != REPOSITORY
            or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch"
            or os.environ.get("GITHUB_REF") != "refs/heads/" + BRANCH):
        raise RuntimeError("Manual repair-branch workflow only")
    source_commit = command("git", "rev-parse", "HEAD")
    expected = {path: command("git", "rev-parse", f"HEAD:{path}") for path in INPUTS}
    head = api("git/ref/heads/" + BRANCH)["object"]["sha"]
    remote_tree = api(f"git/trees/{head}?recursive=1")
    if remote_tree.get("truncated"):
        raise RuntimeError("Cannot validate truncated tree")
    assert_inputs_unchanged(expected, remote_tree["tree"])
    entries = []
    hashes = {}
    for relative in platform_paths(args.platform):
        path = Path(PACKAGE + relative)
        if path.is_symlink() or not path.is_file():
            raise RuntimeError(f"Expected built binary missing: {path}")
        data = path.read_bytes()
        if not data or len(data) > 50 * 1024 * 1024:
            raise RuntimeError(f"Unexpected binary size: {path}")
        hashes[relative] = hashlib.sha256(data).hexdigest()
        blob = api("git/blobs", {"content": base64.b64encode(data).decode(), "encoding": "base64"})
        if blob["sha"] != command("git", "hash-object", str(path)):
            raise RuntimeError(f"Uploaded blob differs: {path}")
        entries.append({"path": str(path).replace("\\", "/"), "mode": "100644",
                        "type": "blob", "sha": blob["sha"]})
    provenance = {
        "source_commit": source_commit,
        "source_tree": expected[PACKAGE + "native/source"],
        "source_files": source_checksums(Path(PACKAGE + "native/source")),
        "build_inputs": expected,
        "workflow_run": f"https://github.com/{REPOSITORY}/actions/runs/{os.environ['GITHUB_RUN_ID']}",
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
        "platform": args.platform,
        "host": host_platform.platform(),
        "rustc": command("rustc", "--version"),
        "cargo": command("cargo", "--version"),
        "sha256": hashes,
    }
    if args.platform == "apple":
        provenance["xcode"] = command("xcodebuild", "-version")
    if args.platform == "android":
        ndk = Path(os.environ["ANDROID_NDK_HOME"])
        provenance["android_ndk"] = (ndk / "source.properties").read_text()
        provenance["android_api"] = os.environ.get("ANDROID_API", "24")
    if args.platform == "windows":
        vswhere = Path(os.environ["ProgramFiles(x86)"]) / "Microsoft Visual Studio/Installer/vswhere.exe"
        provenance["visual_studio"] = command(str(vswhere), "-latest", "-products", "*", "-format", "json")
    entries.append({"path": PACKAGE + f"native/provenance/{args.platform}.json", "mode": "100644",
                    "type": "blob", "content": json.dumps(provenance, indent=2) + "\n"})
    tree = api("git/trees", {"base_tree": remote_tree["sha"], "tree": entries})
    commit = api("git/commits", {
        "message": f"build(flutter): refresh {args.platform} native binaries from {source_commit[:12]}",
        "tree": tree["sha"], "parents": [head],
    })
    api("git/refs/heads/" + BRANCH, {"sha": commit["sha"], "force": False}, "PATCH")
    print(json.dumps({"commit": commit["sha"], "provenance": provenance}, indent=2))


if __name__ == "__main__":
    main()
