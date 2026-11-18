#!/usr/bin/env python3
"""Gather and SHA-256-verify third-party licence texts into target/packaging/notices/."""
from __future__ import annotations

import hashlib
import json
import os
import subprocess
import shutil
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SOURCES_PATH = ROOT / "packaging/licence_sources.json"
NOTICES_DIR = ROOT / "target/packaging/notices"


def load_sources() -> list[dict]:
    """Return pinned primary-source licence records."""
    return json.loads(SOURCES_PATH.read_text())


def _sources_by_file() -> dict[str, dict]:
    return {record["file"]: record for record in load_sources()}


def verify_sha256(data: bytes, expected_sha256: str, label: str) -> bytes:
    """Verify downloaded bytes against a pinned SHA-256 digest."""
    if not expected_sha256:
        raise ValueError(f"missing pinned sha256 for {label}")
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected_sha256:
        raise ValueError(
            f"SHA-256 mismatch for {label}: expected {expected_sha256}, got {actual}"
        )
    return data


def fetch_verified(record: dict, dest_dir: Path = NOTICES_DIR) -> Path:
    """Return a verified notice path under dest_dir, downloading if not cached."""
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / record["file"]
    expected = record["sha256"]
    if dest.is_file() and hashlib.sha256(dest.read_bytes()).hexdigest() == expected:
        return dest
    data = urllib.request.urlopen(record["raw_url"], timeout=60).read()
    if not data or b"<!DOCTYPE html" in data[:100]:
        raise ValueError(f"not a licence text: {record['raw_url']}")
    verify_sha256(data, expected, f"{record['file']} ({record['raw_url']})")
    dest.write_bytes(data)
    return dest


def ensure_notices(
    prefix_or_names: str | list[str], dest_dir: Path = NOTICES_DIR
) -> list[Path]:
    """Fetch and verify all pinned notices matching a filename prefix or list of names."""
    records = load_sources()
    if isinstance(prefix_or_names, str):
        selected = [r for r in records if r["file"].startswith(prefix_or_names)]
    else:
        wanted = set(prefix_or_names)
        selected = [r for r in records if r["file"] in wanted]
        missing = wanted - {r["file"] for r in selected}
        if missing:
            raise KeyError(f"unknown licence_sources.json entries: {sorted(missing)}")
    if not selected:
        raise KeyError(f"no licence_sources.json entries match {prefix_or_names!r}")
    return [fetch_verified(r, dest_dir) for r in sorted(selected, key=lambda r: r["file"])]


def resolve_trellis_db_copying(database: Path | None = None) -> Path:
    """Prefer COPYING from the local database checkout; fall back to pinned download."""
    if database is not None:
        local = database / "COPYING"
        if local.is_file():
            return local
    return fetch_verified(_sources_by_file()["trellis-db-CC0.txt"])


def resolve_trellis_copying(trellis: Path | None = None) -> Path:
    """Prefer COPYING from a local prjtrellis checkout; fall back to pinned download."""
    env = os.environ.get("ECP5_TRELLIS_SOURCE")
    candidates = ([trellis] if trellis else []) + ([Path(env)] if env else []) + [
        ROOT / "target/packaging/prjtrellis",
        Path.home() / "FPGA/prjtrellis",
    ]
    for root in candidates:
        local = root / "COPYING"
        if local.is_file():
            return local
    return fetch_verified(_sources_by_file()["trellis-ISC.txt"])


def resolve_boost_license() -> Path:
    """Prefer a local Boost LICENSE_1_0.txt if installed; fall back to pinned download."""
    candidates = [
        os.environ.get("BOOST_ROOT"),
        os.environ.get("Boost_DIR"),
    ]
    # Homebrew's boost, wherever Homebrew lives.
    brew = shutil.which("brew")
    if brew:
        prefix = subprocess.run([brew, "--prefix", "boost"], capture_output=True, text=True)
        if prefix.returncode == 0:
            candidates.append(prefix.stdout.strip())
    for base in candidates:
        if not base:
            continue
        for rel in ("LICENSE_1_0.txt", "share/doc/boost/LICENSE_1_0.txt"):
            local = Path(base) / rel
            if local.is_file():
                return local
    return fetch_verified(_sources_by_file()["boost-BSL-1.0.txt"])


