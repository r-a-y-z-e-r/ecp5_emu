#!/usr/bin/env python3
"""Copy the complete non-system Mach-O closure, rewrite references, sign, audit."""
import argparse
from pathlib import Path
import re
import shutil
import subprocess
import sys


def output(*args):
    return subprocess.check_output(args, text=True)


def dependencies(path):
    return [line.strip().split(" (compatibility")[0]
            for line in output("otool", "-L", str(path)).splitlines()[1:]]


def rpaths(path):
    return re.findall(r"cmd LC_RPATH\n\s+cmdsize \d+\n\s+path (.*?) \(offset",
                      output("otool", "-l", str(path)))


def system(path):
    return path.startswith(("/System/Library/", "/usr/lib/"))


def resolve(dependency, source, executable):
    def expand(value):
        return value.replace("@loader_path", str(source.parent)).replace(
            "@executable_path", str(executable.parent))
    if dependency.startswith("@rpath/"):
        for path in rpaths(source) + rpaths(executable):
            candidate = Path(expand(path)) / dependency[len("@rpath/"):]
            if candidate.is_file():
                return candidate.resolve()
    else:
        candidate = Path(expand(dependency))
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f"cannot resolve {dependency} from {source}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("app", type=Path)
    ap.add_argument("--check-build-paths", type=Path, metavar="CHECKOUT",
                    help="check the fully bundled payload before signing nested Mach-O files")
    args = ap.parse_args()
    contents = args.app.resolve() / "Contents"
    libraries = contents / "Frameworks"
    libraries.mkdir()
    binaries = [contents / "MacOS" / name for name in
                ["ecp5_emu", "ecp5_gui", "trellis_image_reader"]]
    reader = binaries[-1]
    copied = {}

    def bundle(source, destination):
        # Discover using the ORIGINAL file's rpaths, before changing its copy.
        changes = []
        for dependency in dependencies(source):
            if system(dependency):
                continue
            resolved = resolve(dependency, source, reader)
            name = resolved.name
            target = libraries / name
            if name in copied and copied[name] != resolved:
                raise RuntimeError(f"dylib basename collision: {name}")
            if name not in copied:
                copied[name] = resolved
                shutil.copy2(resolved, target)
                target.chmod(0o755)
                bundle(resolved, target)
            changes.append((dependency, "@loader_path/" +
                            ("" if destination.parent == libraries else "../Frameworks/") + name))
        for old, new in changes:
            subprocess.run(["install_name_tool", "-change", old, new, str(destination)], check=True)
        if destination.parent == libraries:
            subprocess.run(["install_name_tool", "-id", "@rpath/" + destination.name,
                            str(destination)], check=True)
        for path in set(rpaths(destination)):
            subprocess.run(["install_name_tool", "-delete_rpath", path, str(destination)], check=True)

    for binary in binaries:
        bundle(binary, binary)
    if args.check_build_paths is not None:
        subprocess.run([sys.executable, str(Path(__file__).parents[1] / 'check_build_paths.py'),
                        str(args.app), '--root', str(args.check_build_paths)], check=True)
    for path in sorted(libraries.iterdir()) + binaries:
        subprocess.run(["codesign", "--force", "-s", "-", str(path)], check=True)
        for dependency in dependencies(path):
            if system(dependency):
                continue
            if dependency == "@rpath/" + path.name and path.parent == libraries:
                continue  # the dylib's own install ID
            if not dependency.startswith("@loader_path/"):
                raise RuntimeError(f"external dependency in {path}: {dependency}")
            resolved = (path.parent / dependency[len("@loader_path/"):]).resolve()
            if not resolved.is_relative_to(contents) or not resolved.is_file():
                raise RuntimeError(f"dependency escapes bundle: {dependency}")
        if rpaths(path):
            raise RuntimeError(f"unexpected rpath in {path}")
        print(output("otool", "-L", str(path)), end="")


if __name__ == "__main__":
    main()
