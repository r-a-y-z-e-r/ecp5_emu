#!/usr/bin/env python3
"""Build the runtime libtrellis image bridge without changing the vendor checkout."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from toolchain_paths import resolve_toolchain, trellis_library
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--trellis', type=Path, help='prjtrellis source checkout (TRELLIS_ROOT or parent of ECP5_DB)')
    ap.add_argument('--prefix', type=Path, help='libtrellis install prefix (TRELLIS_PREFIX)')
    ap.add_argument('--output', type=Path,
                    default=Path(os.environ.get('ECP5_IMAGE_READER', ROOT/'target/trellis_image_reader')),
                    help='Reader output (ECP5_IMAGE_READER, otherwise target/trellis_image_reader)')
    args = resolve_toolchain(ap, ap.parse_args())
    library = trellis_library(args.prefix)
    source = ROOT/'tools/trellis_image_reader.cpp'
    args.output.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(['clang++', '-std=c++14', '-O2', '-Wall', '-Wextra', '-Werror',
                    '-I', str(args.trellis/'libtrellis/include'), '-I', str(args.prefix/'include'),
                    str(source), '-L', str(library.parent), '-ltrellis',
                    '-Wl,-rpath,'+str(library.parent), '-o', str(args.output)], check=True)
    report = {'schema_version': 1, 'reader_source_sha256': sha(source), 'reader_binary_sha256': sha(args.output),
              'libtrellis_sha256': sha(library), 'hardware': 'not_used'}
    args.output.with_suffix('.build.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps({'reader': str(args.output), 'status': 'built'}))


if __name__ == '__main__':
    main()
