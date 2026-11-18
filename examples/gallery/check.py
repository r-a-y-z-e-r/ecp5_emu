#!/usr/bin/env python3
"""Checks a gallery demo against its expected UART output.

Usage: examples/gallery/check.py DEMO... [--build] [--emulator] [--board [--loads N]]
                                          [--compare FILE]

DEMO is a folder name under examples/gallery, several of them, or `all` for every folder
with a check.json. Each demo has a `check.json`:

  {"cycles": 4000000,          # board clock cycles the expected output covers
   "input": "r",               # UART bytes sent to the design (JSON string escapes)
   "marker": "RESTART",        # the comparison starts at the first occurrence of this text
   "board_seconds": 3.0,      # how long the board check listens after sending the input
   "needs": "peripherals",  # optional: pass --peripherals; skip if that option is unknown
   "peripherals": ["oled"]} # optional required models; default ["sdram"]

and `expected/uart.txt`, the bytes an RTL simulation of top.v (the demo's sim/) printed
over `cycles` cycles with `input` sent from the start.

A run passes when the expected text from the marker on is a prefix of the run's text from
the marker on. The marker is the first thing the design prints in answer to the input, so
whatever the design printed before the input reached it (at a different moment on the
board, where the serial port opens after the load) is left out.

--build     runs examples/ulx3s/build.sh into target/gallery/DEMO
--emulator  runs the emulator in fast mode at the board's 25 MHz clock for `cycles` plus a
            margin, with `input` on the UART receiver; the transcript goes to
            target/gallery/DEMO/emulator_uart.txt. With needs=peripherals, require healthy
            status for each model in peripherals (default: SDRAM). OLED also checks CRCs.
--board     takes the board's lock (out/board.lock of the main checkout, shared by every
            session that uses the ULX3S), loads target/gallery/DEMO/top.bit into the ULX3S
            SRAM (openFPGALoader -b ulx3s; the flash is never written), opens the serial
            port, sends `input`, listens for
            `board_seconds`; each load's transcript goes to target/gallery/DEMO/board_uart_N.txt
            and a line to target/gallery/DEMO/board_runs.log
--compare   compares FILE (a transcript from elsewhere, e.g. the GUI) with the expected text
"""
import argparse
import datetime
import glob
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[2]


def load_check(demo):
    folder = ROOT / "examples/gallery" / demo
    check = json.loads((folder / "check.json").read_text())
    check["input_bytes"] = check.get("input", "").encode("latin-1")
    check["expected"] = (folder / "expected/uart.txt").read_bytes()
    return folder, check


def compare(expected, got, marker):
    """Returns (ok, message): whether `expected` from the marker on is a prefix of `got`
    from the marker on."""
    m = marker.encode("latin-1")
    e = expected.find(m)
    g = got.find(m)
    if e < 0:
        return False, f"the marker {marker!r} is not in the expected text"
    if g < 0:
        return False, f"the marker {marker!r} never arrived ({len(got)} bytes arrived)"
    want, have = expected[e:], got[g:]
    if have.startswith(want):
        return True, f"{len(want)} bytes from the marker on match ({len(have)} arrived from the marker)"
    n = next((i for i, (a, b) in enumerate(zip(want, have)) if a != b), min(len(want), len(have)))
    lo = max(0, n - 60)
    return False, (
        f"differs at byte {n} after the marker (of {len(want)} expected, {len(have)} arrived):\n"
        f"  expected {want[lo:n + 60]!r}\n"
        f"  arrived  {have[lo:n + 60]!r}"
    )


def build(demo, folder, out):
    out.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(ROOT / "examples/ulx3s/build.sh"), str(folder), str(out)], check=True)


def peripherals_option_unknown(stderr):
    """Match only main.rs's unknown-option error, with Rust's optional Error: prefix."""
    return stderr.strip() in (
        b"unknown option --peripherals",
        b"Error: unknown option --peripherals",
    )


