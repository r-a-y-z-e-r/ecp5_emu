#!/usr/bin/env python3
"""Load ULX3S SRAM and stream chronological UART records, based on HB11/run.py.

Usage: run.py BITSTREAM PREFIX LOADS SECONDS [--port DEVICE]
Example: run.py /tmp/hy04.bit Y4 1 90 --settle-seconds 2 --port DEVICE > capture.txt
Requires pyserial and openFPGALoader. This helper is for the later hardware run.
"""
import argparse
import datetime
import os
import subprocess
import sys
import time


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("bitstream")
    p.add_argument("prefix")
    p.add_argument("loads", type=int)
    p.add_argument("seconds", type=float)
    p.add_argument("--port", default=os.environ.get("ECP5_SERIAL_PORT"))
    p.add_argument("--settle-seconds", type=float, default=2.0,
                   help="wait after loading before opening UART (default: 2 s)")
    a = p.parse_args()
    if not a.port:
        p.error("set ECP5_SERIAL_PORT or pass --port with the board UART device")
    if a.loads < 1 or a.seconds <= 0 or a.settle_seconds < 0:
        p.error("loads and seconds must be positive; settle-seconds must be nonnegative")
    import serial
    for number in range(1, a.loads+1):
        stamp = datetime.datetime.now(datetime.timezone.utc).isoformat()
        # This FTDI exposes JTAG and UART on one chip. Keep UART closed while
        # openFPGALoader owns it, then allow the macOS device to settle.
        result = subprocess.run(["openFPGALoader", "-b", "ulx3s", a.bitstream],
                                capture_output=True, text=True)
        print(f"load {number} at {stamp}: exit={result.returncode}", file=sys.stderr, flush=True)
        print(result.stdout + result.stderr, file=sys.stderr, end="", flush=True)
        if result.returncode:
            raise SystemExit(result.returncode)
        time.sleep(a.settle_seconds)
        with serial.Serial(a.port, 115200, timeout=0.2) as port:
            port.reset_input_buffer()
            end, pending, done = time.monotonic()+a.seconds, b"", False
            saw_initial = False
            while time.monotonic() < end:
                pending += port.read(4096)
                while b"\n" in pending:
                    raw, pending = pending.split(b"\n", 1)
                    line = raw.rstrip(b"\r").decode("ascii", "replace")
                    if line.startswith(a.prefix):
                        print(line, flush=True)
                        if (line.startswith("Y4I") and len(line) == 25
                                and all(c in "0123456789ABCDEF" for c in line[3:])):
                            saw_initial = True
                        # Attaching after completion may first catch an old Z.
                        # Wait for a full replay marker before accepting Z.
                        if saw_initial and line.startswith("Y4Z"):
                            done = True
                if done:
                    break
            if not done:
                print(f"load {number}: timed out before Y4Z; retain partial capture for analysis",
                      file=sys.stderr, flush=True)
                raise SystemExit(2)


if __name__ == "__main__":
    main()
