"""Load a reference design into ULX3S SRAM and record every UART line it prints.
Usage: run.py BITSTREAM PREFIX LOADS SECONDS

For each load, prints the lines that start with PREFIX, in the order they
came. The first and last line of a window can be cut by its start and end.
The shared board lock (out/board.lock in the main folder) is taken by the
caller around the whole run."""
import subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial

bit, prefix, loads, seconds = sys.argv[1], sys.argv[2], int(sys.argv[3]), float(sys.argv[4])
for n in range(1, loads + 1):
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    load = subprocess.run(["openFPGALoader", "-b", "ulx3s", bit], capture_output=True, text=True)
    ok = "DONE" in load.stdout + load.stderr
    buf = b""
    with serial.Serial(serial_port, 115200, timeout=0.2) as port:
        port.reset_input_buffer()
        end = time.time() + seconds
        while time.time() < end:
            buf += port.read(4096)
    lines = [l for l in buf.decode("ascii", "replace").split("\r\n") if l.startswith(prefix)]
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(lines)}", flush=True)
    for line in lines:
        print(f"  {line}", flush=True)
