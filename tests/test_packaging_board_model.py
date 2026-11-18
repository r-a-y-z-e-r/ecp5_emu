"""Board-model selection works without a GUI, a database or Blender."""
import contextlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("resources", ROOT / "packaging/resources.py")
resources = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(resources)


class BoardModelTests(unittest.TestCase):
    def test_candidate_order_and_missing_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidates = [root / name for name in ("env", "out", "main-out", "assets")]
            for path in candidates:
                path.write_bytes(b"glTF")
            for expected in candidates:
                self.assertEqual(resources.find_board_model(candidates), expected)
                expected.unlink()
            self.assertIsNone(resources.find_board_model(candidates))

    def test_pointer_warns_and_falls_through(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            pointer, model = root / "pointer.glb", root / "model.glb"
            pointer.write_text("version https://git-lfs.github.com/spec/v1\n"
                               "oid sha256:" + "0" * 64 + "\nsize 11507464\n")
            model.write_bytes(b"glTF")
            warning = io.StringIO()
            with contextlib.redirect_stderr(warning):
                self.assertEqual(resources.find_board_model([pointer, model]), model)
                self.assertIsNone(resources.find_board_model([pointer]))
            self.assertIn(str(pointer), warning.getvalue())
            self.assertIn("git LFS pointer", warning.getvalue())
            self.assertIn("git lfs pull", warning.getvalue())


if __name__ == "__main__":
    unittest.main()
