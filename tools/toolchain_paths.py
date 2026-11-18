"""Portable discovery for collectors and the native image-reader builder.

Explicit options and environment overrides are authoritative: invalid overrides
fail instead of silently selecting a different installation.
"""
import os
from functools import lru_cache
from pathlib import Path
import shutil
import subprocess


@lru_cache(maxsize=1)
def install_prefixes():
    brew = shutil.which("brew")
    prefixes = []
    if brew:
        result = subprocess.run([brew, "--prefix"], capture_output=True, text=True, check=False)
        if result.returncode == 0 and result.stdout.strip():
            prefixes.append(Path(result.stdout.strip()))
    return list(dict.fromkeys(prefixes + [Path("/opt/homebrew"), Path("/usr/local")]))


def _find(explicit, env, candidates, valid, hint):
    override = explicit if explicit is not None else os.environ.get(env)
    paths = [Path(override).expanduser()] if override is not None else candidates
    for path in paths:
        path = path.expanduser()
        if valid(path):
            return path.resolve()
    detail = f" (invalid path: {override})" if override is not None else ""
    raise ValueError(f"{hint}{detail}")


def discover_trellis(explicit=None):
    candidates = []
    if os.environ.get("ECP5_DB"):
        candidates.append(Path(os.environ["ECP5_DB"]).expanduser().parent)
    candidates += [Path.home() / "FPGA/prjtrellis", Path("/usr/local/share/trellis")]
    candidates += [p / "share/trellis" for p in install_prefixes()]
    return _find(explicit, "TRELLIS_ROOT", candidates,
                 lambda p: (p / "libtrellis/include/Chip.hpp").is_file(),
                 "prjtrellis source not found; use --trellis PATH or TRELLIS_ROOT "
                 "(or ECP5_DB pointing into a source checkout)")


def discover_nextpnr(explicit=None):
    candidates = [Path.home() / "FPGA/nextpnr", Path("/usr/local/share/nextpnr")]
    candidates += [p / "share/nextpnr" for p in install_prefixes()]
    return _find(explicit, "NEXTPNR_ROOT", candidates,
                 lambda p: (p / "ecp5/bitstream.cc").is_file(),
                 "nextpnr source not found; use --nextpnr/--nextpnr-source PATH or NEXTPNR_ROOT")


def discover_database(explicit=None, trellis=None):
    candidates = []
    if trellis is not None:
        candidates.append(trellis / "database")
    if os.environ.get("TRELLIS_ROOT"):
        candidates.append(Path(os.environ["TRELLIS_ROOT"]) / "database")
    candidates.append(Path.home() / "FPGA/prjtrellis/database")
    for prefix in [Path("/usr/local")] + install_prefixes():
        candidates += [prefix / "share/trellis/database", prefix / "share/trellis"]
    return _find(explicit, "ECP5_DB", candidates,
                 lambda p: (p / "ECP5/LFE5U-85F/tilegrid.json").is_file(),
                 "Trellis database not found; use --database PATH or ECP5_DB (or TRELLIS_ROOT)")


def trellis_library(prefix):
    for directory in ("lib/trellis", "lib", "lib64/trellis", "lib64"):
        for name in ("libtrellis.dylib", "libtrellis.so"):
            path = prefix / directory / name
            if path.is_file():
                return path
    return None


def discover_prefix(explicit=None):
    candidates = [Path.home() / "FPGA/trellis-install", Path.home() / "FPGA/prjtrellis"]
    candidates += install_prefixes()
    return _find(explicit, "TRELLIS_PREFIX", candidates, trellis_library,
                 "libtrellis library not found; use --prefix PATH or TRELLIS_PREFIX")


def resolve_toolchain(parser, args):
    """Resolve only options declared by this command, after --help is handled."""
    try:
        for name, discover in (("trellis", discover_trellis), ("nextpnr", discover_nextpnr),
                               ("nextpnr_source", discover_nextpnr), ("database", discover_database),
                               ("prefix", discover_prefix)):
            if hasattr(args, name):
                if name == "database":
                    path = discover(getattr(args, name), getattr(args, "trellis", None))
                else:
                    path = discover(getattr(args, name))
                setattr(args, name, path)
    except ValueError as error:
        hint = ""
        if hasattr(args, "prefix"):
            hint = "; configure --trellis/TRELLIS_ROOT (or ECP5_DB) and --prefix/TRELLIS_PREFIX"
        parser.error(str(error) + hint)
    return args


def portable_path(path):
    """Keep repository records independent of the current home directory."""
    path = Path(path).resolve()
    repo = Path(__file__).resolve().parents[1]
    if path.is_relative_to(repo):
        return path.relative_to(repo).as_posix()
    home = Path.home().resolve()
    if path.is_relative_to(home):
        return "~/" + path.relative_to(home).as_posix()
    return str(path)
