"""SRAM-load HI12, then collect UART with per-load raw byte companions.
Usage: run.py BITSTREAM SERIAL_PORT --output run.log --loads 3 --seconds 2
No complete HC record makes the command fail after saving every load.
"""
import argparse
import datetime
from pathlib import Path
import re
import subprocess
import sys
import time

COMPLETE = re.compile(r'HC[0-4] [0-9A-F]{176}')
HEARTBEAT = re.compile(r'HB[0-4] [0-9A-F]{2} [0-9A-F]{8}')


def parse(data):
    lines = data.decode('ascii', 'replace').splitlines()
    return ([s for s in lines if COMPLETE.fullmatch(s)],
            [s for s in lines if HEARTBEAT.fullmatch(s)])


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('bitstream')
    p.add_argument('serial_port')
    p.add_argument('--output', required=True, type=Path,
                   help='new parsed log; raw load-NNN.bin files are saved beside it')
    p.add_argument('--loads', type=int, default=3)
    p.add_argument('--seconds', type=float, default=2)
    args = p.parse_args()
    if args.loads < 1 or args.seconds <= 0:
        p.error('--loads and --seconds must be positive')
    raw_paths = [args.output.with_name(f'{args.output.stem}.load-{n:03d}.bin')
                 for n in range(1, args.loads+1)]
    if args.output.exists() or any(path.exists() for path in raw_paths):
        p.error('output or raw companion already exists; choose a new --output')
    import serial
    args.output.parent.mkdir(parents=True, exist_ok=True)
    failed = False
    with args.output.open('x') as parsed:
        def report(line):
            print(line, flush=True)
            parsed.write(line+'\n')
            parsed.flush()

        for load, raw_path in enumerate(raw_paths, 1):
            stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
            error = None
            # Create the raw file even if loading/opening/reading later fails.
            with raw_path.open('xb') as raw:
                try:
                    # No flash option. Serial is opened only after SRAM load.
                    subprocess.run(['openFPGALoader', '-b', 'ulx3s', args.bitstream], check=True)
                    with serial.Serial(args.serial_port, 115200, timeout=0.2) as port:
                        end = time.monotonic() + args.seconds
                        while time.monotonic() < end:
                            chunk = port.read(4096)
                            raw.write(chunk)
                            raw.flush()
                except (OSError, subprocess.SubprocessError) as exc:
                    error = f'{type(exc).__name__}: {exc}'
            data = raw_path.read_bytes()
            records, heartbeats = parse(data)
            report(f'load {load} at {stamp}: bytes={len(data)} raw={raw_path.name} '
                   f'complete={len(records)} distinct={len(set(records))} heartbeats={len(heartbeats)}')
            if heartbeats:
                last = heartbeats[-1]
                flags = int(last[4:6], 16)
                names = ('LOCK', 'OBSERVER_SEEN', 'MID_DONE', 'FREEZE', 'FINISHED')
                report(last+' '+ ' '.join(f'{name}={(flags >> bit) & 1}'
                                         for bit, name in enumerate(names)))
            for record in sorted(set(records)):
                report(record)
            if error:
                report('ERROR: '+error)
            if not records:
                report('ERROR: no complete HI12 startup record; '
                       + ('heartbeat received (observer/measurement incomplete)' if heartbeats
                          else 'no heartbeat received (inspect raw bytes)'))
            failed |= error is not None or not records
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
