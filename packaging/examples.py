#!/usr/bin/env python3
"""Stage the tracked examples archive, preserving internal relative symlinks."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import posixpath
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def stage(destination, manifest, root=ROOT):
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='examples-', dir=destination.parent) as temporary:
        temporary = Path(temporary)
        supplied = os.environ.get('ECP5_EXAMPLES_ARCHIVE')
        archive = Path(supplied).resolve(strict=True) if supplied else temporary / 'examples.tar'
        if not supplied:
            try:
                with archive.open('wb') as stream:
                    subprocess.run(['git', '-C', str(root), 'archive', '--format=tar',
                                    'HEAD', 'examples'], stdout=stream, stderr=subprocess.PIPE,
                                   check=True)
            except (subprocess.CalledProcessError, FileNotFoundError) as error:
                raise RuntimeError('Cannot archive tracked examples from Git HEAD; set '
                                   'ECP5_EXAMPLES_ARCHIVE to a tar produced by '
                                   '`git archive HEAD examples` for a copied tree without .git') from error
        with tarfile.open(archive, 'r:') as stream:
            members = stream.getmembers()
            names = set()
            for member in members:
                path = PurePosixPath(member.name)
                if (path.is_absolute() or '..' in path.parts or not path.parts
                        or path.parts[0] != 'examples' or str(path) in names):
                    raise ValueError(f'unsafe or duplicate examples archive entry: {member.name}')
                names.add(str(path))
                if member.issym():
                    target = PurePosixPath(posixpath.normpath(str(path.parent / member.linkname)))
                    if (PurePosixPath(member.linkname).is_absolute() or not target.parts
                            or target.parts[0] != 'examples'):
                        raise ValueError(f'example symlink escapes examples/: {member.name}')
                elif not member.isdir() and not member.isfile():
                    raise ValueError(f'unsupported examples archive entry: {member.name}')
            extracted = temporary / 'extracted'
            stream.extractall(extracted, members=members, filter='data')
            commit = stream.pax_headers.get('comment', '').strip()
        examples = extracted / 'examples'
        for required in ['ulx3s/README.md', 'gallery/README.md', 'showcase.toml']:
            if not (examples / required).is_file():
                raise ValueError(f'examples archive missing examples/{required}')
        records = []
        for path in sorted(examples.rglob('*')):
            if path.is_symlink():
                if not path.resolve(strict=True).is_relative_to(examples.resolve()):
                    raise ValueError(f'example link escapes archive: {path.relative_to(examples)}')
                records.append({'path': str(path.relative_to(examples)), 'symlink': os.readlink(path)})
            elif path.is_file():
                records.append({'path': str(path.relative_to(examples)),
                                'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
        file_bytes = sum((examples / r['path']).stat().st_size for r in records if 'sha256' in r)
        symlink_bytes = sum(len(os.fsencode(r['symlink'])) for r in records if 'symlink' in r)
        size = {'entries': len(records), 'files': sum('sha256' in r for r in records),
                'symlinks': sum('symlink' in r for r in records),
                'file_bytes': file_bytes, 'symlink_bytes': symlink_bytes,
                'bytes': file_bytes + symlink_bytes}
        payload = {'source': 'ECP5_EXAMPLES_ARCHIVE' if supplied else 'git archive HEAD examples',
                   'archive_sha256': hashlib.sha256(archive.read_bytes()).hexdigest(),
                   'commit': commit, 'size': size, 'files': records}
        if destination.exists() or destination.is_symlink():
            raise FileExistsError(f'examples destination already exists: {destination}')
        shutil.move(examples, destination)
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(json.dumps(payload, indent=2) + '\n')
        print(f'Examples location: {destination}')
        print(f'Examples size: {size["files"]} files + {size["symlinks"]} relative symlinks '
              f'({size["entries"]} tracked entries), {size["bytes"]} bytes '
              f'({file_bytes} file bytes + {symlink_bytes} link-target bytes); manifest: {manifest.name}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    args = parser.parse_args()
    stage(args.destination, args.manifest)


if __name__ == '__main__':
    main()
