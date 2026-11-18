"""Print HC08 LOCK and frequency versus configuration time, per build and load."""
import argparse
from pathlib import Path
import re

RECORD = re.compile(rb'HC08([LS])([01])([0-9A-F]{8})([0-9A-F]{8})')
PIN_HZ = 25_000_000
STEP = 250_000


def parse(data):
    records = []
    for line in data.splitlines():
        match = RECORD.fullmatch(line)
        if match:
            kind, lock, pin, op = match.groups()
            records.append(dict(kind=kind.decode(), lock=int(lock),
                                pin=int(pin, 16), op=int(op, 16)))
    return records


def analyse(data, label, emit=print, require_startup=True):
    records = parse(data)
    startup = {r['pin']: r for r in records if r['kind'] == 'S'}
    # Live records captured early can also fill the startup set.
    startup.update({r['pin']: r for r in records
                    if r['pin'] <= 200*STEP and r['pin'] % STEP == 0})
    missing = sorted(set(range(0, 201*STEP, STEP))-startup.keys())
    emit(f'{label}: bytes={len(data)} records={len(records)} '
         f'startup={201-len(missing)}/201 missing={len(missing)}')
    if not records:
        emit('ERROR: no complete HC08 lines (raw bytes retained).')
        return False
    if require_startup and missing:
        emit('ERROR: incomplete first 2 s; capture at least 8 s, or longer after a delayed serial open.')
    # Replayed records are immutable; a mismatch indicates corruption.
    seen, conflict = {}, False
    for r in records:
        key = r['pin']
        value = (r['lock'], r['op'])
        if key in seen and seen[key] != value:
            conflict = True
        seen[key] = value
    if conflict:
        emit('ERROR: conflicting records for the same pin timestamp.')
    if startup:
        high = [t for t, r in startup.items() if r['lock']]
        emit('startup first LOCK=1 sample: '+
             (f'{min(high)/PIN_HZ*1000:.3f} ms (10 ms resolution; synchronized)' if high
              else 'none in recovered startup samples'))
    emit('kind time_ms LOCK delta_pin delta_CLKOP ratio CLKOP_MHz')
    previous = {}
    # Group startup chronologically; keep live in arrival order for wrap handling.
    rows = [dict(startup[t], kind='S') for t in sorted(startup)]
    rows += [r for r in records if r['kind'] == 'L']
    for r in rows:
        old = previous.get(r['kind'])
        if old is None:
            values = '- - - -'
        else:
            dp = (r['pin']-old['pin']) & 0xffffffff
            do = (r['op']-old['op']) & 0xffffffff
            values = (f'{dp} {do} {do/dp:.8f} {PIN_HZ*do/dp/1e6:.6f}'
                      if dp else '0 - - -')
        emit(f"{r['kind']} {r['pin']/PIN_HZ*1000:.3f} {r['lock']} {values}")
        previous[r['kind']] = r
    return not conflict and (bool(records) and (not require_startup or not missing))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('raw', type=Path, nargs='+', help='build/load identified by each filename')
    p.add_argument('--allow-partial', action='store_true', help='e.g. short emulator captures')
    args = p.parse_args()
    results = [analyse(path.read_bytes(), path.name, require_startup=not args.allow_partial)
               for path in args.raw]
    raise SystemExit(not all(results))