def sdram_status(stderr):
    """Return (healthy, lines); a required SDRAM report must be present."""
    lines = [line for line in stderr.decode(errors="replace").splitlines()
             if line.startswith("peripheral: sdram ")]
    required = {"init": "yes", "timing_violations": "0",
                "unknown_samples": "0", "refresh_shortfalls": "0"}
    for line in lines:
        fields = dict(field.split("=", 1) for field in line.split()[2:] if "=" in field)
        if any(fields.get(key) != value for key, value in required.items()):
            return False, lines
    return bool(lines), lines


def oled_status(stderr, uart):
    """Require completed, error-free OLED fills with a CRC printed in this run."""
    lines = [line for line in stderr.decode(errors="replace").splitlines()
             if line.startswith("peripheral: oled ")]
    printed = set(re.findall(rb"(?m)^F[0-9A-Fa-f]{4} C([0-9A-Fa-f]{8})\b", uart))
    printed = {crc.decode().upper() for crc in printed}
    for line in lines:
        fields = dict(field.split("=", 1) for field in line.split()[2:] if "=" in field)
        if any(fields.get(k) != v for k, v in
               {"errors": "0", "unknown_commands": "0", "display_on": "yes"}.items()):
            return False, lines
        try:
            if int(fields.get("frames", "0")) <= 0:
                return False, lines
        except ValueError:
            return False, lines
        if fields.get("crc32", "").upper() not in printed:
            return False, lines
    return bool(lines), lines


def peripheral_status(check, stderr, uart):
    """Only the demo's required models decide the result; preserve the SDRAM default."""
    lines, failed = [], []
    for model in check.get("peripherals", ["sdram"]):
        if model == "sdram":
            healthy, report = sdram_status(stderr)
        elif model == "oled":
            healthy, report = oled_status(stderr, uart)
        else:
            healthy, report = False, []
        lines.extend(report)
        if not healthy:
            failed.append(model)
    return not failed, lines, ", ".join(failed)


