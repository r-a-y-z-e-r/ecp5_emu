#!/usr/bin/env python3
"""Reject personal paths, allowing only identical paths in recorded upstream inputs."""
import argparse
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys


def build_patterns(username, home):
    if not username or not home or not os.path.isabs(home):
        raise ValueError('build user and absolute HOME are required')
    return {'HOME': os.fsencode(home.rstrip('/') or '/'),
            'macOS user path': os.fsencode('/Users/' + username + '/'),
            'Linux user path': os.fsencode('/home/' + username + '/')}


def expressions(patterns):
    # Keep spaces in paths. Quotes, NUL and line boundaries delimit source/binary strings.
    return {label: re.compile(rb'(?<![A-Za-z0-9._~/-])' + re.escape(needle) +
                             (rb'(?![A-Za-z0-9._~-])' if label == 'HOME' else b'') +
                             rb'[^\x00\r\n\t"\'<>]*')
            for label, needle in patterns.items()}


def matches(data, regexes, final, start=0):
    found, unfinished = set(), len(data)
    for label, regex in regexes.items():
        for match in regex.finditer(data, start):
            if match.end() == len(data) and not final:
                unfinished = min(unfinished, match.start())
            else:
                found.add((label, match.group().rstrip(b' ')))
    return found, unfinished


def file_matches(path, patterns, chunk_size=1024 * 1024):
    """Return complete matching path strings, not just a shared personal prefix."""
    regexes = expressions(patterns)
    overlap = max(map(len, patterns.values())) - 1
    found, tail = set(), b''
    start = 0
    with path.open('rb') as stream:
        while chunk := stream.read(chunk_size):
            data = tail + chunk
            complete, unfinished = matches(data, regexes, final=False, start=start)
            found.update(complete)
            cut = min(unfinished, max(0, len(data) - overlap))
            if cut:
                # Retain real preceding context; a chunk boundary is not a path boundary.
                tail, start = data[cut - 1:], 1
            else:
                tail = data
            if len(tail) > 4 * 1024 * 1024:
                raise ValueError(f'unterminated personal path exceeds scan limit: {path}')
    found.update(matches(tail, regexes, final=True, start=start)[0])
    return found


def generated(relative):
    parts = relative.parts
    return (parts[0] == 'bin' or str(relative) in {
        'VERSION', 'environment.sh', 'components.json', 'bundle-manifest.json', 'source-origins.json'}
        or (len(parts) == 2 and parts[0].startswith('zig-')
            and parts[1] in {'clang', 'clang-version.txt'}))


class Upstream:
    def __init__(self, package, suite_root, zig_root, build_root):
        self.package = package
        self.suite_root = Path(suite_root).resolve() if suite_root else None
        self.zig_root = Path(zig_root).resolve() if zig_root else None
        self.build_root = Path(build_root) if build_root else None
        self.manifests, self.cached = {}, {}

    def source(self, path):
        for prefix in ['Contents/Resources/toolchain', 'share/toolchain']:
            top = self.package / prefix
            if not path.is_relative_to(top):
                continue
            relative = path.relative_to(top)
            if generated(relative):
                return None
            if top not in self.manifests:
                manifest = top / 'source-origins.json'
                if manifest.is_symlink():
                    raise ValueError('source-origins.json must be a generated regular file')
                data = json.loads(manifest.read_text()) if manifest.is_file() else {}
                if not isinstance(data, dict) or (data and
                        (data.get('version') != 1 or not isinstance(data.get('files'), dict))):
                    raise ValueError('invalid source-origins.json')
                self.manifests[top] = data
            data = self.manifests[top]
            origin = data.get('files', {}).get(str(relative))
            if not origin:
                return None
            if not isinstance(origin, dict):
                raise ValueError('invalid upstream origin record')
            name = PurePosixPath(origin['path'])
            if name.is_absolute() or '..' in name.parts:
                raise ValueError('unsafe upstream origin path')
            root = self.suite_root if origin['root'] == 'suite' else self.zig_root
            if origin['root'] == 'zig' and root is None and self.build_root:
                directory = data.get('zig_directory', '')
                if directory and directory not in {'.', '..'} and Path(directory).name == directory:
                    root = (self.build_root / 'target/packaging' / directory).resolve()
            if origin['root'] not in {'suite', 'zig'} or root is None:
                return None
            source = root / str(name)
            if not source.parent.resolve().is_relative_to(root):
                raise ValueError('upstream origin escapes source root')
            if (not source.exists() and not source.is_symlink()
                    or source.is_symlink() != path.is_symlink()
                    or origin['kind'] != ('symlink' if path.is_symlink() else 'file')):
                return None
            return source
        return None

    def allows(self, path, token, patterns):
        source = self.source(path)
        if source is None:
            return False
        if source not in self.cached:
            found = (matches(os.fsencode(os.readlink(source)), expressions(patterns), final=True)[0]
                     if source.is_symlink() else file_matches(source, patterns))
            self.cached[source] = {value for _, value in found}
        return token in self.cached[source]


def find_leaks(package, patterns, suite_root=None, zig_root=None, build_root=None):
    """Scan all regular files/link targets; generated files have no upstream allowance."""
    def fail(error):
        raise error

    upstream = Upstream(package, suite_root, zig_root, build_root)
    leaks = []
    for directory, dirs, files in os.walk(package, followlinks=False, onerror=fail):
        for name in sorted(dirs + files):
            path = Path(directory) / name
            mode = path.lstat().st_mode
            if stat.S_ISLNK(mode):
                found = matches(os.fsencode(os.readlink(path)), expressions(patterns), final=True)[0]
            elif stat.S_ISREG(mode):
                found = file_matches(path, patterns)
            else:
                continue
            labels = {label for label, token in found if not upstream.allows(path, token, patterns)}
            if labels:
                leaks.append((path.relative_to(package), sorted(labels)))
    return sorted(leaks)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('--root', type=Path, default=os.environ.get('ROOT', Path(__file__).resolve().parents[1]),
                        help='neutral build root, used only to locate downloaded Zig inputs; never a leak pattern')
    args = parser.parse_args(argv)
    try:
        if not args.package.is_dir():
            raise ValueError('package must be an existing directory')
        username = subprocess.check_output(['id', '-un'], text=True).strip()
        patterns = build_patterns(username, os.environ.get('HOME', ''))
        leaks = find_leaks(args.package, patterns, os.environ.get('OSS_CAD_SUITE_ROOT'),
                           os.environ.get('ZIG_ROOT'), args.root)
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f'Build-path guard could not complete: {error}', file=sys.stderr)
        return 1
    if leaks:
        print(f'Build-path guard FAILED: {len(leaks)} offending file(s)/symlink(s):', file=sys.stderr)
        for path, labels in leaks:
            print(f'  {path}: {", ".join(labels)}', file=sys.stderr)
        return 1
    print('Build-path guard PASS: no unapproved personal paths in package bytes/link targets')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
