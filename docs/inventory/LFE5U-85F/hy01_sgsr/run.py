"""Load SRAM first, then open UART; emit complete raw HY01 records plus metadata.
Usage: run.py BITSTREAM --port DEVICE --loads 3 --seconds 20
Requires openFPGALoader and pyserial. Never writes flash.
"""
import argparse
import datetime
import os
import subprocess
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('bitstream')
    p.add_argument('--port', default=os.environ.get('ECP5_SERIAL_PORT'))
    p.add_argument('--loads', type=int, default=3)
    p.add_argument('--seconds', type=float, default=20)
    args = p.parse_args()
    if not args.port:
        p.error("set ECP5_SERIAL_PORT or pass --port with the board UART device")
    if args.loads < 1 or args.seconds <= 0:
        p.error('loads and seconds must be positive')
    import serial
    for load in range(1, args.loads+1):
        stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
        result = subprocess.run(['openFPGALoader', '-b', 'ulx3s', args.bitstream],
                                capture_output=True, text=True)
        if result.returncode or 'DONE' not in result.stdout + result.stderr:
            raise SystemExit(f'load {load} failed: {result.stdout}{result.stderr}')
        print(f'# load={load} utc={stamp} loader_done=True', flush=True)
        lines = fragments = 0
        # Deliberately after SRAM loading, as in HB11. Initial partial UART
        # line is discarded; s repeats its frozen first window indefinitely.
        with serial.Serial(args.port, 115200, timeout=0.2) as port:
            port.reset_input_buffer()
            pending = b''
            end = time.monotonic() + args.seconds
            while time.monotonic() < end:
                pending += port.read(4096)
                while b'\n' in pending:
                    raw, pending = pending.split(b'\n', 1)
                    if raw.startswith((b'Y1R', b'Y1S', b'Y1B', b'Y1C')):
                        print(raw.rstrip(b'\r').decode('ascii', 'replace'), flush=True)
                        lines += 1
                    else:
                        fragments += 1
            if pending:
                fragments += 1
        print(f'# load={load} candidate_lines={lines} discarded_fragments={fragments}', flush=True)


if __name__ == '__main__':
    main()
