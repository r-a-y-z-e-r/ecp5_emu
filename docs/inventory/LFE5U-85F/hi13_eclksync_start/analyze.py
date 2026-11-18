"""Decode raw HI13 UART bytes or runner logs; report startup deficits per lane."""
import argparse
import json
from pathlib import Path
import sys
sys.dont_write_bytecode = True
from run import parse

FIELDS = ('I', 'A', 'CE_at', 'G_in', 'P_r', 'P_f', 'G_r', 'G_f')


def inspect(record):
    data = record.split()[1]
    values = [int(data[n:n+8], 16) for n in range(0, 128, 8)]
    snapshots = [dict(zip(FIELDS, values[n:n+8])) for n in (0, 8)]
    errors = []
    for number, s in enumerate(snapshots):
        if s['I'] != (524288, 1048576)[number]:
            errors.append(f'snapshot {number}: unexpected freeze boundary')
        if s['CE_at'] != 64 or s['A'] != s['I']-s['CE_at']:
            errors.append(f'snapshot {number}: CE/pin reference mismatch')
        if s['G_in'] != s['A']:
            errors.append(f'snapshot {number}: DCCA input qualification failed (G_in != A)')
        s['P_deficit'] = s['I']-s['P_r']-s['P_f']
        s['G_deficit'] = s['G_in']-s['G_r']-s['G_f']
        for lane in ('P', 'G'):
            if abs(s[lane+'_r']-s[lane+'_f']) > 1 or s[lane+'_deficit'] < 0:
                errors.append(f'snapshot {number}: invalid /2 {lane} edge balance')
    for lane in ('P', 'G'):
        if snapshots[0][lane+'_deficit'] != snapshots[1][lane+'_deficit']:
            errors.append(lane+': deficit changed between snapshots; not a fixed startup loss')
    p, g = snapshots[-1]['P_deficit'], snapshots[-1]['G_deficit']
    if errors:
        reading = 'invalid measurement: investigate qualification/errors before interpreting startup'
    elif p == 0 and g in (4, 5):
        reading = ('supports pre-running versus delayed-clock history: P passes from the fabric '
                   'boundary, G pays startup; does not directly observe configuration-time edges')
    elif p == g == 4:
        reading = 'both pay the implemented HI04 four-fall startup; HI12 zero deficit not reproduced'
    elif p == g == 0:
        reading = ('both pass immediately; delayed ECLKI did not preserve the HI04 initial block; '
                   'simple input-edge-only prehistory explanation is insufficient')
    else:
        reading = 'different startup result; retain raw bytes and investigate phase/path/init behavior'
    return dict(snapshots=snapshots, errors=errors, reading=reading)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('capture', type=Path, nargs='+')
    args = parser.parse_args()
    failed = False
    for path in args.capture:
        records, heartbeats = parse(path.read_bytes())
        entries = [inspect(record) for record in sorted(set(records))]
        print(json.dumps(dict(file=path.name, records=len(records), heartbeats=len(heartbeats),
                              distinct=len(entries), measurements=entries), indent=2))
        failed |= not entries or any(entry['errors'] for entry in entries)
        # A raw per-load file must repeat one immutable result. A combined runner
        # log may legitimately contain a different startup result on each load.
        failed |= path.suffix == '.bin' and len(entries) != 1
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
