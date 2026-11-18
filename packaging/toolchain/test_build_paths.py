"""Personal-path guards and per-file upstream provenance regressions."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('build_paths', Path(__file__).parents[1] / 'check_build_paths.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class BuildPathTests(unittest.TestCase):
    def patterns(self, username='runner'):
        return guard.build_patterns(username, '/work/build-home')

    def origin(self, package, relative, root='suite', source=None, kind='file', **extra):
        top = package / 'Contents/Resources/toolchain'
        top.mkdir(parents=True, exist_ok=True)
        (top / 'source-origins.json').write_text(json.dumps({
            'version': 1, 'zig_directory': 'zig-input', 'files': {
                relative: {'root': root, 'path': source or relative, 'kind': kind}}, **extra}))
        return top / relative

    def test_words_and_neutral_checkout_paths_pass(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'program').write_bytes(b'headless runner\x00/private/tmp/ecp5rel\x00/build/ecp5_emu')
            (root / 'alias').symlink_to('program')
            self.assertEqual(guard.find_leaks(root, self.patterns()), [])

    def test_other_users_upstream_paths_need_no_allowance(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = root / 'share/toolchain/lib/ghdl.o'
            path.parent.mkdir(parents=True)
            path.write_bytes(b'/Users/runner/work/ghdl/source.cc\x00')
            self.assertEqual(guard.find_leaks(root, self.patterns('someone-else')), [])

    def test_each_personal_pattern_in_owned_files_fails(self):
        for label, needle in self.patterns().items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as name:
                root = Path(name)
                token = needle.rstrip(b'/') + b'/project/source.cc'
                (root / 'top_routed.json').write_bytes(b'prefix\x00' + token + b'\x00')
                self.assertEqual(guard.find_leaks(root, self.patterns()), [(Path('top_routed.json'), [label])])

    def test_similar_usernames_and_home_prefixes_do_not_match(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'file').write_text('/work/build-home-other/x\n/Users/runner-other/x\n/home/runner2/x')
            self.assertEqual(guard.find_leaks(root, self.patterns()), [])

    def test_root_home_does_not_match_atspi_object_path(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'ecp5_gui').write_bytes(b'/org/a11y/atspi/accessible/root')
            self.assertEqual(guard.find_leaks(root, guard.build_patterns('root', '/root')), [])

    def test_root_home_cargo_path_fails(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'binary').write_bytes(b'\x00/root/.cargo/registry')
            self.assertEqual(guard.find_leaks(root, guard.build_patterns('root', '/root')),
                             [(Path('binary'), ['HOME'])])

    def test_rootfs_does_not_match_root_home(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'binary').write_bytes(b'/rootfs/x')
            self.assertEqual(guard.find_leaks(root, guard.build_patterns('root', '/root')), [])

    def test_leading_boundary_for_every_personal_pattern(self):
        regexes = guard.expressions(self.patterns())
        for needle in self.patterns().values():
            for before in b'Az09._~-/':
                with self.subTest(needle=needle, before=before):
                    data = bytes([before]) + needle + b'/source.cc\x00'
                    self.assertEqual(guard.matches(data, regexes, final=True)[0], set())
            for before in [b'', b'\x00', b'=', b':', b' ', b'"']:
                with self.subTest(needle=needle, before=before):
                    self.assertTrue(guard.matches(before + needle + b'/source.cc\x00', regexes, final=True)[0])

    def test_home_trailing_boundary(self):
        regexes = guard.expressions(guard.build_patterns('root', '/root'))
        for after in b'Az09._~-':
            with self.subTest(after=after):
                self.assertEqual(guard.matches(b'/root' + bytes([after]) + b'/x', regexes, final=True)[0], set())
        for after in [b'', b'/', b':', b'!', b'\x00', b'\xff']:
            with self.subTest(after=after):
                self.assertTrue(guard.matches(b'/root' + after, regexes, final=True)[0])

    def test_identical_suite_path_passes_for_runner(self):
        with tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as input_name:
            root, suite = Path(name), Path(input_name)
            path = self.origin(root, 'lib/ghdl.o')
            path.parent.mkdir(parents=True)
            data = b'\x00/Users/runner/work/ghdl/source.cc\x00'
            path.write_bytes(b'relocated binary header\x00' + data)
            (suite / 'lib').mkdir()
            (suite / 'lib/ghdl.o').write_bytes(data)
            self.assertEqual(guard.find_leaks(root, self.patterns(), suite_root=suite), [])
            # Sharing /Users/runner/ is insufficient: each complete path must match.
            path.write_bytes(data + b'/Users/runner/new/private.cc\x00')
            self.assertEqual(guard.find_leaks(root, self.patterns(), suite_root=suite),
                             [(path.relative_to(root), ['macOS user path'])])

    def test_generated_files_never_inherit_source_allowances(self):
        names = ['bin/yosys', 'environment.sh', 'bundle-manifest.json', 'components.json',
                 'source-origins.json', 'zig-0.17.1/clang', 'zig-0.17.1/clang-version.txt']
        for relative in names:
            with self.subTest(relative=relative), tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as original:
                root, suite = Path(name), Path(original)
                path = self.origin(root, relative, source='source-file')
                data = b'/Users/runner/work/ghdl/source.cc\x00'
                (suite / 'source-file').write_bytes(data)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                self.assertEqual(guard.find_leaks(root, self.patterns(), suite_root=suite),
                                 [(path.relative_to(root), ['macOS user path'])])

    def test_zig_origin_and_default_download_location(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            package = root / 'package'
            zig = root / 'target/packaging/zig-input'
            (zig / 'lib').mkdir(parents=True)
            data = b'/home/runner/upstream/input.zig\n'
            (zig / 'lib/std.zig').write_bytes(data)
            path = self.origin(package, 'zig-0.17.1/lib/std.zig', root='zig', source='lib/std.zig')
            path.parent.mkdir(parents=True)
            path.write_bytes(data)
            self.assertEqual(guard.find_leaks(package, self.patterns(), zig_root=zig), [])
            self.assertEqual(guard.find_leaks(package, self.patterns(), build_root=root), [])

    def test_absent_origin_or_source_fails_closed(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            path = self.origin(root, 'lib/ghdl.o')
            path.parent.mkdir(parents=True)
            path.write_text('/Users/runner/work/ghdl/source.cc')
            self.assertTrue(guard.find_leaks(root, self.patterns()))
            (root / 'Contents/Resources/toolchain/source-origins.json').unlink()
            self.assertTrue(guard.find_leaks(root, self.patterns()))

    def test_outside_toolchain_has_no_allowance(self):
        with tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as original:
            root = Path(name)
            path = root / 'top_routed.json'
            path.write_text('/Users/runner/work/ghdl/source.cc')
            (Path(original) / 'top_routed.json').write_bytes(path.read_bytes())
            self.assertTrue(guard.find_leaks(root, self.patterns(), suite_root=original))

    def test_link_targets_checked_without_following_them(self):
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            (root / 'broken').symlink_to('/work/build-home/absent')
            (root / 'directory-link').symlink_to('/home/runner/folder', target_is_directory=True)
            self.assertEqual(guard.find_leaks(root, self.patterns()),
                             [(Path('broken'), ['HOME']), (Path('directory-link'), ['Linux user path'])])

    def test_source_symlink_compares_literal_target(self):
        with tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as original:
            root, suite = Path(name), Path(original)
            path = self.origin(root, 'lib/alias', kind='symlink')
            path.parent.mkdir(parents=True)
            (suite / 'lib').mkdir()
            target = '/home/runner/unavailable'
            path.symlink_to(target)
            (suite / 'lib/alias').symlink_to(target)
            self.assertEqual(guard.find_leaks(root, self.patterns(), suite_root=suite), [])

    def test_external_directory_links_are_not_traversed(self):
        with tempfile.TemporaryDirectory() as name, tempfile.TemporaryDirectory() as external:
            root = Path(name)
            (Path(external) / 'not-packaged').write_text('/work/build-home/private')
            (root / 'external').symlink_to(external, target_is_directory=True)
            self.assertEqual(guard.find_leaks(root, self.patterns()), [])

    def test_complete_paths_across_read_boundaries(self):
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / 'binary'
            token = b'/work/build-home/path with spaces/source.cc'
            path.write_bytes(b'x' * 29 + b'\x00' + token + b'\x00')
            self.assertEqual(guard.file_matches(path, self.patterns(), chunk_size=16), {('HOME', token)})

    def test_streamed_scans_preserve_path_boundary_context(self):
        patterns = guard.build_patterns('root', '/root')
        cases = [(b'/org/a11y/atspi/accessible/root', set()),
                 (b'\x00/root/.cargo/registry', {('HOME', b'/root/.cargo/registry')}),
                 (b'/rootfs/x', set()),
                 (b'/root/x', {('HOME', b'/root/x')})]
        with tempfile.TemporaryDirectory() as name:
            path = Path(name) / 'binary'
            for data, expected in cases:
                for padding in range(20):
                    path.write_bytes(b'x' * padding + b'\x00' + data + b'\x00' + b'y' * 35)
                    for chunk_size in [1, 2, 4, 8, 16, 32]:
                        with self.subTest(data=data, padding=padding, chunk_size=chunk_size):
                            self.assertEqual(guard.file_matches(path, patterns, chunk_size=chunk_size), expected)

    def test_empty_home_fails_closed(self):
        with self.assertRaises(ValueError):
            guard.build_patterns('builder', '')


if __name__ == '__main__':
    unittest.main()
