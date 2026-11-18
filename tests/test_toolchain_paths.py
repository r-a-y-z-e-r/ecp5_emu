"""Discovery precedence and portable record paths, without vendor installations."""
import argparse
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import toolchain_paths as paths


class ToolchainPathsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        for context in (patch.dict(os.environ, {}, clear=True),
                        patch.object(Path, "home", return_value=self.root / "home"),
                        patch.object(paths, "install_prefixes", return_value=[self.root / "install"])):
            context.start()
            self.addCleanup(context.stop)

    def source(self, name):
        root = self.root / name
        self.file(root / "libtrellis/include/Chip.hpp")
        return root

    def file(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.touch()
        return path

    def test_source_option_environment_database_parent_and_home_precedence(self):
        explicit, env, db = [self.source(name) for name in ("explicit", "env", "db")]
        home = self.source("home/FPGA/prjtrellis")
        with patch.dict(os.environ, TRELLIS_ROOT=str(env), ECP5_DB=str(db / "database")):
            self.assertEqual(paths.discover_trellis(explicit), explicit)
            self.assertEqual(paths.discover_trellis(), env)
            del os.environ["TRELLIS_ROOT"]
            self.assertEqual(paths.discover_trellis(), db)
            os.environ["ECP5_DB"] = str(self.root / "standalone-db")
            self.assertEqual(paths.discover_trellis(), home)

    def test_invalid_explicit_and_environment_overrides_fail(self):
        self.source("home/FPGA/prjtrellis")
        with self.assertRaisesRegex(ValueError, "--trellis.*TRELLIS_ROOT"):
            paths.discover_trellis(self.root / "missing")
        with patch.dict(os.environ, TRELLIS_ROOT=str(self.root / "missing")):
            with self.assertRaisesRegex(ValueError, "invalid path"):
                paths.discover_trellis()

    def test_installed_source_headers_and_library_layouts(self):
        source = self.source("install/share/trellis")
        prefix = self.root / "install"
        library = self.file(prefix / "lib64/trellis/libtrellis.so")
        self.assertEqual(paths.discover_trellis(), source)
        self.assertEqual(paths.discover_prefix(), prefix)
        self.assertEqual(paths.trellis_library(prefix), library)
        override = self.root / "override"
        self.file(override / "lib/libtrellis.dylib")
        with patch.dict(os.environ, TRELLIS_PREFIX=str(override)):
            self.assertEqual(paths.discover_prefix(), override)
            self.assertEqual(paths.discover_prefix(prefix), prefix)

    def test_database_and_nextpnr_environment_overrides(self):
        db = self.root / "database"
        self.file(db / "ECP5/LFE5U-85F/tilegrid.json")
        nextpnr = self.root / "nextpnr"
        self.file(nextpnr / "ecp5/bitstream.cc")
        with patch.dict(os.environ, ECP5_DB=str(db), NEXTPNR_ROOT=str(nextpnr)):
            self.assertEqual(paths.discover_database(), db)
            self.assertEqual(paths.discover_nextpnr(), nextpnr)

    def test_explicit_source_also_selects_its_database(self):
        source = self.source("explicit")
        self.file(source / "database/ECP5/LFE5U-85F/tilegrid.json")
        parser = argparse.ArgumentParser()
        args = argparse.Namespace(trellis=source, database=None)
        paths.resolve_toolchain(parser, args)
        self.assertEqual(args.database, source / "database")

    def test_brew_prefix_and_fallbacks(self):
        paths.install_prefixes.cache_clear()
        self.addCleanup(paths.install_prefixes.cache_clear)
        # setUp mocks this function; use the wrapped original for this test.
        original = self._install_prefixes_original
        with patch.object(paths.shutil, "which", return_value="brew"), patch.object(
                paths.subprocess, "run", return_value=argparse.Namespace(
                    returncode=0, stdout=str(self.root / "brew") + "\n")):
            self.assertEqual(original()[0], self.root / "brew")
        original.cache_clear()
        with patch.object(paths.shutil, "which", return_value=None):
            self.assertEqual(original(), [Path("/opt/homebrew"), Path("/usr/local")])

    def test_portable_record_paths(self):
        self.assertEqual(paths.portable_path(self.root / "home/FPGA/prjtrellis"), "~/FPGA/prjtrellis")
        repo_file = Path(paths.__file__).resolve()
        self.assertEqual(paths.portable_path(repo_file), "tools/toolchain_paths.py")


ToolchainPathsTests._install_prefixes_original = staticmethod(paths.install_prefixes)

if __name__ == "__main__":
    unittest.main()
