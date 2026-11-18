#!/usr/bin/env python3
"""Stage only verified runtime inputs and release notices into a resource dir."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import notices

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "docs/inventory/LFE5U-85F"


def find_board_model(candidates):
    lfs_prefix = b"version https://git-lfs.github.com/spec/v1"
    for path in candidates:
        if not path.is_file():
            continue
        with path.open("rb") as stream:
            if stream.read(len(lfs_prefix)) == lfs_prefix:
                print(f"WARNING: skipping git LFS pointer {path}; run `git lfs pull`.",
                      file=sys.stderr)
                continue
        return path
    return None


def verified_copy(source, destination, records):
    for record in records:
        relative = Path(record["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe manifest path: {relative}")
        path = source / relative
        if hashlib.sha256(path.read_bytes()).hexdigest() != record["sha256"]:
            raise ValueError(f"SHA-256 mismatch: {path}")
        dest = destination / relative
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--database", type=Path, required=True)
    ap.add_argument("--trellis", type=Path, default=None)
    ap.add_argument("--destination", type=Path, required=True)
    ap.add_argument("--target", required=True)
    args = ap.parse_args()
    dest = args.destination
    dest.mkdir(parents=True, exist_ok=True)
    architecture = dest / "architecture"
    architecture.mkdir()
    for name in ["model_manifest.json", "configuration_manifest.json"]:
        manifest = json.loads((EVIDENCE / name).read_text())
        verified_copy(EVIDENCE, architecture, manifest["files"])
        shutil.copy2(EVIDENCE / name, architecture / name)
    inventory = json.loads((architecture / "device_inventory.json").read_text())
    verified_copy(args.database, dest / "database", inventory["source_files"])
    db_copying = notices.resolve_trellis_db_copying(args.database)
    shutil.copy2(db_copying, dest / "database/COPYING")
    # git-common-dir points at the main checkout's .git for linked worktrees;
    # a copy without .git (a build host, an unpacked archive) has no main checkout.
    main_out = []
    try:
        common = Path(subprocess.check_output(
            ["git", "rev-parse", "--git-common-dir"], cwd=ROOT, text=True,
            stderr=subprocess.DEVNULL).strip())
        if not common.is_absolute():
            common = ROOT / common
        main_out = [common.resolve().parent / "out/ulx3s_3d/ulx3s_v3.0.8.glb"]
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    override = os.environ.get("ECP5_BOARD_MODEL")
    candidates = ([Path(override)] if override else []) + [
        ROOT / "out/ulx3s_3d/ulx3s_v3.0.8.glb", *main_out,
        ROOT / "assets/ulx3s_3d/ulx3s_v3.0.8.glb"]
    model = find_board_model(candidates)
    if override and not Path(override).is_file():
        raise FileNotFoundError(override)
    if model:
        shutil.copy2(model, dest / "ulx3s_v3.0.8.glb")
        print(f"Board model: {model}")
    else:
        print("WARNING: no board .glb found; board rendering requires ECP5_BOARD_MODEL")
    with (dest / "THIRD_PARTY_NOTICES.txt").open("w") as stream:
        for package in ["ecp5_emu", "ecp5_gui"]:
            output = ROOT / "target/packaging" / f"THIRD_PARTY_NOTICES-{package}.txt"
            subprocess.run(["python3", str(ROOT / "packaging/third_party.py"),
                            "--package", package, "--target", args.target,
                            "--output", str(output)], check=True)
            stream.write(output.read_text() + "\n")
        if "apple-darwin" in args.target:
            stream.write(
                "\n--- objc2 crates (madsmtm/objc2): upstream LICENSE.md note ---\n"
                "The objc2 crates are derived from Apple's Xcode SDKs; crates.io omits LICENSE.md.\n"
                '"From reading the license, it is unclear whether distributing derived works '
                'such as these crates are allowed?"\n'
                "Accepted by the project owner on 2026-11-17.\n"
            )
        # A single notices file, not a directory of duplicated licence texts.
        non_rust_notices = [
            ("prjtrellis-db COPYING", db_copying),
            ("prjtrellis COPYING", notices.resolve_trellis_copying(args.trellis)),
            ("boost LICENSE_1_0.txt", notices.resolve_boost_license()),
        ]
        for label, path in non_rust_notices:
            stream.write(f"\n--- {label} ---\n{path.read_text()}\n")
        for relative in ["assets/ulx3s/LICENSE.md", "assets/fonts/OFL-1.1.txt",
                         "assets/fonts/BLENDER_FONT_NOTICE.md"]:
            path = ROOT / relative
            stream.write(f"\n--- {relative} ---\n{path.read_text()}\n")
        notices.append_toolchain_notices(stream, dest / 'toolchain')
        notices.append_examples_notices(stream, dest / 'examples')
    shutil.copy2(ROOT / "packaging/README.md", dest / "README.md")
    print(f"Verified {len(inventory['source_files'])} database files; staged {dest}")


if __name__ == "__main__":
    main()
