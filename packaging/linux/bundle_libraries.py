#!/usr/bin/env python3
"""Bundle Trellis/Boost's ELF closure; keep the platform C/C++ runtime on host."""
import argparse
from pathlib import Path
import re
import shutil
import subprocess

HOST = re.compile(r"^(?:linux-vdso|ld-linux|lib(?:c|m|pthread|dl|rt|stdc\+\+|gcc_s))[-.]")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("directory", type=Path)
    args = ap.parse_args()
    root = args.directory.resolve()
    libraries = root / "lib"
    libraries.mkdir()
    pending = [root / "libexec/trellis_image_reader"]
    seen = {}
    index = 0
    while index < len(pending):
        binary = pending[index]
        index += 1
        text = subprocess.check_output(["ldd", str(binary)], text=True)
        if "not found" in text:
            raise RuntimeError(text)
        for name, source in re.findall(r"^\s*(\S+) => (/\S+) ", text, re.MULTILINE):
            if HOST.match(name):
                continue
            source = Path(source).resolve()
            if name in seen and seen[name] != source:
                raise RuntimeError(f"ELF soname collision: {name}")
            if name not in seen:
                seen[name] = source
                destination = libraries / name
                shutil.copy2(source, destination)
                destination.chmod(0o755)
                # Traverse original files before replacing their rpaths.
                pending.append(source)
    for name in seen:
        subprocess.run(["patchelf", "--set-rpath", "$ORIGIN", str(libraries / name)], check=True)
    subprocess.run(["patchelf", "--set-rpath", "$ORIGIN/../lib",
                    str(root / "libexec/trellis_image_reader")], check=True)
    for binary in [root / "libexec/trellis_image_reader", *sorted(libraries.iterdir())]:
        text = subprocess.check_output(["ldd", str(binary)], text=True)
        if "not found" in text:
            raise RuntimeError(text)
        for name, source in re.findall(r"^\s*(\S+) => (/\S+) ", text, re.MULTILINE):
            if not HOST.match(name) and not Path(source).resolve().is_relative_to(root):
                raise RuntimeError(f"external runtime library: {name}: {source}")
        print(f"{binary}:\n{text}")


if __name__ == "__main__":
    main()
