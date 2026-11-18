#!/usr/bin/env python3
"""Assemble verified corresponding-source items and inventory final package bytes.

Python 3 standard library; git is required for commit-pinned source trees. An
incomplete result is useful for auditing, but is explicitly NOT release clearance.
See source_assembly in components.json and the bundled-toolchain licence audit.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
from contextlib import contextmanager, nullcontext
from dataclasses import dataclass, replace
import fnmatch
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import re
import shutil
import stat
import subprocess
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CONFIG = Path(__file__).with_name('components.json')
LIMIT = 2 * 1024**3  # Every asset must be strictly smaller.
CACHE_LIMIT = 8 * 1024**3
CHUNK = 1024 * 1024
GROUPS = ('suite', 'ubuntu-runtime', 'macos-runtime')


def canonical(value):
    return (json.dumps(value, sort_keys=True, ensure_ascii=True, separators=(',', ':')) + '\n').encode()


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def safe_name(name):
    p = PurePosixPath(name)
    if not name or p.is_absolute() or '..' in p.parts or '\\' in name or '\x00' in name:
        raise ValueError('unsafe archive path')
    return p.as_posix()


def sanitized_error(exc):
    # Subprocess diagnostics can contain temporary checkout paths.
    text = str(exc).replace(str(ROOT), '<checkout>')
    return re.sub(r'/(?:Users|home)/[^\s\"\']+', '<local-path>', text)[:1600]


@dataclass(frozen=True)
class Entry:
    name: str
    path: Path | None = None
    data: bytes | None = None
    mode: int = 0o644
    target: str | None = None
    directory: bool = False
    offset: int = 0
    length: int | None = None
    item: str | None = None

    @property
    def size(self):
        if self.target is not None or self.directory:
            return 0
        if self.length is not None:
            return self.length
        return self.path.stat().st_size if self.path else len(self.data or b'')


def write_tar(destination, entries):
    """Stable PAX tar + XZ (no host metadata, wall clock, or directory traversal)."""
    with tarfile.open(destination, 'w:xz', format=tarfile.PAX_FORMAT, preset=6) as tf:
        seen = set()
        for entry in sorted(entries, key=lambda e: e.name):
            name = safe_name(entry.name)
            if name in seen:
                raise ValueError('duplicate archive member: ' + name)
            seen.add(name)
            info = tarfile.TarInfo(name)
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ''
            info.mode = entry.mode
            if entry.target is not None:
                info.type = tarfile.SYMTYPE
                info.linkname = entry.target
                info.mode = 0o777
                tf.addfile(info)
            elif entry.directory:
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                tf.addfile(info)
            else:
                info.size = entry.size
                if entry.path:
                    with entry.path.open('rb') as stream:
                        stream.seek(entry.offset)
                        tf.addfile(info, stream)
                else:
                    tf.addfile(info, io.BytesIO(entry.data or b''))


def tree_entries(root):
    """Do not follow directory symlinks, include empty directories, omit git DBs."""
    root = Path(root)
    for current, dirs, files in os.walk(root, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d != '.git')
        for name in sorted(dirs + files):
            if name == '.git':
                continue
            path = Path(current) / name
            rel = path.relative_to(root).as_posix()
            st = path.lstat()
            if stat.S_ISLNK(st.st_mode):
                yield Entry(rel, target=os.readlink(path))
            elif stat.S_ISDIR(st.st_mode):
                yield Entry(rel, directory=True)
            elif stat.S_ISREG(st.st_mode):
                yield Entry(rel, path, mode=0o755 if st.st_mode & 0o111 else 0o644)
            else:
                raise ValueError('unsupported source object: ' + rel)


def cache_size(cache):
    return sum(p.stat().st_size for p in cache.rglob('*') if p.is_file() and not p.is_symlink())


def check_cache(cache, incoming=0):
    if cache_size(cache) + incoming >= CACHE_LIMIT:
        raise ValueError('8 GiB cache budget exceeded; remove expendable evidence/downloads first')


def fetch(url, digest, cache, algorithm='sha256', size=None):
    """Only checksum-verified downloads enter the reusable source cache."""
    if algorithm not in ('sha256', 'sha512') or not re.fullmatch(r'[0-9a-f]+', digest or ''):
        raise ValueError('missing published SHA-256/SHA-512 for ' + url)
    if len(digest) != hashlib.new(algorithm).digest_size * 2:
        raise ValueError('invalid checksum for ' + url)
    dest = cache / 'downloads' / (algorithm + '-' + digest)
    dest.parent.mkdir(parents=True, exist_ok=True)

    def valid():
        if not dest.is_file() or (size is not None and dest.stat().st_size != size):
            return False
        with dest.open('rb') as src:
            return hashlib.file_digest(src, algorithm).hexdigest() == digest

    if valid():
        return dest
    dest.unlink(missing_ok=True)
    partial = dest.with_suffix('.partial')
    check_cache(cache, size or 0)
    try:
        for attempt in range(3):
            try:
                with urllib.request.urlopen(urllib.request.Request(url, headers={
                        'User-Agent': 'ecp5-emu-source-archive/1'}), timeout=90) as src, partial.open('wb') as out:
                    h = hashlib.new(algorithm)
                    count = 0
                    while block := src.read(CHUNK):
                        out.write(block)
                        h.update(block)
                        count += len(block)
                        if count % (64 * CHUNK) < CHUNK:
                            check_cache(cache)
                    if h.hexdigest() != digest or (size is not None and count != size):
                        raise ValueError('checksum/size mismatch: ' + url)
                partial.replace(dest)
                break
            except (urllib.error.URLError, TimeoutError, ConnectionError):
                partial.unlink(missing_ok=True)
                if attempt == 2:
                    raise
                time.sleep(attempt + 1)
    finally:
        partial.unlink(missing_ok=True)
    return dest


def git_command(args, cwd):
    env = {**os.environ, 'GIT_CONFIG_NOSYSTEM': '1', 'GIT_CONFIG_GLOBAL': os.devnull,
           'GIT_TERMINAL_PROMPT': '0', 'LC_ALL': 'C'}
    result = subprocess.run(['git', '-c', 'core.autocrlf=false', '-c', 'core.hooksPath=/dev/null',
                             *args], cwd=cwd, env=env, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True)
    if result.returncode:
        raise ValueError('git ' + args[0] + ': ' + sanitized_error(result.stderr))
    return result.stdout.rstrip()


def normalize_submodules(records):
    # Discard git-describe labels: they depend on whichever mutable refs a fetch saw.
    result = []
    for record in records:
        if isinstance(record, dict):
            result.append(record)
        elif record:
            revision, path = record.strip().split(' ', 1)
            result.append({'revision': revision, 'path': path.split(' (', 1)[0]})
    return sorted(result, key=lambda r: r['path'])


def git_source(item, cache):
    revision = item['revision']
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('git source must pin a full commit, not a branch/tag')
    key = hashlib.sha256(canonical([item['repository'], revision])).hexdigest()
    dest = cache / 'git-items' / (key + '.tar.xz')
    metadata = dest.with_suffix('.json')
    dest.parent.mkdir(parents=True, exist_ok=True)
    if dest.is_file() and metadata.is_file():
        info = json.loads(metadata.read_text())
        if info.get('sha256') == sha256(dest):
            return dest, normalize_submodules(info['submodules'])
    with tempfile.TemporaryDirectory(prefix='git-', dir=cache) as tmp:
        work = Path(tmp)
        git_command(['init', '-q'], work)
        git_command(['remote', 'add', 'origin', item['repository']], work)
        git_command(['fetch', '--depth=1', 'origin', revision], work)
        if git_command(['rev-parse', 'FETCH_HEAD^{commit}'], work) != revision:
            raise ValueError('git commit mismatch')
        git_command(['checkout', '-q', '--detach', revision], work)
        git_command(['submodule', 'update', '--init', '--recursive', '--depth=1'], work)
        submodules = git_command(['submodule', 'status', '--recursive'], work).splitlines()
        if any(line and line[0] != ' ' for line in submodules):
            raise ValueError('missing or mismatched submodule checkout')
        git_command(['fsck', '--full', '--no-reflogs'], work)
        # Check each nested object database as well as the superproject.
        git_command(['submodule', 'foreach', '--recursive', 'git fsck --full --no-reflogs'], work)
        check_cache(cache)
        temporary = dest.with_suffix('.partial')
        try:
            write_tar(temporary, tree_entries(work))
            check_cache(cache)
            temporary.replace(dest)
        finally:
            temporary.unlink(missing_ok=True)
    info = {'sha256': sha256(dest), 'submodules': normalize_submodules(submodules)}
    metadata.write_bytes(canonical(info))
    return dest, info['submodules']


def parse_dsc(data):
    """Parse clear-signed Debian control fields; prefer strong, listed hashes."""
    text = data.decode('utf-8')
    if text.startswith('-----BEGIN PGP SIGNED MESSAGE-----'):
        text = text.split('\n\n', 1)[1].split('-----BEGIN PGP SIGNATURE-----', 1)[0]
        text = '\n'.join(line[2:] if line.startswith('- ') else line for line in text.splitlines())
    fields = {}
    field = None
    for line in text.splitlines():
        if line.startswith((' ', '\t')) and field:
            fields[field] += '\n' + line.strip()
        elif ':' in line:
            field, value = line.split(':', 1)
            fields[field] = value.strip()
        elif not line:
            field = None
    files = {}
    for label, alg in [('Checksums-Sha256', 'sha256'), ('Checksums-Sha512', 'sha512')]:
        for line in fields.get(label, '').splitlines():
            if not line.strip():
                continue
            digest, size, name = line.split()
            safe_name(name)
            if '/' in name:
                raise ValueError('DSC filename is not a basename')
            files.setdefault(name, {})[alg] = digest
            files[name]['size'] = int(size)
    if not files:
        raise ValueError('DSC has no strong checksums')
    # Also check all weaker hashes listed by the maintainer; never use them as the trust anchor.
    for label, alg in [('Files', 'md5'), ('Checksums-Sha1', 'sha1')]:
        for line in fields.get(label, '').splitlines():
            if not line.strip():
                continue
            digest, size, name = line.split()
            if name not in files or int(size) != files[name]['size']:
                raise ValueError('inconsistent DSC file lists')
            files[name][alg] = digest
    return fields, files


def licence_paths(path, prefix, cache=None):
    inventory = None
    if cache:
        key = sha256(path)
        inventory = cache / 'notice-inventories' / (key + '.json')
        if inventory.is_file():
            saved = json.loads(inventory.read_text())
            if saved.get('sha256') == hashlib.sha256(canonical(saved['paths'])).hexdigest():
                return [prefix + '!' + n for n in saved['paths']]
    found = []

    def scan(stream, nested_prefix='', depth=0):
        try:
            with tarfile.open(fileobj=stream, mode='r|*') as tf:
                for member in tf:
                    parts = PurePosixPath(member.name).parts
                    base = PurePosixPath(member.name).name.lower()
                    if not member.isfile():
                        continue
                    name = nested_prefix + member.name.removeprefix('./')
                    if (base.startswith(('copying', 'copyright', 'licence', 'license'))
                            or any(p.lower() in ('licenses', 'licences') for p in parts[:-1])):
                        found.append(name)
                    # Ubuntu GCC orig payloads contain a second source tar. Avoid
                    # recursively scanning deeply nested test-fixture archives.
                    if (depth < 2 and len(parts) <= 2
                            and base.endswith(('.tar', '.tar.gz', '.tar.xz', '.tar.bz2'))):
                        scan(tf.extractfile(member), name + '!', depth + 1)
        except tarfile.ReadError:
            pass  # Detached signatures, DSCs, diffs or intentional test fixtures.

    with path.open('rb') as stream:
        scan(stream)
    found = sorted(set(found))
    if inventory:
        inventory.parent.mkdir(parents=True, exist_ok=True)
        inventory.write_bytes(canonical({'paths': found, 'sha256': hashlib.sha256(canonical(found)).hexdigest()}))
    return [prefix + '!' + n for n in found]


def prepare_containers(items, cache):
    # A resource tar is large: verify it once, stream every requested receipt once.
    groups = defaultdict(dict)
    for item in items:
        for file in item.get('downloads', []):
            if 'container_url' not in file:
                continue
            path = cache / 'downloads' / ('sha256-' + file['sha256'])
            if path.is_file() and sha256(path) == file['sha256']:
                continue
            key = (file['container_url'], file['container_sha256'])
            groups[key][safe_name(file['member'])] = file
    for (url, digest), wanted in sorted(groups.items()):
        container = fetch(url, digest, cache)
        with tarfile.open(container, 'r|*') as tf:
            for member in tf:
                file = wanted.get(member.name)
                if not file:
                    continue
                if not member.isfile():
                    raise ValueError('resource receipt is not a regular file')
                data = tf.extractfile(member).read()
                if hashlib.sha256(data).hexdigest() != file['sha256']:
                    raise ValueError('resource receipt checksum mismatch')
                (cache / 'downloads' / ('sha256-' + file['sha256'])).write_bytes(data)
                del wanted[member.name]
                if not wanted:
                    break
        if wanted:
            raise ValueError('missing requested resource receipts')


def prepare_item(item, cache):
    entries = []
    info = {k: v for k, v in item.items() if k not in ('patch_inputs',)}
    prefix = 'items/' + safe_name(item['id']) + '/'
    downloaded = []
    if item['kind'] == 'git':
        path, submodules = git_source(item, cache)
        entries.append(Entry(prefix + 'source.tar.xz', path, item=item['id']))
        info.update(source_sha256=sha256(path), submodules=submodules)
        downloaded.append(('source.tar.xz', path))
    elif item['kind'] in ('tarball', 'files'):
        for file in item['downloads']:
            name = safe_name(file.get('name') or urllib.parse.unquote(file['url'].rsplit('/', 1)[1]))
            if 'text' in file:
                data = file['text'].encode()
                if hashlib.sha256(data).hexdigest() != file['sha256']:
                    raise ValueError('inline input checksum mismatch')
                path = cache / 'downloads' / ('sha256-' + file['sha256'])
                path.parent.mkdir(parents=True, exist_ok=True)
                if not path.exists() or sha256(path) != file['sha256']:
                    path.write_bytes(data)
            elif 'container_url' in file:
                path = cache / 'downloads' / ('sha256-' + file['sha256'])
                if not path.exists() or sha256(path) != file['sha256']:
                    prepare_containers([item], cache)
                if not path.exists() or sha256(path) != file['sha256']:
                    raise ValueError('resource receipt not recovered')
            else:
                algorithm = 'sha512' if file.get('sha512') else 'sha256'
                path = fetch(file['url'], file[algorithm], cache, algorithm, file.get('size'))
                if sha256(path) != file['sha256']:
                    raise ValueError('source SHA-256 mismatch')
            entries.append(Entry(prefix + name, path, item=item['id']))
            downloaded.append((name, path))
        info['source_sha256'] = (sha256(downloaded[0][1]) if len(downloaded) == 1 else
                                 hashlib.sha256(canonical([(n, sha256(p)) for n, p in downloaded])).hexdigest())
    elif item['kind'] == 'dsc':
        dsc = fetch(item['url'], item['sha256'], cache)
        fields, files = parse_dsc(dsc.read_bytes())
        if fields.get('Source') != item['package'] or fields.get('Version') != item['version']:
            raise ValueError('DSC source/version differs from pinned identity')
        name = urllib.parse.unquote(item['url'].rsplit('/', 1)[1])
        entries.append(Entry(prefix + name, dsc, item=item['id']))
        downloaded.append((name, dsc))
        for name, hashes in sorted(files.items()):
            url = item.get('file_urls', {}).get(name) or urllib.parse.urljoin(item['url'], name)
            alg = 'sha256' if 'sha256' in hashes else 'sha512'
            path = fetch(url, hashes[alg], cache, alg, hashes['size'])
            for algorithm, expected in hashes.items():
                if algorithm == 'size':
                    continue
                with path.open('rb') as stream:
                    if hashlib.file_digest(stream, algorithm).hexdigest() != expected:
                        raise ValueError('DSC listed hash mismatch: ' + name)
            entries.append(Entry(prefix + name, path, item=item['id']))
            downloaded.append((name, path))
        info['dsc_checksums'] = files
        info['source_sha256'] = hashlib.sha256(canonical([(n, sha256(p)) for n, p in downloaded])).hexdigest()
    else:
        raise ValueError('unsupported source kind')
    info['files'] = [{'name': n, 'sha256': sha256(p), 'size': p.stat().st_size} for n, p in downloaded]
    info['licence_files'] = sorted({licence for n, p in downloaded
                                   for licence in licence_paths(p, prefix + n, cache)})
    entries.append(Entry(prefix + 'METADATA.json', data=canonical(info), item=item['id']))
    return entries, info


def patch_inventory(recipe, item):
    # The entire pinned recipe is included. Hash every build/patch input, not guessed subsets.
    paths = ('src/', 'default/', 'scripts/', 'docker/', '.github/')
    with tarfile.open(recipe) as tf:
        records = []
        for member in tf:
            if member.isfile() and member.name.startswith(paths):
                records.append({'path': member.name, 'sha256': hashlib.sha256(tf.extractfile(member).read()).hexdigest()})
    return {'recipe_revision': item['revision'], 'files': sorted(records, key=lambda r: r['path'])}


def sources_markdown(infos, item_ids, fragment_records):
    lines = ['# Corresponding-source items', '',
             'Source delivery uses exact sources where known and documented closest sources otherwise.',
             'See the release report for copyleft classifications and nonblocking provenance notes.',
             'Source archives retain all nested notices. `!` in a notice path enters a nested tar.',
             'For split source files, concatenate fragments by their numeric suffix; verify the original SHA-256.', '']
    for item_id in sorted(item_ids):
        info = infos[item_id]
        lines += ['## ' + item_id, '',
                  'Version/revision: ' + str(info.get('version') or info.get('revision')), '',
                  'Source SHA-256: `' + info['source_sha256'] + '`', '',
                  'Patch/build inventory SHA-256: `' + str(info.get('patch_set_sha256')) + '`', '']
        if info.get('corresponding_source_statement'):
            lines += [info['corresponding_source_statement'], '']
        if info.get('source_note'):
            lines += [info['source_note'], '']
        for key in ('url', 'repository', 'evidence_url'):
            if info.get(key):
                lines += [key + ': ' + info[key], '']
        for record in info['files']:
            lines += ['- ' + record['name'] + ': SHA-256 `' + record['sha256'] + '` (' + str(record['size']) + ' bytes)']
        for download in info.get('downloads', []):
            if download.get('url') or download.get('container_url'):
                lines += ['- URL: ' + download.get('url', download.get('container_url'))]
            if download.get('sha512'):
                lines += ['- Published SHA-512: `' + download['sha512'] + '`']
        lines += ['']
    if fragment_records:
        lines += ['## File fragments', '', '```json', json.dumps(fragment_records, sort_keys=True, indent=2), '```', '']
    return ('\n'.join(lines) + '\n').encode()


def split_parts(out, stem, entries, infos, limit=LIMIT, readme=b''):
    """Conservative uncompressed budgeting, then enforce the actual compressed limit.

    A source file larger than the budget is fragmented without dropping bytes. Each
    part remains a standalone tar.xz, with reassembly metadata in SOURCES.md.
    """
    if limit < 16384:
        raise ValueError('part limit must be at least 16 KiB')
    fragments = []
    expanded = []
    payload_budget = max(1024, (limit - 12288) // 2)
    for entry in sorted(entries, key=lambda e: e.name):
        if entry.size > payload_budget and entry.path:
            count = (entry.size + payload_budget - 1) // payload_budget
            for i in range(count):
                size = min(payload_budget, entry.size - i * payload_budget)
                fragment = replace(entry, name=entry.name + '.fragment-' + str(i + 1).zfill(5),
                                   offset=entry.offset + i * payload_budget, length=size)
                expanded.append(fragment)
                fragments.append({'file': entry.name, 'fragment': fragment.name,
                                  'offset': i * payload_budget, 'size': size,
                                  'sha256': sha256(entry.path)})
        else:
            expanded.append(entry)

    def doc(batch):
        names = {e.name for e in batch}
        return sources_markdown(infos, {e.item for e in batch if e.item},
                                [f for f in fragments if f['fragment'] in names])

    def budget(batch):
        # PAX extended headers, tar block padding + end record, XZ worst-case expansion.
        raw = 10240 + 2048 + ((len(doc(batch)) + 511) // 512) * 512
        if readme:
            raw += 2048 + ((len(readme) + 511) // 512) * 512
        raw += sum(2048 + ((e.size + 511) // 512) * 512 for e in batch)
        return raw + raw // 1000 + 1024

    batches = []
    batch = []
    for entry in expanded:
        if batch and budget(batch + [entry]) >= limit:
            batches.append(batch)
            batch = []
        batch.append(entry)
        if budget(batch) >= limit:
            raise ValueError('part metadata exceeds size limit')
    if batch:
        batches.append(batch)
    result, item_parts = [], defaultdict(list)
    for i, batch in enumerate(batches, 1):
        name = stem + (('-' + str(i)) if len(batches) > 1 else '') + '.tar.xz'
        path = out / name
        temporary = path.with_suffix('.partial')
        try:
            extra = [Entry('README.md', data=readme)] if readme else []
            write_tar(temporary, [*batch, *extra, Entry('SOURCES.md', data=doc(batch))])
            if temporary.stat().st_size >= limit:
                raise ValueError('compressed part exceeds limit')
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
        result.append({'file': name, 'size': path.stat().st_size, 'sha256': sha256(path)})
        for item_id in sorted({e.item for e in batch if e.item}):
            item_parts[item_id].append(name)
    return result, dict(item_parts)


@contextmanager
def package_files(path, platform):
    """Yield (version, iterable of final toolchain objects), without unpacking Linux."""
    path = path.resolve(strict=True)
    if platform == 'macos':
        root = path / 'Contents/Resources/toolchain'
        with (path / 'Contents/Info.plist').open('rb') as stream:
            version = plistlib.load(stream)['CFBundleShortVersionString']
        if not root.is_dir():
            raise ValueError('app has no staged toolchain')
        def walk():
            for entry in tree_entries(root):
                if not entry.directory:
                    yield 'Contents/Resources/toolchain/' + entry.name, entry.target, entry.path
        yield version, walk()
    elif path.is_dir():
        root = path / 'share/toolchain'
        if not root.is_dir():
            raise ValueError('Linux root has no share/toolchain')
        version = linux_version(path.name)
        def walk():
            for entry in tree_entries(root):
                if not entry.directory:
                    yield 'share/toolchain/' + entry.name, entry.target, entry.path
        yield version, walk()
    else:
        # Random access allows hardlinks to be hashed as regular shipped files.
        with tarfile.open(path) as tf:
            roots = {PurePosixPath(m.name).parts[0] for m in tf.getmembers() if m.name not in ('.', './')}
            if len(roots) != 1:
                raise ValueError('Linux tarball must have one package root')
            root = roots.pop()
            version = linux_version(root)
            prefix = root + '/share/toolchain/'
            def walk():
                # Keep compressed input reads in archive order. Records are sorted
                # only after hashing, so gzip never rewinds for each pathname.
                for member in tf.getmembers():
                    normalized = safe_name(member.name)
                    if not normalized.startswith(prefix) or member.isdir():
                        continue
                    name = normalized[len(root) + 1:]
                    if member.issym():
                        yield name, member.linkname, None
                    elif member.isfile() or member.islnk():
                        with tf.extractfile(member) as stream:
                            yield name, None, stream
                    else:
                        raise ValueError('unsupported package object: ' + name)
            yield version, walk()


def input_version(inputs, override=None):
    versions = set()
    for platform, path in inputs.items():
        path = path.resolve(strict=True)
        if platform == 'macos':
            with (path / 'Contents/Info.plist').open('rb') as stream:
                version = plistlib.load(stream)['CFBundleShortVersionString']
        else:
            version = linux_version(path.name)
            if not version and path.is_file():
                with tarfile.open(path, 'r|*') as tf:
                    for member in tf:
                        parts = PurePosixPath(member.name).parts
                        if parts and parts[0] != '.':
                            version = linux_version(parts[0])
                            break
        if version:
            versions.add(version)
    if len(versions) > 1 or (override and versions and versions != {override}):
        raise ValueError('package version mismatch')
    version = override or (next(iter(versions)) if versions else None)
    if not version or not re.fullmatch(r'[A-Za-z0-9.+_-]+', version):
        raise ValueError('cannot infer package version; use --version')
    return version


def linux_version(name):
    match = re.fullmatch(r'ecp5_emu-(.+)-linux-x86_64(?:\.tar\.gz)?', name)
    return match.group(1) if match else None


def owners(config, platform, rel):
    # stage.py writes source-origins.json into the toolchain root.
    generated = {'environment.sh', 'components.json', 'VERSION', 'bundle-manifest.json', 'source-origins.json'}
    if (rel in generated or rel.startswith('bin/') or rel in {'zig-' + config['packaging']['zig']['version'] + '/clang',
                                    'zig-' + config['packaging']['zig']['version'] + '/clang-version.txt'}
            or '/_CodeSignature/' in rel):
        return ['this-repository']
    found = set()
    for c in config['components']:
        if c['id'] in ('dlfcn-win32', 'rust', 'apple-system', 'suite-wrappers'):
            continue
        files = c.get('files', {})
        patterns = c.get('file_patterns', {})
        for key in (platform, 'both'):
            if rel in files.get(key, []):
                found.add(c['id'])
            for pattern in patterns.get(key, []):
                embedded = re.findall(r'(?:embedded in|potentially) ([A-Za-z0-9_./+-]+)', pattern)
                if rel in embedded or rel == pattern.rstrip('/**') or fnmatch.fnmatchcase(rel, pattern):
                    found.add(c['id'])
    # Root trees include additions that were absent from the historical closure.
    for tree in config['packaging']['trees']:
        if rel == tree['path'] or rel.startswith(tree['path'] + '/'):
            found.add(tree['component'])
    if rel == 'share/trellis/database':
        found.add('trellis-database')
    if rel == 'lib/yosys-abc':
        found.add('abc')
    if rel.startswith('lib/python3.11/') or rel.startswith('libexec/python3'):
        found.add('python')
    if rel.startswith('Frameworks/Qt'):
        found.add('qt')
    if rel.startswith('license/LICENSE.'):
        report = rel.split('LICENSE.', 1)[1]
        report = {'python3': 'python'}.get(report, report)
        found.add(report if any(c['id'] == report for c in config['components']) else 'suite-reports')
    zigprefix = 'zig-' + config['packaging']['zig']['version'] + '/'
    if rel.startswith(zigprefix):
        found.add('zig')
        sub = rel[len(zigprefix):]
        if sub == 'zig':
            found.add('zig-llvm')
        if sub.startswith('lib/libc/'):
            if '/glibc/' in sub or '/generic-glibc/' in sub or '/x86-linux-gnu/' in sub:
                found.add('zig-libc-glibc')
            elif '/darwin/' in sub:
                found.add('zig-libc-darwin')
            elif '/any-darwin-any/' in sub:
                found.add('zig-libc-darwin-headers')
            elif '/linux/' in sub or '-linux-any/' in sub:
                found.add('zig-libc-linux-uapi')
    return sorted(found) or ['unidentified']


def inventory_packages(inputs):
    """Hash input bytes before downloads/compression; callers may remove inputs later."""
    inventories = {}
    for platform, path in sorted(inputs.items()):
        records = []
        with package_files(path, platform) as (version, files):
            for name, target, source in files:
                digest = None
                if target is None:
                    digest = sha256(source) if isinstance(source, Path) else hashlib.file_digest(source, 'sha256').hexdigest()
                records.append((name, target, digest))
        inventories[platform] = (version, records)
    return inventories


def make_manifest(config, inputs, infos, item_parts, patch_hash, repository_revision, version_override, inventories=None):
    records, counts, versions = [], {}, set()
    binding = config['source_assembly']['bindings']
    missing = defaultdict(set)
    licence_terms = {c['id']: set(re.findall(r'[A-Za-z0-9][A-Za-z0-9.+-]+', c.get('license_spdx', '')))
                     for c in config['components']}
    for platform, path in sorted(inputs.items()):
        count = Counter()
        context = nullcontext(inventories[platform]) if inventories is not None else package_files(path, platform)
        with context as (version, files):
            if version:
                versions.add(version)
            for name, target, source in files:
                rel = name.split('/toolchain/', 1)[1]
                components = []
                licences = set()
                for component in owners(config, platform, rel):
                    if component == 'this-repository':
                        licences.add('repository-at-release-commit/License:LICENSE@' + repository_revision)
                        components.append({'component': component, 'source_item': 'this-repository-at-release-commit',
                                           'source_revision': repository_revision, 'source_part': None,
                                           'source_sha256': None, 'patch_set_sha256': None,
                                           'status': 'repository-reference',
                                           'source_note': 'Generated by this repository at the release commit; repository is not archived.'})
                        continue
                    item_ids = binding.get(platform, {}).get(component, [])
                    if not item_ids:
                        item_ids = [None]
                    for item_id in item_ids:
                        info = infos.get(item_id)
                        parts = item_parts.get(item_id, [])
                        status = 'archived' if info and parts else 'unresolved'
                        record = {'component': component, 'source_item': item_id,
                                  'source_revision': (info.get('revision') or info.get('version')) if info else None,
                                  'source_version': info.get('version') if info else None,
                                  'source_sha256': info.get('source_sha256') if info else None,
                                  'patch_set_sha256': info.get('patch_set_sha256') if info else None,
                                  'source_part': parts[0] if parts else None, 'source_parts': parts,
                                  'status': status}
                        if info and info.get('port_directory_tree_sha1'):
                            record['source_revision_scope'] = 'Matching historical port directory; original global ports-tree is unknown.'
                        if info and info.get('source_note', '').startswith('Evidence only:'):
                            record['status'] = 'evidence-only'
                        gaps = config['source_assembly'].get('correspondence_gaps', {}).get(platform, {}).get(component, [])
                        if gaps:
                            record['correspondence_status'] = 'unresolved'
                            record['unresolved'] = gaps
                        policy = config['source_assembly'].get('component_obligations', {}).get(platform, {}).get(component)
                        if policy:
                            if status == 'unresolved' and policy['class'] == 'provenance unknown, not required':
                                record['status'] = 'provenance-unknown-not-required'
                            record['obligation_class'] = policy['class']
                            record['obligation_reason'] = policy['reason']
                        if info and info.get('corresponding_source_statement'):
                            record['source_statement'] = info['corresponding_source_statement']
                        components.append(record)
                        if info:
                            # Root notices plus explicit exception texts; the complete nested
                            # notice inventory is in the item's METADATA.json.
                            licences.update(n for n in info['licence_files']
                                            if n.split('!', 1)[-1].count('/') <= 2
                                            or PurePosixPath(n).name in ('COPYING.RUNTIME', 'COPYING3', 'COPYING.LIB')
                                            or PurePosixPath(n).stem in licence_terms.get(component, set()))
                            licences.update(info.get('extra_licence_files', []))
                            if component.startswith('zig-libc-') and rel.startswith('zig-' + config['packaging']['zig']['version'] + '/'):
                                # Retained headers often carry their own licence text/SPDX line.
                                subpath = rel.split('/', 1)[1]
                                source_name = info['downloads'][0]['name']
                                licences.add('items/' + item_id + '/' + source_name + '!zig-' + config['packaging']['zig']['version'] + '/' + subpath)
                        required_missing = status == 'unresolved' and not (policy and policy['class'] == 'provenance unknown, not required')
                        if required_missing or gaps:
                            missing[platform + ':' + component].add(name)
                if target is not None:
                    record = {'kind': 'symlink', 'symlink_target': target,
                              'symlink_target_sha256': hashlib.sha256(os.fsencode(target)).hexdigest(), 'sha256': None}
                    count['symlinks'] += 1
                else:
                    if isinstance(source, str):
                        digest = source  # Already hashed during initial input inventory.
                    elif isinstance(source, Path):
                        digest = sha256(source)
                    else:
                        digest = hashlib.file_digest(source, 'sha256').hexdigest()
                    record = {'kind': 'file', 'sha256': digest}
                    count['regular_files'] += 1
                record.update(platform={'macos': 'macos-arm64', 'linux': 'linux-x86_64'}[platform],
                              binary_path=name, components=components,
                              recipe_revision=config['suite']['build_commit'], licence_files=sorted(licences))
                records.append(record)
        if not count['regular_files']:
            raise ValueError('package has no regular toolchain files')
        count['manifest_records'] = count['regular_files'] + count['symlinks']
        counts[platform] = dict(count)
    if len(versions) > 1 or (version_override and versions and versions != {version_override}):
        raise ValueError('package version mismatch')
    version = version_override or (next(iter(versions)) if versions else None)
    if not version or not re.fullmatch(r'[A-Za-z0-9.+_-]+', version):
        raise ValueError('cannot infer package version; use --version')
    return version, records, counts, {k: len(v) for k, v in sorted(missing.items())}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--app', type=Path)
    ap.add_argument('--tarball', type=Path, help='Linux tar.gz or unpacked package root')
    ap.add_argument('--cache', type=Path, default=ROOT / 'out/source-archive/cache')
    ap.add_argument('--out', type=Path, default=ROOT / 'out/source-archive/dist')
    ap.add_argument('--version', help='Required only when an unpacked Linux root has been renamed')
    ap.add_argument('--repository-revision', help='Release commit for generated files; defaults to checkout HEAD')
    args = ap.parse_args(argv)
    if not (args.app or args.tarball):
        ap.error('supply --app and/or --tarball')
    allowed = (ROOT / 'out/source-archive').resolve()
    for path in (args.cache, args.out):
        if not path.resolve().is_relative_to(allowed):
            ap.error('downloads and outputs must stay under out/source-archive/')
    cache, out = args.cache.resolve(), args.out.resolve()
    if cache == out or out.is_relative_to(cache) or cache.is_relative_to(out):
        ap.error('cache and out must be separate directories')
    cache.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    check_cache(cache)
    config = json.loads(CONFIG.read_text())
    inputs = {p: v for p, v in [('macos', args.app), ('linux', args.tarball)] if v}
    version = input_version(inputs, args.version)
    inventories = inventory_packages(inputs)
    print('Inventoried package inputs before source preparation', flush=True)
    revision = args.repository_revision or git_command(['rev-parse', 'HEAD'], ROOT)
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        ap.error('repository revision must be a full commit')
    infos, grouped, failures = {}, defaultdict(list), []
    items = sorted(config['source_assembly']['items'], key=lambda i: (i['id'] != 'suite-recipe', i['id']))
    patch_hash = None
    selected = [i for i in items if set(i.get('platforms', inputs)).intersection(inputs)]
    try:
        prepare_containers(selected, cache)
    except (OSError, ValueError, tarfile.TarError) as exc:
        print('Receipt preparation: ' + sanitized_error(exc), flush=True)
    for item in items:
        if not set(item.get('platforms', inputs)).intersection(inputs):
            continue
        print('Preparing ' + item['id'], flush=True)
        try:
            entries, info = prepare_item(item, cache)
            if item['id'] == 'suite-recipe':
                inventory = patch_inventory(entries[0].path, item)
                patch_hash = hashlib.sha256(canonical(inventory)).hexdigest()
                grouped['suite'].append(Entry('PATCH-INPUTS.json', data=canonical(inventory), item=item['id']))
            info['patch_set_sha256'] = patch_hash
            # Hash distro patch payloads along with the suite inventory, when present.
            distro = [f for f in info['files'] if '.debian.tar.' in f['name'] or '.diff.' in f['name']]
            if distro:
                info['patch_set_sha256'] = hashlib.sha256(canonical({'suite': patch_hash, 'distro': distro})).hexdigest()
            entries = [e for e in entries if not e.name.endswith('/METADATA.json')]
            entries.append(Entry('items/' + item['id'] + '/METADATA.json', data=canonical(info), item=item['id']))
            infos[item['id']] = info
            grouped[item['group']].extend(entries)
        except (OSError, ValueError, tarfile.TarError, subprocess.SubprocessError) as exc:
            reason = sanitized_error(exc)
            failures.append({'source_item': item['id'], 'reason': reason,
                             'resolve_with': 'Restore the pinned source/checksum or network access; rerun.'})
            print('UNRESOLVED ' + item['id'] + ': ' + reason, flush=True)
    if 'suite-recipe' not in infos:
        raise ValueError('cannot assemble without the pinned suite recipe')
    for item in items:
        info = infos.get(item['id'])
        if not info:
            continue
        patch_names = set(item.get('patch_inputs', []))
        patch_files = [f for f in info['files'] if f['name'] in patch_names or f['name'].startswith(('port/', 'receipts/'))
                       or f['name'].endswith(('.diff', '.patch'))]
        build_inputs = {key: infos[key]['source_sha256'] for key in item.get('build_inputs', []) if key in infos}
        if patch_files or build_inputs:
            info['patch_set_sha256'] = hashlib.sha256(canonical({
                'suite_and_distro': info['patch_set_sha256'], 'patch_inputs': patch_files,
                'build_inputs': build_inputs})).hexdigest()
        if set(item.get('build_inputs', [])) - set(build_inputs):
            failures.append({'source_item': item['id'], 'reason': 'Missing declared build input',
                             'resolve_with': 'Restore every item in build_inputs.'})
        group = grouped[item['group']]
        metadata_name = 'items/' + item['id'] + '/METADATA.json'
        group[:] = [e for e in group if e.name != metadata_name]
        group.append(Entry(metadata_name, data=canonical(info), item=item['id']))
    parts, item_parts = [], {}
    for group in GROUPS:
        if grouped[group]:
            measured, mapping = split_parts(out, 'ecp5_emu-' + version + '-sources-' + group,
                                             grouped[group], infos,
                                             readme=config['source_assembly'].get('archive_readme', '').encode())
            parts.extend(measured)
            item_parts.update(mapping)
    version, records, counts, missing = make_manifest(config, inputs, infos, item_parts, patch_hash, revision, args.version, inventories)
    manifest_name = 'ecp5_emu-' + version + '-sources-manifest.jsonl'
    temporary = out / (manifest_name + '.partial')
    with temporary.open('wb') as stream:
        for record in sorted(records, key=lambda r: (r['platform'], r['binary_path'])):
            encoded = canonical(record)
            if re.search(rb'/(?:Users|home)/', encoded):
                raise ValueError('absolute user path in manifest record')
            stream.write(encoded)
    temporary.replace(out / manifest_name)
    unresolved = [e for e in config['source_assembly'].get('unresolved', []) if e.get('platform') in (*inputs, 'both')]
    triage = [e for e in config['source_assembly'].get('obligation_triage', []) if e.get('platform') in (*inputs, 'both')]
    copyleft_open = [e for e in triage if e['class'] == 'copyleft: open']
    required_ids = {iid for platform in inputs
                    for component, policy in config['source_assembly'].get('component_obligations', {}).get(platform, {}).items()
                    if policy['class'].startswith('copyleft:')
                    for iid in config['source_assembly']['bindings'].get(platform, {}).get(component, [])}
    required_failures = [f for f in failures if not triage or f['source_item'] in required_ids]
    for failure in required_failures:
        copyleft_open.append({'class': 'copyleft: open', 'source_item': failure['source_item'],
                              'reason': failure['reason'], 'resolve_with': failure['resolve_with']})
    status = 'incomplete' if unresolved or required_failures or missing or copyleft_open else ('copyleft-source-satisfied' if triage else 'complete')
    report = {'status': status,
              'version': version, 'recipe_revision': config['suite']['build_commit'],
              'repository_revision': revision, 'parts': parts, 'manifest': manifest_name,
              'manifest_sha256': sha256(out / manifest_name), 'counts': counts,
              'unresolved': unresolved, 'fetch_failures': failures, 'unresolved_file_counts': missing,
              'source_items': infos, 'cache_bytes': cache_size(cache),
              'obligation_triage': triage, 'triage_counts': dict(Counter(e['class'] for e in triage)),
              'copyleft_open': copyleft_open, 'required_source_failures': required_failures,
              'provenance_unknown_not_required': [e for e in triage if e['class'] == 'provenance unknown, not required'],
              'third_party_notices_addition': config['source_assembly'].get('third_party_notices_addition', '')}
    report_name = 'ecp5_emu-' + version + '-sources-report.json'
    (out / report_name).write_bytes(canonical(report))
    # Remove only stale parts for this version, after successfully writing the new manifest/report.
    wanted = {p['file'] for p in parts}
    for stale in out.glob('ecp5_emu-' + version + '-sources-*.tar.xz'):
        if stale.name not in wanted:
            stale.unlink()
    print(json.dumps({k: report[k] for k in ('status', 'parts', 'counts', 'cache_bytes')}, indent=2))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
