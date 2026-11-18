"""Load a bitstream into ULX3S SRAM and collect UART report lines.
Usage: run.py BITSTREAM LOADS SECONDS"""
import re, subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial
bit, loads, seconds = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
pattern = re.compile(r"L=([0-9A-F]{6}) P=([0-9A-F]{6}) F=([0-9A-F]{6}) D=([0-9A-F]{2})")
for n in range(1, loads + 1):
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    load = subprocess.run(["openFPGALoader", "-b", "ulx3s", bit], capture_output=True, text=True)
    ok = "DONE" in load.stdout + load.stderr
    lines = []
    with serial.Serial(serial_port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end = time.time() + seconds
        buf = b""
        while time.time() < end:
            buf += port.read(256)
        lines = buf.decode("ascii", "replace").splitlines()
    parsed = [m.groups() for m in map(pattern.search, lines) if m]
    values = sorted({tuple(int(x, 16) for x in g) for g in parsed})
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(parsed)} distinct={values}", flush=True)