def picorv32_header_notice() -> str:
    """Extract the ISC licence header from the vendored picorv32.v source."""
    text = (ROOT / "examples/ulx3s/picorv32_console/picorv32.v").read_text()
    end = text.index("*/") + 2
    return text[:end]


def append_toolchain_notices(stream, toolchain: Path) -> None:
    """Append suite/runtime licences and the shared relinking/exception notice."""
    manifest = json.loads((toolchain / 'components.json').read_text())
    config = manifest['packaging']
    stream.write(
        '\n--- Bundled toolchain ---\n'
        'OSS CAD Suite ' + manifest['suite']['version'] + '; Zig ' + config['zig']['version'] + '.\n'
        'Corresponding sources are published beside the release in separate archives.\n'
        'See toolchain/components.json for component versions, source provenance and unresolved audit items.\n'
    )
    paths = [toolchain / 'license' / ('LICENSE.' + n) for n in config['suite_reports']]
    zig = toolchain / ('zig-' + config['zig']['version'])
    paths.append(zig / 'LICENSE')
    for tree in [toolchain / 'lib/ghdl', zig / 'lib']:
        paths += sorted(p for p in tree.rglob('*') if p.is_file() and
                        p.name.upper().startswith(('LICENSE', 'COPYING', 'COPYRIGHT')))
    for path in paths:
        stream.write(f'\n--- {path.relative_to(toolchain)} ---\n{path.read_text()}\n')
    sources = _sources_by_file()
    for path in ensure_notices(config['notice_files']):
        text = path.read_text()
        extraction = sources[path.name].get('notice_extract')
        if extraction == 'leading-c-comment':
            if not text.startswith('/*') or '*/' not in text:
                raise ValueError(f'missing leading licence comment: {path.name}')
            text = text[:text.index('*/') + 2]
        elif extraction == 'leading-ada-comments':
            text = text.split('\npackage ', 1)[0]
            if not text.startswith('--') or 'special exception' not in text:
                raise ValueError(f'missing GRT exception notice: {path.name}')
        stream.write(f'\n--- {path.name} ---\n{text}\n')
    stream.write('\n' + (ROOT / 'packaging/toolchain/THIRD_PARTY_NOTICES-addition.txt').read_text())


def append_examples_notices(stream, examples: Path) -> None:
    """Use the staged archive, so the notices describe precisely the shipped sources."""
    stream.write(
        '\n--- Bundled examples ---\n'
        'The tracked examples/ tree is included at Contents/Resources/examples/ on macOS '
        'and share/examples/ in the Linux package. File hashes/links are in the resource directory\'s '
        'examples-manifest.json.\n'
        'Project RTL, firmware, scripts and original terminal font: project Apache-2.0 LICENSE.\n'
        'ULX3S pin constraints (*.lpf): assets/ulx3s/LICENSE.md reproduced above.\n'
        'PicoRV32 (including shared symlinks in picorv32_flash, picorv32_sdram, riscv_shell and '
        'riscv_terminal): ISC notice below and in picorv32_console/picorv32.v.\n'
        'SERV: ISC notice below and gallery/serv_shell/serv/LICENSE.\n'
        'SERV compressed decoder: Apache-2.0; lowRISC/ETH/University of Bologna attribution '
        'retained below and in gallery/serv_shell/serv/serv_compdec.v.\n'
    )
    for label, relative in [
        ('PicoRV32 ISC header', 'ulx3s/picorv32_console/picorv32.v'),
        ('SERV ISC licence', 'gallery/serv_shell/serv/LICENSE'),
        ('SERV compressed decoder Apache-2.0 header', 'gallery/serv_shell/serv/serv_compdec.v'),
    ]:
        text = (examples / relative).read_text()
        if relative.endswith('.v'):
            if not text.startswith('/*') or '*/' not in text:
                raise ValueError(f'missing licence header: examples/{relative}')
            text = text[:text.index('*/') + 2]
        stream.write(f'\n--- {label}: examples/{relative} ---\n{text}\n')
    stream.write('\n--- Apache-2.0 (project examples and SERV compressed decoder) ---\n'
                 + (ROOT / 'LICENSE').read_text() + '\n')
