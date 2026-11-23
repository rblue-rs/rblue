# Native library Releases

This is a follow-up to PR #46. Merge its validated source/artifact provenance
checks and rebuilt libraries first. This change does not run or enable the old
`Release examples` workflow, deploy to Pgyer, or publish to any package registry.
It creates no Release until one of the explicit triggers below is used.

## Triggers (ordinary commits and PRs never publish)

- **Manual:** on the main branch, run **Release native libraries** with
  `version` (a new `vX.Y.Z` semantic-version tag, optionally prerelease/build
  metadata) and `source_ref` (the exact full 40-character commit SHA).
  Only commits already merged into main with successful main-branch CI can release.
  The workflow creates that new tag only after validation. Tags created with its
  `GITHUB_TOKEN` do not start another workflow run.
- **Version tag:** deliberately push `v1.2.3` or e.g. `v1.2.3-rc.1` pointing to
  an already merged, fully checked main commit. The existing tag is verified, not
  changed. Non-semver `v*` tags start validation but fail before publication.
  Other tags, branch pushes, PRs and CI completions never trigger publication.

No tag or Release should be created merely to test this configuration.
The workflow must exist on main for manual dispatch; tagged commits must contain
this workflow and the final provenance verifier from PR #46.

## Gates and assets

The newest main-branch push or manual runs of `ci.yml`, `example-android.yml`,
`example-flutter.yml` and `example-ios.yml` for the exact chosen SHA must all
succeed. If a path-filtered workflow (e.g. iOS) did not run for this SHA, run
it manually on main at that exact commit and wait for success before releasing.
Source snapshot sync, the complete seven-library checksum set, exact
source/binary provenance, and successful recorded hosted rebuild runs are
required. An older checksum-only verifier fails closed. Failed, pending, skipped
or missing checks are not bypassed. API failures also stop publication.

Assets: Android (three ABIs), iOS XCFramework, macOS universal XCFramework,
Windows x64 DLL, and the standalone source snapshot, each in a versioned tar.gz;
headers, build instructions, licenses and native build provenance are included.
`SHA256SUMS.txt` and `release-provenance.json` identify the exact release commit.
These are the validated **packaged rblue-c native libraries**, not newly built
example applications or all Rust backends. A stale source/binary pair cannot be
released; rebuild/update it first through normal reviewed CI.

Only the final job has contents-write permission. It uses a standard public
`ubuntu-latest` hosted runner, no Actions cache or artifact uploads, no paid
runner, and no package/deployment secrets. Files remain local until uploaded as
Release assets. The tag is checked again immediately before publishing.

Existing Releases (including drafts) are never edited, reused or overwritten;
manual dispatch also rejects an existing tag. There is no `--clobber`. The new
Release is assembled as a draft and published only after all asset uploads
succeed; it does not automatically replace GitHub's latest-release designation.
Prerelease versions are marked prerelease. A failed publish can leave a tag or
an incomplete draft: stop and inspect it, do not automatically retry, delete or
overwrite it. A maintainer must decide how to recover or use a new version.

## Local validation (no publication)

```sh
python3 -m unittest discover -s tools/release -p 'test_*.py'
actionlint .github/workflows/release-native.yml .github/workflows/release-policy.yml
```
