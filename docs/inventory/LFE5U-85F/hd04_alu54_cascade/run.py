"""Load a reference design into ULX3S SRAM and collect its UART lines.
Usage: run.py BITSTREAM PREFIXES LOADS SECONDS

As hd02's run.py, with several one-letter prefixes (HD04 prints one "I" line
and then "T" lines). Prints, for each load, how many lines arrived and the
distinct ones. The last line of a window can be cut by the window's end."""
import subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial

bit, prefixes, loads, seconds = sys.argv[1], sys.argv[2], int(sys.argv[3]), float(sys.argv[4])
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
    lines = [l for l in buf.decode("ascii", "replace").split("\r\n") if l[:1] in tuple(prefixes)]
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(lines)} distinct={sorted(set(lines))}", flush=True)
