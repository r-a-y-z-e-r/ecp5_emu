#!/usr/bin/env python3
"""Generate release notices from the locked, target-specific Cargo dependency set.

Includes normal dependencies and build/proc-macro dependencies for completeness;
excludes dev-only dependencies. Run once per binary. No generated notices belong
in git. Default features match the release scripts (no extra features).
"""
import argparse
import collections
import json
from pathlib import Path
import re
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import notices

ROOT = Path(__file__).resolve().parents[1]

# Some published crates omit their workspace's root licence documents. Keep
# pinned upstream URLs + SHA-256 digests in licence_sources.json and gather
# verified copies into target/packaging/notices/ at build time. Never invent a notice.
SUPPLEMENTAL = {
    "https://github.com/emilk/egui": "crate-egui-",
    "https://github.com/AccessKit/accesskit": "crate-accesskit-",
    "https://github.com/madsmtm/objc2": "crate-objc2-",
    "https://github.com/aclysma/profiling": "crate-profiling-",
    "https://github.com/zakarumych/gpu-descriptor": "crate-gpu-descriptor-",
    "https://github.com/gfx-rs/rspirv": "crate-spirv-",
}


def cargo(*args):
    return subprocess.check_output(["cargo", *args], cwd=ROOT, text=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--package", choices=["ecp5_emu", "ecp5_gui"], required=True)
    ap.add_argument("--target", default=None)
    ap.add_argument("--output", type=Path)
    args = ap.parse_args()
    target = args.target or subprocess.check_output(
        ["rustc", "-vV"], text=True).split("host: ")[1].splitlines()[0]
    metadata = json.loads(cargo("metadata", "--locked", "--format-version", "1",
                                "--filter-platform", target))
    # cargo metadata resolves the workspace together. cargo tree selects the
    # chosen binary's feature/dependency closure, excluding test dependencies.
    tree = cargo("tree", "--locked", "-p", args.package, "--target", target,
                 "--edges", "normal,build", "--prefix", "none", "--format", "{p}")
    selected = set(re.findall(r"^([\w-]+) v([^\s]+)", tree, re.MULTILINE))
    packages = sorted((p for p in metadata["packages"]
                       if (p["name"], p["version"]) in selected and p["source"]),
                      key=lambda p: (p["name"], p["version"]))
    out = args.output or ROOT / "target" / "packaging" / f"THIRD_PARTY_NOTICES-{args.package}.txt"
    out.parent.mkdir(parents=True, exist_ok=True)
    counts = collections.Counter()
    flags = []
    missing = []
    with out.open("w") as stream:
        stream.write(f"Third-party Rust crates for {args.package} ({target})\n"
                     "Cargo.lock; default features; normal + build/proc-macro dependencies.\n"
                     "Dev-only crates and other platforms are excluded. Licence expressions\n"
                     "are upstream declarations, not a legal compatibility determination.\n\n")
        for package in packages:
            expression = package["license"] or "UNKNOWN"
            counts[expression] += 1
            if re.search(r"(?:GPL|LGPL|AGPL|MPL|UNKNOWN)", expression):
                flags.append(f"{package['name']} {package['version']}: {expression}")
            stream.write("=" * 78 + "\n")
            stream.write(f"{package['name']} {package['version']}\nLicence: {expression}\n"
                         f"Repository: {package['repository'] or 'not declared'}\n"
                         f"Source: {package['source']}\n")
            source = Path(package["manifest_path"]).parent
            files = set()
            for path in source.rglob("*"):
                if not path.is_file():
                    continue
                name = path.name.lower()
                if (name.startswith(("license", "licence", "copying", "copyright", "notice", "unlicense"))
                        or ("fonts" in path.parts and path.suffix == ".txt")):
                    files.add(path)
            if package.get("license_file"):
                files.add(source / package["license_file"])
            supplemental = []
            if not files:
                for repository, prefix in SUPPLEMENTAL.items():
                    if (package["repository"] or "").startswith(repository):
                        supplemental = notices.ensure_notices(prefix)
                        break
                if package["name"] == "epaint_default_fonts":
                    supplemental.extend(notices.ensure_notices("font-"))
            if not files and not supplemental:
                missing.append(f"{package['name']} {package['version']}")
                stream.write("No licence text shipped in the crate source; review upstream.\n")
            for path in sorted(files):
                stream.write(f"\n--- {path.relative_to(source)} ---\n")
                stream.write(path.read_text(errors="replace") + "\n")
            for path in supplemental:
                stream.write(f"\n--- verified upstream workspace document: {path.name} ---\n")
                stream.write(path.read_text() + "\n")
            stream.write("\n")
        # Also covers the Rust standard library distributed with the toolchain.
        stream.write("Rust standard library/toolchain: https://github.com/rust-lang/rust/tree/1.92.0\n")
        for path in notices.ensure_notices("rust-"):
            stream.write(f"\n--- {path.name} ---\n{path.read_text()}\n")
    ambiguities = []
    if any((p["repository"] or "").startswith("https://github.com/madsmtm/objc2") for p in packages):
        ambiguities.append("objc2: upstream LICENSE.md flags Apple SDK-derived binding rights")
    if any(p["name"] == "hexf-parse" for p in packages):
        ambiguities.append("hexf-parse 0.2.1: Cargo.toml declares CC0-1.0, crate omits LICENSE, upstream master added 0BSD LICENSE")
    if any(p["name"] == "dispatch" and p["version"] == "0.2.0" for p in packages):
        ambiguities.append("dispatch 0.2.0: Cargo.toml declares MIT, no LICENSE file in crate or upstream repo")
    report = {"package": args.package, "target": target, "crates": len(packages),
              "licence_expressions": dict(sorted(counts.items())),
              "review_copyleft_or_unknown": flags, "missing_licence_text": missing,
              "upstream_ambiguities": ambiguities,
              "output": str(out)}
    out.with_suffix(".json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
