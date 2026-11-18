"""Packaging regressions that do not require the Linux suite or native tools."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from stage import stage_tool_links
from native import Closure
import stage as toolchain_stage

spec = importlib.util.spec_from_file_location('staged_examples', Path(__file__).parents[1] / 'examples.py')
examples = importlib.util.module_from_spec(spec)
spec.loader.exec_module(examples)

spec = importlib.util.spec_from_file_location('toolchain_notices', Path(__file__).parents[1] / 'notices.py')
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class PackagingTests(unittest.TestCase):
    def test_generated_toolchain_notices_include_relinking_on_both_platforms(self):
        for layout in ['Contents/Resources', 'share']:
            with self.subTest(layout=layout), tempfile.TemporaryDirectory() as temporary:
                resources = Path(temporary) / 'package' / layout
                toolchain = resources / 'toolchain'
                zig = toolchain / 'zig-0.17.1'
                zig.mkdir(parents=True)
                (zig / 'LICENSE').write_text('Zig licence fixture\n')
                (toolchain / 'components.json').write_text(json.dumps({
                    'suite': {'version': '2026-10-22'},
                    'packaging': {'zig': {'version': '0.17.1'},
                                  'suite_reports': [], 'notice_files': []}}))
                output = resources / 'THIRD_PARTY_NOTICES.txt'
                with patch.object(notices, 'ensure_notices', return_value=[]), output.open('w') as stream:
                    notices.append_toolchain_notices(stream, toolchain)
                text = output.read_text()
                self.assertIn('Bundled toolchain: relinking, source delivery and runtime exceptions', text)
                self.assertIn('## GCC and GHDL runtime exceptions', text)
                self.assertIn('share/toolchain/lib', text)
                self.assertIn('Contents/Resources/toolchain/lib', text)
                self.assertIn('Contents/Resources/toolchain/Frameworks', text)
                self.assertNotIn('Contents/Frameworks', text)

    def test_suite_copies_record_relative_origins(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            suite, top = root / 'suite', root / 'staged'
            (suite / 'lib').mkdir(parents=True)
            (suite / 'lib/source.o').write_bytes(b'/Users/runner/work/source.cc\x00')
            (suite / 'lib/alias.o').symlink_to('source.o')
            closure = Closure(suite, top, 'linux')
            closure.copy(suite / 'lib')
            self.assertEqual(closure.origins, {
                'lib/source.o': {'root': 'suite', 'path': 'lib/source.o', 'kind': 'file'},
                'lib/alias.o': {'root': 'suite', 'path': 'lib/alias.o', 'kind': 'symlink'}})

    def test_zig_copies_record_inputs_but_not_generated_wrappers(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source, top = root / 'zig-input', root / 'toolchain'
            (source / 'lib/headers').mkdir(parents=True)
            (source / 'lib/libc/host').mkdir(parents=True)
            top.mkdir()
            for name in ['zig', 'LICENSE', 'lib/headers/stdio.h', 'lib/libc/host/stub.s']:
                (source / name).write_text('upstream bytes\n')
            (source / 'lib/headers/alias.h').symlink_to('stdio.h')
            config = {'zig': {'version': '0.17.1', 'support': ['headers'],
                              'platforms': {'test': {'libc': ['host']}}}}
            origins = {}
            def wrapper(destination, *_):
                (destination / 'clang').write_text('generated wrapper')
                (destination / 'clang-version.txt').write_text('generated identity')
            with patch.object(toolchain_stage, 'clang_wrapper', side_effect=wrapper):
                toolchain_stage.stage_zig(source, top, config, 'test', origins)
            expected = {'zig', 'LICENSE', 'lib/headers/stdio.h', 'lib/headers/alias.h', 'lib/libc/host/stub.s'}
            self.assertEqual(set(origins), {'zig-0.17.1/' + path for path in expected})
            for relative in expected:
                self.assertEqual(origins['zig-0.17.1/' + relative], {
                    'root': 'zig', 'path': relative,
                    'kind': 'symlink' if relative.endswith('alias.h') else 'file'})

    def test_linux_helper_aliases_follow_the_suite(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            suite, top = root / 'suite', root / 'staged'
            (suite / 'lib').mkdir(parents=True)
            (top / 'bin').mkdir(parents=True)
            for name in ['yosys-abc', 'future-helper']:
                (top / 'bin' / name).write_text('#!/bin/sh\nexit 0\n')
                (suite / 'lib' / name).symlink_to('../bin/' + name)
            (suite / 'lib/libalias.so').symlink_to('libactual.so')
            origins = {}
            stage_tool_links(suite, top, origins)
            for name in ['yosys-abc', 'future-helper']:
                self.assertEqual(os.readlink(top / 'lib' / name), '../bin/' + name)
                self.assertEqual((top / 'lib' / name).resolve(), top / 'bin' / name)
                self.assertEqual(origins['lib/' + name], {
                    'root': 'suite', 'path': 'lib/' + name, 'kind': 'symlink'})
            self.assertFalse((top / 'lib/libalias.so').is_symlink())
            stage_tool_links(suite, top) # Repeated staging preserves the same aliases.

    def test_unstaged_helper_target_fails(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            suite, top = root / 'suite', root / 'staged'
            (suite / 'lib').mkdir(parents=True)
            (suite / 'lib/helper').symlink_to('../bin/not-staged')
            with self.assertRaisesRegex(ValueError, 'unstaged wrapper bin/not-staged'):
                stage_tool_links(suite, top)

    def archive(self, root, extra=None, omit=()):
        archive = root / 'examples.tar'
        with tarfile.open(archive, 'w') as stream:
            for name, data in [('showcase.toml', b'showcase = []\n'), ('ulx3s/top.v', b'module top; endmodule\n'),
                               ('ulx3s/README.md', b'ULX3S demos\n'), ('gallery/README.md', b'Gallery demos\n')]:
                if name in omit:
                    continue
                member = tarfile.TarInfo('examples/' + name)
                member.size = len(data)
                stream.addfile(member, io.BytesIO(data))
            link = tarfile.TarInfo('examples/gallery/shared.v')
            link.type = tarfile.SYMTYPE
            link.linkname = '../ulx3s/top.v'
            stream.addfile(link)
            if extra:
                stream.addfile(extra)
        return archive

    def test_archive_override_in_copied_tree_preserves_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            archive = self.archive(root)
            (root / 'untracked-output.bit').write_bytes(b'must not ship')
            manifest = root / 'package/share/examples-manifest.json'
            destination = root / 'package/share/examples'
            with patch.dict(os.environ, ECP5_EXAMPLES_ARCHIVE=str(archive)):
                examples.stage(destination, manifest, root)
            self.assertTrue((destination / 'gallery/shared.v').is_symlink())
            self.assertEqual(os.readlink(destination / 'gallery/shared.v'), '../ulx3s/top.v')
            payload = json.loads(manifest.read_text())
            self.assertEqual({r['path'] for r in payload['files']}, {
                'showcase.toml', 'ulx3s/top.v', 'gallery/shared.v', 'ulx3s/README.md', 'gallery/README.md'})
            self.assertEqual(payload['size']['files'], 4)
            self.assertEqual(payload['size']['symlinks'], 1)
            self.assertEqual(payload['size']['entries'], 5)
            self.assertEqual(payload['size']['file_bytes'], 62)
            self.assertEqual(payload['size']['symlink_bytes'], len('../ulx3s/top.v'))
            self.assertEqual(payload['size']['bytes'], 62 + len('../ulx3s/top.v'))

    def test_examples_archive_requires_both_readme_markers(self):
        for marker in ['ulx3s/README.md', 'gallery/README.md']:
            with self.subTest(marker=marker), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                archive = self.archive(root, omit=[marker])
                with patch.dict(os.environ, ECP5_EXAMPLES_ARCHIVE=str(archive)):
                    with self.assertRaisesRegex(ValueError, 'archive missing examples/' + marker):
                        examples.stage(root / 'package/share/examples', root / 'manifest.json', root)

    def test_examples_archive_cannot_escape(self):
        for target in ['../../../outside', '../..']:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                bad = tarfile.TarInfo('examples/gallery/escape')
                bad.type = tarfile.SYMTYPE
                bad.linkname = target
                archive = self.archive(root, bad)
                with patch.dict(os.environ, ECP5_EXAMPLES_ARCHIVE=str(archive)):
                    with self.assertRaisesRegex(ValueError, 'symlink escapes'):
                        examples.stage(root / 'package/examples', root / 'manifest.json', root)
                self.assertFalse((root / 'package/examples').exists())

    def test_missing_git_and_archive_has_actionable_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            with patch.dict(os.environ):
                os.environ.pop('ECP5_EXAMPLES_ARCHIVE', None)
                with self.assertRaisesRegex(RuntimeError, 'ECP5_EXAMPLES_ARCHIVE'):
                    examples.stage(root / 'package/examples', root / 'manifest.json', root)

    def test_cached_compiler_identity_needs_no_zig_or_home(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            compiler = root / 'compiler'
            compiler.mkdir()
            zig = compiler / 'zig'
            zig.write_text('#!/bin/sh\nprintf "%s\\n" "clang version test" "Target: test" '
                           '"InstalledDir: /build/machine"\n')
            zig.chmod(0o755)
            config = {'zig': {'version': '0.17.1', 'platforms': {'macos': {'target': 'aarch64-macos.15.0.0'}}}}
            with patch.object(toolchain_stage, 'ROOT', root):
                toolchain_stage.clang_wrapper(compiler, config, 'macos')
            zig.unlink()
            result = subprocess.run(
                [str(compiler / 'clang'), '--version'], env={'PATH': '/usr/bin:/bin'},
                text=True, capture_output=True, check=True)
            self.assertEqual(result.stdout, 'clang version test\nTarget: test\n')


if __name__ == '__main__':
    unittest.main()
