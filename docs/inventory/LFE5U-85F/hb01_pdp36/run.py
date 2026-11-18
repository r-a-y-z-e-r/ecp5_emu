"""Load the HB01 bitstream into ULX3S SRAM and collect its report lines.
Usage: run.py BITSTREAM LOADS SECONDS

Each load is read for SECONDS (one pass of the program takes about 0.17 s).
For every step the distinct lines seen are printed; a stable bench shows
exactly one line per step. The lines of every load go to stdout, so
`run.py ... | tee observations.log` keeps the raw record."""
import re, subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial

bit, loads, seconds = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
pattern = re.compile(r"^S([0-9A-F]{2}) A([0-9A-F]{5}) B([0-9A-F]{5}) C([0-9A-F]{5}) D([0-9A-F]{5}) E([0-9A-F]{5})$")
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
    lines = buf.decode("ascii", "replace").split("\r\n")
    # Each line ends with a space before CR LF.
    good = [l.rstrip() for l in lines if pattern.match(l.rstrip())]
    by_step = {}
    for l in good:
        by_step.setdefault(l[:3], set()).add(l)
    unstable = {s: sorted(v) for s, v in by_step.items() if len(v) > 1}
    print(f"load {n} at {stamp}: loader_done={ok} lines={len(good)} other={len(lines) - len(good)} "
          f"steps={len(by_step)} unstable={len(unstable)}", flush=True)
    for step in sorted(by_step):
        for line in sorted(by_step[step]):
            print(f"  {line}", flush=True)
