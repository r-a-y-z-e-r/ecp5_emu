#!/usr/bin/env python3
"""Compare ECP5_TIMING_OUTPUT folders from two builds; fail on missing evidence.

Capture each build before running this script (all timings exclude snapshot IO):
  ECP5_FABRIC_CACHE=0 ECP5_TIMING_DESIGNS=life_full,picorv32_console,blinky_cycle \
    ECP5_TIMING_OUTPUT=out/before cargo test --release --lib \
    compile::timing::window_compile -- --ignored --nocapture
  # Repeat with the other build and ECP5_TIMING_OUTPUT=out/after.
  python3 tools/compare_compile.py out/before out/after

Production cache payloads are compared wherever encode supports the kernel.
Full test snapshots cover every kernel, including cache-ineligible timed blocks
and EBRs. Their vectors preserve signal/driver order; scopes are normalized and
hash collections sorted. Kernel/Compiled do not implement PartialEq.
"""
import argparse
import hashlib
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(chunk)
    return result.hexdigest()


def compare(left, right):
    # Compare bytes as well as printing their digests.
    with left.open("rb") as a, right.open("rb") as b:
        while True:
            x, y = a.read(1024 * 1024), b.read(1024 * 1024)
            if x != y:
                raise ValueError(f"different bytes: {left.name}")
            if not x:
                break
    print(f"  {left.name}: identical ({left.stat().st_size} bytes, sha256 {digest(left)})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--designs", default="life_full,picorv32_console,blinky_cycle")
    args = parser.parse_args()
    for design in args.designs.split(","):
        print(design)
        for suffix in ("json", "kernel.bin"):
            compare(args.before / f"{design}.{suffix}", args.after / f"{design}.{suffix}")
        paths = [folder / f"{design}.plan.bin" for folder in (args.before, args.after)]
        if any(path.exists() for path in paths):
            compare(*paths)
        else:
            compare(*(folder / f"{design}.plan.unavailable" for folder in (args.before, args.after)))
            print("  production encode unavailable; full kernel snapshot compared")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error
