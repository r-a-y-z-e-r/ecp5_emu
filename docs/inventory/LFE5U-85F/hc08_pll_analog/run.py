"""SRAM load first, THEN open serial, retain every byte and analyse each load.

Requires pyserial. Never probes ports, opens serial before load, or flashes.
"""
import argparse
import datetime
from pathlib import Path
import subprocess
import sys
import time
sys.dont_write_bytecode = True
from analyse import analyse
from gen import BASE_PROFILES, PROFILES


def collect(bitstream, port_name, raw_path, loader_path, seconds, serial_factory):
    error = None
    with raw_path.open('xb') as raw, loader_path.open('xb') as loader:
        try:
            subprocess.run(['openFPGALoader', '-b', 'ulx3s', str(bitstream)],
                           stdout=loader, stderr=subprocess.STDOUT, check=True)
            # Do not reset_input_buffer: the earliest buffered bytes matter.
            with serial_factory(port_name, 115200, timeout=0.1) as port:
                end = time.monotonic()+seconds
                while time.monotonic() < end:
                    chunk = port.read(4096)
                    raw.write(chunk)
                    raw.flush()
        except (OSError, subprocess.SubprocessError) as exc:
            error = f'{type(exc).__name__}: {exc}'
    return error


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('build', type=Path, help='external scratch directory made by build.py')
    p.add_argument('serial_port', help='explicit port; no auto discovery')
    p.add_argument('--output', type=Path, required=True, help='new run directory')
    p.add_argument('--profiles', nargs='+', choices=PROFILES, default=BASE_PROFILES)
    p.add_argument('--loads', type=int, default=3)
    p.add_argument('--seconds', type=float, default=8)
    args = p.parse_args()
    if args.loads < 1 or args.seconds < 5:
        p.error('--loads must be positive and --seconds >=5 (8 recommended)')
    if len(args.profiles) != len(set(args.profiles)):
        p.error('duplicate profiles')
    if args.output.exists():
        p.error('--output must be new; captures are never overwritten')
    for profile in args.profiles:
        if not (args.build/profile/'top.bit').is_file():
            p.error(f'missing {profile}/top.bit')
    import serial
    args.output.mkdir(parents=True)
    passed = True
    with (args.output/'analysis.txt').open('x') as log:
        def emit(line):
            print(line, flush=True)
            log.write(line+'\n')
            log.flush()
        for profile in args.profiles:
            for load in range(1, args.loads+1):
                name = f'{profile}.load-{load:03d}'
                raw = args.output/(name+'.bin')
                emit(f'{name} UTC={datetime.datetime.now(datetime.timezone.utc).isoformat()}')
                error = collect(args.build/profile/'top.bit', args.serial_port, raw,
                                args.output/(name+'.loader.log'), args.seconds, serial.Serial)
                good = analyse(raw.read_bytes(), name, emit)
                if error:
                    emit('ERROR: '+error)
                passed &= good and error is None
    return not passed


if __name__ == '__main__':
    raise SystemExit(main())
