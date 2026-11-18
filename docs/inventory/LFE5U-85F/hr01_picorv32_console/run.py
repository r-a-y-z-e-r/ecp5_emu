"""Loads a bitstream into ULX3S SRAM, then opens the serial port and types
each argument (with escapes) after a pause, printing what came back.
Usage: load_then_type.py BITSTREAM TEXT..."""
import subprocess, sys, time, datetime
import os

serial_port = os.environ.get("ECP5_SERIAL_PORT")
if not serial_port:
    raise SystemExit("Set ECP5_SERIAL_PORT to the board UART device before running this script.")

import serial
bit, texts = sys.argv[1], [t.encode().decode("unicode_escape").encode("latin-1") for t in sys.argv[2:]]
stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
load = subprocess.run(["openFPGALoader", "-b", "ulx3s", bit], capture_output=True, text=True)
lines = (load.stdout + load.stderr).strip().splitlines()
print("time:", stamp, "| loader exit:", load.returncode, "|", [l for l in lines if "DONE" in l or "rror" in l][-2:])
if load.returncode != 0:
    sys.exit(1)
with serial.Serial(serial_port, 115200, timeout=0.3) as port:
    time.sleep(0.3)
    print("on opening:", repr(port.read(4096)))
    for text in texts:
        port.write(text)
        port.flush()
        time.sleep(0.5)
        print(f"typed {text!r} ->", repr(port.read(4096)))
