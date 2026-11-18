#!/usr/bin/env python3
"""Prepare the selected 2026-11-18 takes for publication, entirely offline.

Run from any directory. --check checks selected sources and provenance only;
without it, copy stills and re-encode clips into site/media/. No capture or lock.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

import encode

KIT = Path(__file__).resolve().parent
ROOT = KIT.parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    manifest = json.loads((KIT / 'publication.json').read_text())
    entries = {e['label']: e for e in json.loads((KIT / 'batch.json').read_text())['records']}
    checked = []
    for row in manifest['files']:
        name, folder = row['file'], row['take']
        if Path(name).name != name or Path(folder).name != folder:
            raise ValueError('publication paths must be basenames')
        take = ROOT / 'out/site_media' / folder
        source = take / (row['raw'] if row['kind'] == 'clip' else name)
        if Path(row['raw']).name != row['raw'] or not source.is_file():
            raise ValueError(f'missing selected source: {source}')
        provenance = json.loads((take / 'provenance.json').read_text())
        candidate = next(c for c in provenance['candidates'] if c['shot'] == row['shot'])
        if provenance['gui_sha256'] != manifest['gui_sha256'] or candidate['rect'] != row['rect']:
            raise ValueError(f'provenance mismatch: {name}')
        stream = encode.probe(source)['streams'][0]
        if row['kind'] == 'clip':
            encode.clip_profile(**row['encoding'])
            encode.check_source(stream, row['rect'][2:])
            if row['encoding'] != entries[row['shot']]['clip_encoding']:
                raise ValueError(f'publication/capture profile mismatch: {name}')
        else:
            if source.stat().st_size > encode.LIMIT:
                raise ValueError(f'WebP exceeds 250 KiB: {name}')
            if not entries[row['shot']].get('crop'):
                encode.check_source(stream, row['rect'][2:])
            # Audit the two takes made before implicit cropping was removed.
            if row['shot'] in ('cross-probe', 'start'):
                encode.check_source(encode.probe(take / row['raw'])['streams'][0], row['rect'][2:])
        checked.append((row, source))
    if args.check:
        print(f'PASS: {len(checked)} selected assets, provenance, aspect ratios and encoding profiles; no files changed')
        return
    for row, source in checked:
        dest = ROOT / 'site/media' / row['file']
        if row['kind'] == 'clip':
            encode.clip(source, dest, row['seconds'], expected_size=row['rect'][2:], **row['encoding'])
            if row['old_bytes'] and dest.stat().st_size > row['old_bytes'] * 1.5:
                raise ValueError(f'clip exceeds 1.5x previous size: {row["file"]}')
        else:
            shutil.copy2(source, dest)
        print(f'{row["file"]}: {row["old_bytes"]} -> {dest.stat().st_size} bytes; '
              f'sha256={hashlib.sha256(dest.read_bytes()).hexdigest()}', flush=True)


if __name__ == '__main__':
    main()
