import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)

class ReleaseTests(unittest.TestCase):
    def test_semver(self):
        for tag in ("v1.2.3", "v0.0.1-rc.1", "v1.2.3-beta+build.4"):
            self.assertEqual(release.validate_tag(tag), tag)
        for tag in ("main", "examples-v1.2.3", "v01.2.3", "v1.2", "v1.2.3-01", "v1.2.3\n", "v1.2.3;id"):
            with self.assertRaises(ValueError):
                release.validate_tag(tag)

    def test_only_manual_main_or_version_tag(self):
        self.assertEqual(release.request("workflow_dispatch", "refs/heads/main", "v1.2.3", "a" * 40), ("v1.2.3", "a" * 40))
        self.assertEqual(release.request("push", "refs/tags/v1.2.3", "", ""), ("v1.2.3", "refs/tags/v1.2.3"))
        for args in (("push", "refs/heads/main", "", ""), ("pull_request", "refs/pull/1/merge", "", ""), ("workflow_run", "", "", ""), ("workflow_dispatch", "refs/heads/evil", "v1.2.3", "a"*40), ("workflow_dispatch", "refs/heads/main", "v1.2.3", "main")):
            with self.assertRaises(ValueError):
                release.request(*args)

    def test_requires_latest_success_for_all_workflows(self):
        runs = [{"id": n, "path": p, "conclusion": "success", "status": "completed", "event": "push"} for n, p in enumerate(release.CI_PATHS)]
        release.require_ci(runs)
        release.require_ci([dict(r, event="workflow_dispatch") for r in runs])
        with self.assertRaises(ValueError):
            release.require_ci(runs[:-1])
        with self.assertRaises(ValueError):
            release.require_ci(runs + [dict(runs[0], id=100, conclusion="failure")])
        with self.assertRaises(ValueError):
            release.require_ci([dict(r, event="pull_request") for r in runs])

    def test_existing_release_including_draft_is_never_overwritten(self):
        for draft in (True, False):
            with patch.object(release, "pages", return_value=[[{"tag_name": "v1.2.3", "draft": draft}]]):
                with self.assertRaisesRegex(ValueError, "already exists"):
                    release.guard("rblue-rs/rblue", "v1.2.3", "a" * 40, False)

    def test_manual_existing_tag_is_rejected(self):
        with patch.object(release, "pages", side_effect=[[[]], [[{"name": "v1.2.3"}]]]):
            with self.assertRaisesRegex(ValueError, "new, unused"):
                release.guard("rblue-rs/rblue", "v1.2.3", "a" * 40, True)

    def test_tag_must_still_match_source(self):
        with patch.object(release, "pages", side_effect=[[[]], [[{"name": "v1.2.3"}]]]), patch.object(release, "api", return_value={"sha": "b" * 40}):
            with self.assertRaisesRegex(ValueError, "no longer points"):
                release.guard("rblue-rs/rblue", "v1.2.3", "a" * 40, False)

    def test_api_failure_is_not_treated_as_nonexistent_release(self):
        with patch.object(release, "pages", side_effect=RuntimeError("API unavailable")):
            with self.assertRaises(RuntimeError):
                release.guard("rblue-rs/rblue", "v1.2.3", "a" * 40, True)

    def test_no_automatic_branch_or_pr_release(self):
        workflow = Path(".github/workflows/release-native.yml").read_text()
        self.assertIn('tags: ["v*"]', workflow)
        for forbidden in ("pull_request:", "workflow_run:", "branches:", "self-hosted", "upload-artifact", "actions/cache", "--clobber"):
            self.assertNotIn(forbidden, workflow)
        self.assertEqual(workflow.count("contents: write"), 1)

if __name__ == "__main__":
    unittest.main()
