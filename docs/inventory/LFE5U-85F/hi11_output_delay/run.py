"""Load ULX3S SRAM first, then open UART; never writes flash.
Usage: python3 run.py BITSTREAM PREFIX LOADS SECONDS [--port PORT] [--out DIR]
Example: python3 run.py /tmp/hi11/hi11/top.bit H11F 3 3 --out /tmp/hi11-records
"""
import argparse
import datetime
import os
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('bitstream')
parser.add_argument('prefix', choices=['H11F', 'H11G', 'H11B',
                                     *(f'H11S{i}' for i in range(6))])
parser.add_argument('loads', type=int)
parser.add_argument('seconds', type=float)
parser.add_argument('--port', default=os.environ.get('ECP5_SERIAL_PORT'))
parser.add_argument('--out', type=Path, default=Path('hi11-records'))
args = parser.parse_args()
if not args.port:
    parser.error("set ECP5_SERIAL_PORT or pass --port with the board UART device")
import serial

args.out.mkdir(parents=True, exist_ok=True)
for n in range(1, args.loads + 1):
    stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
    load = subprocess.run(['openFPGALoader', '-b', 'ulx3s', args.bitstream],
                          capture_output=True, text=True, check=True)
    # JTAG and UART share the FTDI interface: open only AFTER the loader exits.
    buf = bytearray()
    with serial.Serial(args.port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end = time.monotonic() + args.seconds
        while time.monotonic() < end:
            buf.extend(port.read(4096))
    path = args.out / f'{args.prefix}-load-{n}.txt'
    path.write_bytes(buf)
    frame_chars = 4111 if args.prefix.startswith('H11S') else 2061
    lines = [line for line in buf.decode('ascii', 'replace').split('\r\n')
             if line.startswith(args.prefix) and len(line) == frame_chars]
    # Persist only the record basename, independent of the operator's home path.
    log = f'load {n} at {stamp}: bytes={len(buf)} complete_lines={len(lines)} record={path.name}'
    print(log, flush=True)
    (args.out / f'{args.prefix}-load-{n}.log').write_text(
        log + '\n' + load.stdout + load.stderr)
    if not lines:
        raise SystemExit('No complete record; inspect the saved UART bytes and loader log')