def emulator(demo, check, out):
    emu = ROOT / "target/release/ecp5_emu"
    cycles = check["cycles"] + check.get("margin_cycles", check["cycles"] // 10 + 200_000)
    cmd = [
        str(emu), "emulate", str(out / "top.config"), "--names", str(out / "top_routed.json"),
        "--fast", "--startup", "configured", "--scope", "observed", "--clock", "net:clk_25mhz",
        "--period-ns", "40", "--cycles", str(cycles),
        "--console-out", "net:ftdi_rxd", "--console-in", "net:ftdi_txd",
    ]
    needs_peripherals = check.get("needs") == "peripherals"
    if needs_peripherals:
        # Older main.rs consumes a value before rejecting an unknown option;
        # keep another argument after this flag so it reports "unknown option".
        cmd.insert(3, "--peripherals")
    start = time.time()
    run = subprocess.run(cmd, input=check["input_bytes"], capture_output=True)
    wall = time.time() - start
    (out / "emulator_uart.txt").write_bytes(run.stdout)
    (out / "emulator.log").write_bytes(run.stderr)
    if run.returncode != 0:
        if needs_peripherals and peripherals_option_unknown(run.stderr):
            print(f"{demo}: skipped, needs emulate --peripherals (checked in the window, HR03)")
            return True
        print(run.stderr.decode(errors="replace"), file=sys.stderr)
        return False
    speed = [l for l in run.stderr.decode(errors="replace").splitlines() if l.startswith("Fast mode")]
    for line in speed:
        print(f"emulator: {line}")
    ok, message = compare(check["expected"], run.stdout, check.get("marker", ""))
    if ok and needs_peripherals:
        models_ok, lines, failed = peripheral_status(check, run.stderr, run.stdout)
        for line in lines:
            print(f"emulator: {line}")
        if not models_ok:
            ok = False
            message = f"peripheral status failed: {failed}"
    print(f"emulator: {'PASS' if ok else 'FAIL'}: {message} ({wall:.1f} s)")
    return ok


def lock_path():
    """The board's lock: out/board.lock of the main checkout, the same file for every
    worktree (git's common directory is the main checkout's .git). ECP5_BOARD_LOCK
    overrides it."""
    if os.environ.get("ECP5_BOARD_LOCK"):
        return pathlib.Path(os.environ["ECP5_BOARD_LOCK"])
    common = subprocess.run(["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    return pathlib.Path(common).parent / "out/board.lock"


class BoardLock:
    """Holds the board's lock file while loading and reading the board: created
    atomically, waited for while another session holds it, removed afterwards."""

    def __init__(self, who, wait_s=1800):
        self.path, self.who, self.wait_s = lock_path(), who, wait_s

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        start = time.time()
        while True:
            try:
                fd = os.open(self.path, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                os.write(fd, f"{self.who} {int(time.time())}\n".encode())
                os.close(fd)
                return self
            except FileExistsError:
                holder = self.path.read_text(errors="replace").strip() if self.path.exists() else "?"
                if time.time() - start > self.wait_s:
                    raise SystemExit(f"board: {self.path} held by {holder!r} for over {self.wait_s} s; "
                                     "check that session and remove a stale lock by hand")
                print(f"board: waiting for the lock {self.path} ({holder})", file=sys.stderr)
                time.sleep(10)

    def __exit__(self, *exc):
        try:
            self.path.unlink()
        except FileNotFoundError:
            pass


def board(demo, check, out, loads):
    with BoardLock(f"demos:{demo}"):
        return board_locked(demo, check, out, loads)


def board_locked(demo, check, out, loads):
    bit = out / "top.bit"
    digest = hashlib.sha256(bit.read_bytes()).hexdigest()
    ports = sorted(glob.glob("/dev/cu.usbserial-*"))
    if not ports:
        print("board: no /dev/cu.usbserial-* (is the ULX3S plugged in?)", file=sys.stderr)
        return False
    import serial  # pyserial

    passed = 0
    for n in range(1, loads + 1):
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        load = subprocess.run(["openFPGALoader", "-b", "ulx3s", str(bit)], capture_output=True, text=True)
        done = load.returncode == 0 and "DONE" in load.stdout + load.stderr
        got = b""
        if done:
            with serial.Serial(ports[0], 115200, timeout=0.05) as port:
                time.sleep(0.3)
                got += port.read(65536)
                port.write(check["input_bytes"])
                port.flush()
                end = time.time() + check["board_seconds"]
                while time.time() < end:
                    got += port.read(65536)
        (out / f"board_uart_{n}.txt").write_bytes(got)
        ok, message = compare(check["expected"], got, check.get("marker", "")) if done else (False, "the load failed")
        passed += ok
        line = (f"load {n} at {stamp}: loader_done={done} bytes={len(got)} "
                f"{'PASS' if ok else 'FAIL'}: {message.splitlines()[0]} top.bit sha256 {digest}")
        print(f"board: {line}")
        if not ok and done:
            print(message)
        with open(out / "board_runs.log", "a") as log:
            log.write(line + "\n")
    print(f"board: {passed} of {loads} loads pass")
    return passed == loads


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("demo", nargs="+", help="demo folder names, or `all`")
    ap.add_argument("--build", action="store_true")
    ap.add_argument("--emulator", action="store_true")
    ap.add_argument("--board", action="store_true")
    ap.add_argument("--loads", type=int, default=3)
    ap.add_argument("--compare", type=pathlib.Path)
    args = ap.parse_args()
    gallery = ROOT / "examples/gallery"
    demos = (sorted(p.parent.name for p in gallery.glob("*/check.json"))
             if args.demo == ["all"] else args.demo)
    if args.compare and len(demos) > 1:
        ap.error("--compare takes one demo")
    ok = True
    for demo in demos:
        if len(demos) > 1:
            print(f"== {demo}", flush=True)
        folder, check = load_check(demo)
        out = ROOT / "target/gallery" / demo
        if args.build:
            build(demo, folder, out)
        if args.emulator:
            ok &= emulator(demo, check, out)
        if args.board:
            ok &= board(demo, check, out, args.loads)
        if args.compare:
            good, message = compare(check["expected"], args.compare.read_bytes(), check.get("marker", ""))
            print(f"{args.compare}: {'PASS' if good else 'FAIL'}: {message}")
            ok &= good
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
