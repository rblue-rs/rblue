"""Safety regression tests for manual native binary recording."""
import importlib.util
from pathlib import Path
import unittest
import tempfile

MODULE = Path(__file__).with_name("record-native-rebuild.py")

class NativeRebuildTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location("record_native", MODULE)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)

    def test_allowlist_has_exactly_seven_unique_binaries(self):
        paths = sum((self.module.platform_paths(p) for p in ("android", "apple", "windows")), [])
        self.assertEqual(len(paths), 7)
        self.assertEqual(len(set(paths)), 7)
        self.assertTrue(all(path.endswith((".so", ".a", ".dll")) for path in paths))

    def test_source_manifest_changes_with_source_but_excludes_build_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "Cargo.toml"
            source.write_text("before")
            before = self.module.source_checksums(root)
            (root / "target").mkdir()
            (root / "target" / "output").write_text("build product")
            self.assertEqual(before, self.module.source_checksums(root))
            source.write_text("after")
            self.assertNotEqual(before, self.module.source_checksums(root))

    def test_unknown_platform_rejected(self):
        with self.assertRaises(ValueError):
            self.module.platform_paths("../../main")

    def test_changed_rebuild_input_rejected(self):
        tree = [{"path": "examples/flutter/rblue_flutter/native/source", "sha": "changed"}]
        with self.assertRaisesRegex(RuntimeError, "changed"):
            self.module.assert_inputs_unchanged({"examples/flutter/rblue_flutter/native/source": "expected"}, tree)

    def test_unchanged_rebuild_input_accepted(self):
        self.module.assert_inputs_unchanged({"source": "same"}, [{"path": "source", "sha": "same"}])

if __name__ == "__main__":
    unittest.main()
