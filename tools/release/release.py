#!/usr/bin/env python3
"""Fail-closed native Release preparation; publication is workflow-only."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
from urllib.parse import quote

CI_PATHS = tuple(f".github/workflows/{name}.yml" for name in
                 ("ci", "example-android", "example-flutter", "example-ios"))
NUMBER = r"(?:0|[1-9][0-9]*)"
IDENT = r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
SEMVER = rf"v{NUMBER}\.{NUMBER}\.{NUMBER}(?:-{IDENT}(?:\.{IDENT})*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"


def validate_tag(tag):
    if not re.fullmatch(SEMVER, tag):
        raise ValueError("version must be a v-prefixed semantic version")
    return tag


def request(event, ref, version, source):
    if event == "push" and ref.startswith("refs/tags/"):
        return validate_tag(ref.removeprefix("refs/tags/")), ref
    if event == "workflow_dispatch" and ref == "refs/heads/main":
        if not re.fullmatch(r"[0-9a-f]{40}", source):
            raise ValueError("source_ref must be a full 40-character commit SHA")
        return validate_tag(version), source
    raise ValueError("only manual dispatch on main or version-tag push may release")


def require_ci(runs):
    for path in CI_PATHS:
        candidates = [r for r in runs if r["path"] == path and r["event"] in ("push", "workflow_dispatch")]
        latest = max(candidates, key=lambda r: r["id"], default={})
        if latest.get("status") != "completed" or latest.get("conclusion") != "success":
            raise ValueError(f"latest main push/manual CI must succeed for {path}")


def run(*args, cwd=None):
    return subprocess.check_output(args, text=True, cwd=cwd).strip()


def api(path):
    return json.loads(run("gh", "api", path))


def pages(path):
    return json.loads(run("gh", "api", "--paginate", "--slurp", path))


def guard(repo, tag, sha, manual):
    # Revalidate against the server, not just checkout refs, immediately before publishing.
    releases = [r for page in pages(f"repos/{repo}/releases?per_page=100") for r in page]
    if any(r["tag_name"] == tag for r in releases):
        raise ValueError("release (including draft) already exists; never overwrite")
    tags = [t for page in pages(f"repos/{repo}/tags?per_page=100") for t in page]
    existing = next((t for t in tags if t["name"] == tag), None)
    if manual and existing:
        raise ValueError("manual release requires a new, unused version tag")
    if not manual and (not existing or api(f"repos/{repo}/commits/{quote(tag, safe='')}")["sha"] != sha):
        raise ValueError("pushed tag no longer points to the verified source")
    runs = [r for page in pages(f"repos/{repo}/actions/runs?head_sha={sha}&per_page=100")
            for r in page["workflow_runs"] if r["head_sha"] == sha and r["head_branch"] == "main"]
    require_ci(runs)


def metadata():
    tag, source = request(os.environ["EVENT_NAME"], os.environ["EVENT_REF"],
                          os.environ.get("INPUT_VERSION", ""), os.environ.get("INPUT_SOURCE", ""))
    sha = run("git", "rev-parse", f"{source}^{{commit}}")
    subprocess.run(["git", "merge-base", "--is-ancestor", sha, "origin/main"], check=True)
    guard(os.environ["GITHUB_REPOSITORY"], tag, sha, os.environ["EVENT_NAME"] == "workflow_dispatch")
    with open(os.environ["GITHUB_OUTPUT"], "a") as out:
        out.write(f"tag={tag}\nsha={sha}\n")


def prepare(root, tag, sha):
    root = Path(root).resolve()
    if run("git", "rev-parse", "HEAD", cwd=root) != sha:
        raise ValueError("checkout does not match verified commit")
    subprocess.run(["bash", "examples/flutter/tools/check-snapshot-sync.sh"], cwd=root, check=True)
    verifier = root / "examples/flutter/tools/verify-native-artifacts.py"
    spec = importlib.util.spec_from_file_location("native_verifier", verifier)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if not callable(getattr(module, "verify_provenance", None)):
        raise ValueError("requires merged PR #46 source/artifact provenance gate")
    module.main()
    package = root / "examples/flutter/rblue_flutter"
    # Verify rebuild runs really succeeded for the provenance's recorded source commit.
    for platform in ("android", "apple", "windows"):
        report = json.loads((package / f"native/provenance/{platform}.json").read_text())
        run_id = report["workflow_run"].rsplit("/", 1)[1]
        result = api(f"repos/{os.environ['GITHUB_REPOSITORY']}/actions/runs/{run_id}")
        if result["conclusion"] != "success" or result["head_sha"] != report["source_commit"]:
            raise ValueError(f"unverified hosted rebuild: {platform}")
    output = Path("release-assets").resolve()
    output.mkdir(exist_ok=False)
    groups = {
        "android": ["android/src/main/jniLibs"],
        "ios": ["ios/rblue_flutter/RblueC.xcframework"],
        "macos": ["macos/rblue_flutter/RblueC.xcframework"],
        "windows": ["windows/x64/rblue_c.dll"],
        "source": ["native/source"],
    }
    common = ["native/include", "native/provenance", "native/BUILDING.md"]
    licenses = [p.relative_to(package).as_posix() for p in package.glob("LICENSE*")]
    for platform, paths in groups.items():
        with tarfile.open(output / f"rblue-native-{tag}-{platform}.tar.gz", "w:gz") as archive:
            for relative in paths + common + licenses:
                path = package / relative
                # No build output or untracked files; archives contain only committed inputs.
                tracked = run("git", "ls-files", "--", str(path), cwd=root).splitlines()
                if not tracked:
                    raise ValueError(f"missing tracked release input: {relative}")
                for name in tracked:
                    archive.add(root / name, arcname=(root / name).relative_to(package), recursive=False)
    (output / "release-provenance.json").write_text(json.dumps({
        "repository": os.environ["GITHUB_REPOSITORY"], "tag": tag, "source_commit": sha,
        "release_workflow": os.environ["GITHUB_RUN_ID"],
        "scope": "Packaged rblue-c native libraries; no application or registry publication",
    }, indent=2) + "\n")
    (output / "SHA256SUMS.txt").write_text("".join(
        f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n"
        for p in sorted(output.iterdir())))


if __name__ == "__main__":
    if sys.argv[1] == "metadata":
        metadata()
    elif sys.argv[1] == "guard":
        guard(os.environ["GITHUB_REPOSITORY"], validate_tag(os.environ["RELEASE_TAG"]),
              os.environ["SOURCE_SHA"], os.environ["EVENT_NAME"] == "workflow_dispatch")
    elif sys.argv[1] == "prepare":
        prepare("source", validate_tag(os.environ["RELEASE_TAG"]), os.environ["SOURCE_SHA"])
    else:
        raise SystemExit("unknown command")
