"""Regression checks for source-to-binary provenance validation."""
import importlib.util
from hashlib import sha256
import json
from pathlib import Path
import tempfile
import subprocess
import unittest


class ProvenanceTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            "verify_native", Path(__file__).with_name("verify-native-artifacts.py"))
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)

    def test_matching_sources_and_binaries_pass_but_changed_binary_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory)
            (package / "native/provenance").mkdir(parents=True)
            (package / "native/source").mkdir()
            (package / "native/source/Cargo.toml").write_text("source")
            for platform, paths in self.module.ARTIFACTS.items():
                for relative in paths:
                    artifact = package / relative
                    artifact.parent.mkdir(parents=True, exist_ok=True)
                    artifact.write_bytes(b"compiled binary")
                report = {
                    "platform": platform,
                    "source_files": self.module.source_checksums(package / "native/source"),
                    "source_commit": "a" * 40, "source_tree": "b" * 40,
                    "workflow_run": "https://github.com/rblue-rs/rblue/actions/runs/123",
                    "sha256": {relative: sha256(b"compiled binary").hexdigest() for relative in paths},
                }
                (package / f"native/provenance/{platform}.json").write_text(json.dumps(report))
            self.assertEqual(self.module.verify_provenance(package), [])
            (package / "windows/x64/rblue_c.dll").write_bytes(b"stale binary")
            self.assertTrue(any("binary provenance mismatch" in error
                                for error in self.module.verify_provenance(package)))

    def test_windows_git_checkout_filters_match_but_real_source_edit_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory)
            source = package / "native/source"
            source.mkdir(parents=True)
            file = source / "Cargo.toml"
            file.write_bytes(b"[workspace]\nmembers = []\n")
            def git(*args):
                return subprocess.check_output(["git", "-C", str(package), *args], stderr=subprocess.DEVNULL)
            git("init")
            git("add", ".")
            git("-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-m", "source")
            checkout = git("-c", "core.autocrlf=true", "cat-file", "--filters", "HEAD:native/source/Cargo.toml")
            report = {"source_tree": git("rev-parse", "HEAD:native/source").decode().strip(),
                      "source_files": {"Cargo.toml": sha256(checkout).hexdigest()}}
            self.assertTrue(self.module.matches_windows_git_checkout(package, report))
            file.write_bytes(b'[workspace]\nmembers = ["different"]\n')
            self.assertFalse(self.module.matches_windows_git_checkout(package, report))

    def test_missing_provenance_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            errors = self.module.verify_provenance(Path(directory))
            self.assertTrue(errors)
            self.assertTrue(any("missing" in error for error in errors))

    def test_source_drift_fails_even_when_binary_hash_matches(self):
        with tempfile.TemporaryDirectory() as directory:
            package = Path(directory)
            (package / "native/provenance").mkdir(parents=True)
            (package / "native/source").mkdir()
            (package / "native/source/Cargo.toml").write_text("changed source")
            report = {"platform": "windows", "source_files": {"Cargo.toml": "0" * 64},
                      "sha256": {"windows/x64/rblue_c.dll": "0" * 64}}
            (package / "native/provenance/windows.json").write_text(json.dumps(report))
            errors = self.module.verify_provenance(package)
            self.assertTrue(any("source" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
