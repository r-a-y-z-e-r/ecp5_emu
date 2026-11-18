"""Offline tests for deterministic bytes, lossless splitting and trust boundaries."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'source_archive.py'
TEMP_ROOT = SCRIPT.parents[2] / 'out/source-archive/cache/tests'
TEMP_ROOT.mkdir(parents=True, exist_ok=True)
spec = importlib.util.spec_from_file_location('source_archive', SCRIPT)
archive = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = archive
spec.loader.exec_module(archive)


class ArchiveTests(unittest.TestCase):
    def test_tar_normalizes_metadata_and_preserves_links_modes_and_empty_dirs(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            root = Path(tmp)
            tree = root / 'tree'
            tree.mkdir()
            (tree / 'empty').mkdir()
            (tree / 'run').write_bytes(b'#!/bin/sh\nexit 0\n')
            (tree / 'run').chmod(0o755)
            (tree / 'link').symlink_to('empty')
            (tree / '.git').mkdir()
            (tree / '.git/config').write_text('host-specific')
            first, second = root / 'a.tar.xz', root / 'b.tar.xz'
            archive.write_tar(first, list(archive.tree_entries(tree)))
            os.utime(tree / 'run', (123456, 123456))
            archive.write_tar(second, reversed(list(archive.tree_entries(tree))))
            self.assertEqual(first.read_bytes(), second.read_bytes())
            with tarfile.open(first) as tf:
                self.assertEqual(tf.getnames(), ['empty', 'link', 'run'])
                self.assertTrue(tf.getmember('link').issym())
                self.assertEqual(tf.getmember('link').linkname, 'empty')
                self.assertEqual(tf.getmember('run').mode, 0o755)
                for member in tf:
                    self.assertEqual((member.uid, member.gid, member.mtime, member.uname), (0, 0, 0, ''))

    def test_split_is_deterministic_bounded_and_lossless(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            root = Path(tmp)
            source = root / 'source'
            # Incompressible bytes exercise size limits, not just a highly compressible toy.
            source.write_bytes(hashlib.shake_256(b'source-test').digest(110000))
            infos = {'example': {'revision': 'abc', 'source_sha256': archive.sha256(source),
                                 'files': [{'name': 'source', 'sha256': archive.sha256(source), 'size': 110000}],
                                 'corresponding_source_statement': 'The recorded change is supplied; no further distributor edit is recorded.'}}
            entries = [archive.Entry('items/example/source', source, item='example')]
            first, second = root / 'first', root / 'second'
            first.mkdir()
            second.mkdir()
            parts, mapping = archive.split_parts(first, 'test', entries, infos, limit=40000, readme=b'Replace the separate dynamic library.')
            repeat, _ = archive.split_parts(second, 'test', entries, infos, limit=40000, readme=b'Replace the separate dynamic library.')
            self.assertEqual(parts, repeat)
            self.assertGreater(len(parts), 1)
            self.assertEqual(mapping['example'], [p['file'] for p in parts])
            fragments = {}
            for part in parts:
                self.assertLess(part['size'], 40000)
                with tarfile.open(first / part['file']) as tf:
                    self.assertIn('SOURCES.md', tf.getnames())
                    self.assertEqual(tf.extractfile('README.md').read(), b'Replace the separate dynamic library.')
                    self.assertIn(b'no further distributor edit is recorded', tf.extractfile('SOURCES.md').read())
                    for m in tf:
                        if '.fragment-' in m.name:
                            fragments[m.name] = tf.extractfile(m).read()
            self.assertEqual(b''.join(fragments[n] for n in sorted(fragments)), source.read_bytes())

    def test_single_part_uses_unsuffixed_name(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            info = {'example': {'version': '1', 'source_sha256': '0' * 64, 'files': []}}
            parts, _ = archive.split_parts(Path(tmp), 'test',
                                          [archive.Entry('x', data=b'x', item='example')], info)
            self.assertEqual([p['file'] for p in parts], ['test.tar.xz'])

    def test_dsc_retains_epoch_and_all_listed_hashes(self):
        fields, files = archive.parse_dsc(b'''-----BEGIN PGP SIGNED MESSAGE-----
Hash: SHA256

Source: sample
Version: 2:1.0+dfsg-3ubuntu1
Files:
 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 7 sample.tar.xz
Checksums-Sha1:
 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 7 sample.tar.xz
Checksums-Sha256:
 cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc 7 sample.tar.xz
-----BEGIN PGP SIGNATURE-----
signature
''')
        self.assertEqual(fields['Version'], '2:1.0+dfsg-3ubuntu1')
        self.assertEqual(set(files['sample.tar.xz']), {'size', 'md5', 'sha1', 'sha256'})
        with self.assertRaises(ValueError):
            archive.parse_dsc(b'Source: x\nChecksums-Sha256:\n abc 4 ../escape\n')

    def test_download_rejects_bad_bytes_and_bad_cached_content(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            cache = Path(tmp)
            good = b'published source'
            digest = hashlib.sha256(good).hexdigest()
            with patch.object(archive.urllib.request, 'urlopen', return_value=io.BytesIO(b'bad')):
                with self.assertRaisesRegex(ValueError, 'mismatch'):
                    archive.fetch('https://example.org/source', digest, cache)
            self.assertFalse(list(cache.rglob('*.partial')))
            with patch.object(archive.urllib.request, 'urlopen', return_value=io.BytesIO(good)):
                source = archive.fetch('https://example.org/source', digest, cache)
            source.write_bytes(b'corrupted cache')
            with patch.object(archive.urllib.request, 'urlopen', return_value=io.BytesIO(good)):
                self.assertEqual(archive.fetch('https://example.org/source', digest, cache).read_bytes(), good)

    def test_shared_resource_receipts_are_streamed_once_and_verified(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            root = Path(tmp)
            container = root / 'resource.tar.gz'
            files = []
            with tarfile.open(container, 'w:gz') as tf:
                for n, data in [('a/+PORTFILE', b'port a'), ('b/+CONTENTS', b'receipt b')]:
                    member = tarfile.TarInfo(n)
                    member.size = len(data)
                    tf.addfile(member, io.BytesIO(data))
                    files.append({'container_url': 'https://example.org/resource',
                                  'container_sha256': '0' * 64, 'member': n,
                                  'sha256': hashlib.sha256(data).hexdigest()})
            cache = root / 'cache'
            (cache / 'downloads').mkdir(parents=True)
            with patch.object(archive, 'fetch', return_value=container) as fetch:
                archive.prepare_containers([{'downloads': files}], cache)
                fetch.assert_called_once()
            self.assertEqual((cache / 'downloads' / ('sha256-' + files[0]['sha256'])).read_bytes(), b'port a')
            bad = {**files[0], 'sha256': 'f' * 64}
            with patch.object(archive, 'fetch', return_value=container):
                with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
                    archive.prepare_containers([{'downloads': [bad]}], cache)

    def test_notices_inside_nested_gcc_source_tar_are_retained(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            root = Path(tmp)
            inner = io.BytesIO()
            with tarfile.open(fileobj=inner, mode='w:xz') as tf:
                member = tarfile.TarInfo('gcc/COPYING.RUNTIME')
                member.size = 9
                tf.addfile(member, io.BytesIO(b'exception'))
            outer = root / 'orig.tar.gz'
            with tarfile.open(outer, 'w:gz') as tf:
                member = tarfile.TarInfo('gcc-package/source.tar.xz')
                member.size = len(inner.getvalue())
                tf.addfile(member, io.BytesIO(inner.getvalue()))
            expected = ['orig!gcc-package/source.tar.xz!gcc/COPYING.RUNTIME']
            self.assertEqual(archive.licence_paths(outer, 'orig', root), expected)
            self.assertEqual(archive.licence_paths(outer, 'orig', root), expected)

    def test_initial_inventory_survives_input_cleanup(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            path = Path(tmp) / 'ecp5_emu-1.2.3-linux-x86_64.tar.gz'
            with tarfile.open(path, 'w:gz') as tf:
                member = tarfile.TarInfo('ecp5_emu-1.2.3-linux-x86_64/share/toolchain/VERSION')
                member.size = 4
                tf.addfile(member, io.BytesIO(b'123\n'))
            inventory = archive.inventory_packages({'linux': path})
            path.unlink()
            self.assertEqual(inventory['linux'], ('1.2.3', [
                ('share/toolchain/VERSION', None, hashlib.sha256(b'123\n').hexdigest())]))
            config = {'source_assembly': {'bindings': {}}, 'components': [],
                      'packaging': {'zig': {'version': '0.17.1'}},
                      'suite': {'build_commit': 'a' * 40}}
            version, records, counts, _ = archive.make_manifest(
                config, {'linux': path}, {}, {}, None, 'b' * 40, None, inventory)
            self.assertEqual(version, '1.2.3')
            self.assertEqual(counts['linux']['regular_files'], 1)
            self.assertEqual(records[0]['sha256'], hashlib.sha256(b'123\n').hexdigest())

    def test_source_origins_is_project_generated(self):
        config = {'source_assembly': {'bindings': {}}, 'components': [],
                  'packaging': {'zig': {'version': '0.17.1'}, 'trees': []},
                  'suite': {'build_commit': 'a' * 40}}
        revision = 'b' * 40
        for platform, prefix in [('linux', 'share/toolchain'),
                                 ('macos', 'Contents/Resources/toolchain')]:
            with self.subTest(platform=platform), tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
                package = Path(tmp) / 'ecp5_emu-1.2.3-linux-x86_64'
                toolchain = package / prefix
                toolchain.mkdir(parents=True)
                (toolchain / 'source-origins.json').write_text('{}\n')
                if platform == 'macos':
                    (package / 'Contents/Info.plist').write_bytes(plistlib.dumps({
                        'CFBundleShortVersionString': '1.2.3'}))
                _, records, _, missing = archive.make_manifest(
                    config, {platform: package}, {}, {}, None, revision, None)
                self.assertEqual(len(records), 1)
                self.assertEqual(records[0]['binary_path'], prefix + '/source-origins.json')
                components = records[0]['components']
                self.assertEqual([c['component'] for c in components], ['this-repository'])
                self.assertEqual(components[0]['status'], 'repository-reference')
                self.assertEqual(components[0]['source_revision'], revision)
                self.assertEqual(missing, {})

    def test_nonrequired_provenance_does_not_become_a_source_blocker(self):
        config = {'source_assembly': {'bindings': {}, 'component_obligations': {'linux': {
                  'llvm-ghdl': {'class': 'provenance unknown, not required', 'reason': 'Apache source provenance is not required.'}}}},
                  'packaging': {'zig': {'version': '0.17.1'}, 'trees': []},
                  'components': [{'id': 'llvm-ghdl', 'file_patterns': {'both': ['lib/llvm.so']}, 'license_spdx': 'Apache-2.0 WITH LLVM-exception'}],
                  'suite': {'build_commit': 'a' * 40}}
        inventory = {'linux': ('1.0', [('share/toolchain/lib/llvm.so', None, 'c' * 64)])}
        _, records, _, missing = archive.make_manifest(config, {'linux': Path('gone')}, {}, {}, None, 'b' * 40, None, inventory)
        self.assertEqual(missing, {})
        self.assertEqual(records[0]['components'][0]['status'], 'provenance-unknown-not-required')
        self.assertIsNone(records[0]['components'][0]['source_part'])

    def test_published_sha512_and_recorded_sha256_are_both_verified(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            data = b'upstream release source'
            file = {'name': 'source.txt', 'url': 'https://example.org/source',
                    'sha512': hashlib.sha512(data).hexdigest(), 'sha256': hashlib.sha256(data).hexdigest()}
            item = {'id': 'test', 'kind': 'files', 'downloads': [file]}
            with patch.object(archive.urllib.request, 'urlopen', return_value=io.BytesIO(data)):
                _, info = archive.prepare_item(item, Path(tmp))
            self.assertEqual(info['source_sha256'], file['sha256'])
            self.assertTrue((Path(tmp) / 'downloads' / ('sha512-' + file['sha512'])).exists())
            item['downloads'][0] = {**file, 'sha256': '0' * 64}
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                archive.prepare_item(item, Path(tmp))

    def test_linux_inventory_keeps_symlinks_and_hashes_hardlinks(self):
        with tempfile.TemporaryDirectory(dir=TEMP_ROOT) as tmp:
            path = Path(tmp) / 'ecp5_emu-1.2.3-linux-x86_64.tar.gz'
            with tarfile.open(path, 'w:gz') as tf:
                regular = tarfile.TarInfo('ecp5_emu-1.2.3-linux-x86_64/share/toolchain/lib/tool')
                regular.size = 3
                tf.addfile(regular, io.BytesIO(b'abc'))
                hard = tarfile.TarInfo('ecp5_emu-1.2.3-linux-x86_64/share/toolchain/lib/hard')
                hard.type, hard.linkname = tarfile.LNKTYPE, regular.name
                tf.addfile(hard)
                link = tarfile.TarInfo('ecp5_emu-1.2.3-linux-x86_64/share/toolchain/lib/link')
                link.type, link.linkname = tarfile.SYMTYPE, 'tool'
                tf.addfile(link)
            with archive.package_files(path, 'linux') as (version, objects):
                read = [(n, target, stream.read() if stream else None) for n, target, stream in objects]
            self.assertEqual(version, '1.2.3')
            self.assertEqual(sorted(read), [('share/toolchain/lib/hard', None, b'abc'),
                                    ('share/toolchain/lib/link', 'tool', None),
                                    ('share/toolchain/lib/tool', None, b'abc')])


if __name__ == '__main__':
    unittest.main()
