#!/usr/bin/env python3
"""Reject personal home and scratch paths in tracked text files.

Flag names from USER and optional comma-separated ECP5_PRIVATE_NAMES at runtime.
Match /Users/<name>/, /home/<name>/ and -Users-<name>- scratch paths. The
upstream CI homes /Users/runner/ and /home/runner/ are explicitly allowed.
Exit 1 when any tracked text line contains a private path, otherwise 0.
"""
from pathlib import Path
import os
import re
import subprocess


def main():
    root = Path(subprocess.check_output(
        ["git", "rev-parse", "--show-toplevel"], text=True).strip())
    names = subprocess.check_output(["git", "-C", str(root), "ls-files", "-z"])
    private_names = {
        os.fsencode(name.strip())
        for name in [os.environ.get("USER", ""),
                     *os.environ.get("ECP5_PRIVATE_NAMES", "").split(",")]
        if name.strip()
    }
    # Extract names generically; only configured identities are private.
    # This leaves synthetic test identities and upstream CAD authors alone.
    home = re.compile(rb"/(?:Users|home)/([^/\s\"'`<>]+)(?:/|(?=[\s\"'`<>]|$))")
    scratch = re.compile(rb"-Users-([^/\s\"'`<>]+)")
    failures = 0
    for name in names.split(b"\0"):
        if not name:
            continue
        path = root / name.decode("utf-8", errors="surrogateescape")
        if not path.is_file() or path.is_symlink():
            continue
        data = path.read_bytes()
        if b"\0" in data:
            continue
        for number, line in enumerate(data.splitlines(), 1):
            private_home = any(
                match[1] in private_names and match[1] != b"runner"
                for match in home.finditer(line)
            )
            private_scratch = any(
                match[1].startswith(name + b"-")
                for match in scratch.finditer(line) for name in private_names
            )
            if private_home or private_scratch:
                print(f"{path.relative_to(root)}:{number}: {line.decode('utf-8', errors='replace')}")
                failures += 1
    if failures:
        print(f"Found personal home paths on {failures} tracked text line(s).")
    return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
