#!/usr/bin/env python3
"""Stage the bundled toolchain selected by components.json (no source archive)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

from native import Closure, sign

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = Path(__file__).with_name('components.json')


def load_manifest():
    return json.loads(MANIFEST.read_text())


def zig_input(config, platform):
    if os.environ.get('ZIG_ROOT'):
        source = Path(os.environ['ZIG_ROOT']).resolve(strict=True)
    else:
        record = config['zig']['platforms'][platform]
        cache = ROOT / 'target/packaging'
        cache.mkdir(parents=True, exist_ok=True)
        archive = cache / record['url'].rsplit('/', 1)[1]
        if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != record['sha256']:
            temporary = archive.with_suffix('.download')
            urllib.request.urlretrieve(record['url'], temporary)
            if hashlib.sha256(temporary.read_bytes()).hexdigest() != record['sha256']:
                temporary.unlink()
                raise ValueError('Zig archive SHA-256 mismatch')
            temporary.replace(archive)
        source = cache / archive.name.removesuffix('.tar.xz')
        # Re-extract verified bytes, rather than trusting an older partial extraction.
        if source.exists():
            shutil.rmtree(source)
        with tarfile.open(archive) as stream:
            stream.extractall(cache, filter='data')
    version = subprocess.check_output([str(source / 'zig'), 'version'], text=True).strip()
    if version != config['zig']['version']:
        raise ValueError(f'expected Zig {config["zig"]["version"]}, got {version}')
    return source


def script(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text('#!/bin/sh\nset -eu\n' + text)
    path.chmod(0o755)


def stage_tool_links(suite, top, origins=None):
    """Keep loader-relative helper aliases from lib/ to our staged bin/ wrappers."""
    for source in sorted((suite / 'lib').rglob('*')):
        if not source.is_symlink():
            continue
        target = Path(os.path.abspath(source.parent / os.readlink(source)))
        if not target.is_relative_to(suite / 'bin'):
            continue
        staged_target = top / target.relative_to(suite)
        if not staged_target.is_file():
            raise ValueError(f'suite helper alias {source.relative_to(suite)} targets '
                             f'unstaged wrapper {target.relative_to(suite)}')
        destination = top / source.relative_to(suite)
        destination.parent.mkdir(parents=True, exist_ok=True)
        relative = os.path.relpath(staged_target, destination.parent)
        if origins is not None:
            origins[str(destination.relative_to(top))] = {
                'root': 'suite', 'path': str(source.relative_to(suite)), 'kind': 'symlink'}
        if destination.is_symlink() and os.readlink(destination) == relative:
            continue
        destination.symlink_to(relative)
        print(f'Helper alias: {destination.relative_to(top)} -> {relative}')


def stage_zig(source, top, config, platform, origins):
    dest = top / ('zig-' + config['zig']['version'])
    dest.mkdir()
    for name in ['zig', 'LICENSE']:
        shutil.copy2(source / name, dest / name)
        origins[str((dest / name).relative_to(top))] = {'root': 'zig', 'path': name, 'kind': 'file'}
    # Preserve Zig's compiler support and compiler-rt/std sources conservatively.
    # No C++/sanitizer SDKs, examples/docs or other-platform libc trees are needed by cc.
    for name in config['zig']['support']:
        path = source / 'lib' / name
        target = dest / 'lib' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        if path.is_dir():
            shutil.copytree(path, target, symlinks=True)
        else:
            shutil.copy2(path, target)
        record_zig_origins(target, dest, top, origins)
    for name in config['zig']['platforms'][platform]['libc']:
        target = dest / 'lib/libc' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(source / 'lib/libc' / name, target, symlinks=True)
        record_zig_origins(target, dest, top, origins)
    clang_wrapper(dest, config, platform)


def record_zig_origins(target, dest, top, origins):
    for path in [target, *sorted(target.rglob('*'))]:
        if path.is_symlink() or path.is_file():
            origins[str(path.relative_to(top))] = {
                'root': 'zig', 'path': str(path.relative_to(dest)),
                'kind': 'symlink' if path.is_symlink() else 'file'}


def clang_wrapper(dest, config, platform):
    # Fast mode probes --version in every process, even before a program-cache hit.
    # Record the real compiler identity without a build-machine installation path.
    cache = ROOT / 'target/packaging/zig-version-cache'
    cache.mkdir(parents=True, exist_ok=True)
    environment = dict(os.environ, ZIG_GLOBAL_CACHE_DIR=str(cache),
                       ZIG_LOCAL_CACHE_DIR=str(cache / 'local'))
    version = subprocess.check_output(
        [str(dest / 'zig'), 'cc', '-target', config['zig']['platforms'][platform]['target'], '--version'],
        text=True, env=environment, cwd=cache)
    version = '\n'.join(line for line in version.splitlines() if not line.startswith('InstalledDir:')) + '\n'
    (dest / 'clang-version.txt').write_text(version)
    cache = ('${HOME:?}/Library/Caches' if platform == 'macos'
             else '${XDG_CACHE_HOME:-${HOME:?}/.cache}')
    script(dest / 'clang',
           'if [ "$#" -eq 1 ] && [ "$1" = --version ]; then\n'
           '    cat "$(dirname "$0")/clang-version.txt"\n    exit 0\nfi\n'
           f': "${{ZIG_GLOBAL_CACHE_DIR={cache}/ecp5_emu/zig-{config["zig"]["version"]}}}"\n'
           ': "${ZIG_LOCAL_CACHE_DIR=$ZIG_GLOBAL_CACHE_DIR/local}"\n'
           'export ZIG_GLOBAL_CACHE_DIR ZIG_LOCAL_CACHE_DIR\n'
           'mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"\n'
           # An explicit target prevents Zig's native libc discovery using an installed SDK.
           f'set -- -target "{config["zig"]["platforms"][platform]["target"]}" "$@"\n'
           'exec "$(dirname "$0")/zig" cc "$@"\n')


def record(top, platform):
    records = []
    for path in sorted(top.rglob('*')):
        if path.name == 'bundle-manifest.json' or (path.is_dir() and not path.is_symlink()):
            continue
        relative = str(path.relative_to(top))
        if path.is_symlink():
            if not path.resolve(strict=True).is_relative_to(top.parent):
                raise ValueError(f'link escapes resource root: {relative}')
            records.append({'path': relative, 'symlink': os.readlink(path)})
        else:
            records.append({'path': relative, 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()})
    (top / 'bundle-manifest.json').write_text(json.dumps({
        'platform': platform, 'components_sha256': hashlib.sha256(MANIFEST.read_bytes()).hexdigest(),
        'files': records}, indent=2) + '\n')


def stage(resources, platform):
    manifest = load_manifest()
    config = manifest['packaging']
    suite = Path(os.environ['OSS_CAD_SUITE_ROOT']).resolve(strict=True)
    if (suite / 'VERSION').read_text().strip() != manifest['suite']['version']:
        raise ValueError('OSS_CAD_SUITE_ROOT VERSION must be ' + manifest['suite']['version'])
    zig = zig_input(config, platform)
    resources.mkdir(parents=True, exist_ok=True)
    top = resources.resolve() / 'toolchain'
    top.mkdir()
    closure = Closure(suite, top, platform)
    for name, tool in config['tools'].items():
        closure.copy(suite / tool['path'])
        text = 'top=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)\n'
        text += 'PATH="$top/bin${PATH:+:$PATH}"\nexport PATH\n'
        if name == 'yosys':
            # Even noninteractive -p/-s runs otherwise write HOME/.local/state/yosys/history.
            text += (': "${XDG_STATE_HOME=${TMPDIR:-/tmp}/ecp5_emu_yosys}"\n'
                     'mkdir -p "$XDG_STATE_HOME"\nexport XDG_STATE_HOME\n')
        if tool.get('version_args'):
            text += 'if [ "$#" -eq 1 ] && [ "$1" = --version ]; then\n'
            text += '    set -- ' + ' '.join('"' + a + '"' for a in tool['version_args']) + '\nfi\n'
        if tool.get('version_text'):
            text += 'if [ "$#" -eq 1 ] && [ "$1" = --version ]; then\n'
            text += '    printf "%s\\n" "' + tool['version_text'] + '"\n    exit 0\nfi\n'
        if name in ('nextpnr-ecp5', 'python3'):
            text += ('PYTHONHOME="$top"\nPYTHONNOUSERSITE=1\nPYTHONDONTWRITEBYTECODE=1\n'
                     'export PYTHONHOME PYTHONNOUSERSITE PYTHONDONTWRITEBYTECODE\n')
        if platform == 'linux':
            text += ('exec "$top/lib/ld-linux-x86-64.so.2" --inhibit-cache --inhibit-rpath "" '
                     '--library-path "$top/lib" ')
        else:
            text += 'exec '
        text += f'"$top/{tool["path"]}" "$@"\n'
        script(top / 'bin' / name, text)
    for tree in config['trees']:
        closure.copy(suite / tree['path'])
    stdlib = suite / config['python']['path']
    excludes = config['python']['exclude']
    shutil.copytree(stdlib, top / config['python']['path'], symlinks=True,
                    ignore=shutil.ignore_patterns(*excludes))
    # Include native stdlib modules as closure roots; exclude test/Tk modules too.
    for path in (top / config['python']['path']).rglob('*'):
        if path.is_file():
            closure.copy(suite / path.relative_to(top))
        elif path.is_symlink():
            relative = str(path.relative_to(top))
            closure.origins[relative] = {'root': 'suite', 'path': relative, 'kind': 'symlink'}
    for path in config.get('extra_roots', {}).get(platform, []):
        closure.copy(suite / path)
    closure.collect()
    if platform == 'linux':
        stage_tool_links(suite, top, closure.origins)
    link = top / 'share/trellis/database'
    link.parent.mkdir(parents=True, exist_ok=True)
    link.symlink_to('../../../database')
    origins = closure.origins
    stage_zig(zig, top, config, platform, origins)
    for name in config['suite_reports']:
        source = suite / 'license' / ('LICENSE.' + name)
        target = top / 'license' / source.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        origins[str(target.relative_to(top))] = {
            'root': 'suite', 'path': str(source.relative_to(suite)), 'kind': 'file'}
    (top / 'source-origins.json').write_text(json.dumps({
        'version': 1, 'zig_directory': zig.name, 'files': origins}, indent=2) + '\n')
    # Sourced by the launchers; their CONTENTS/ROOT locates this package without PATH changes.
    resource_root = '$CONTENTS/Resources' if platform == 'macos' else '$ROOT/share'
    environment = f'toolchain="{resource_root}/toolchain"\n'
    overrides = {'ECP5_YOSYS': 'bin/yosys', 'ECP5_NEXTPNR': 'bin/nextpnr-ecp5',
                 'ECP5_ECPPACK': 'bin/ecppack', 'ECP5_ECPBRAM': 'bin/ecpbram',
                 'ECP5_OPENFPGALOADER': 'bin/openFPGALoader', 'ECP5_PYTHON': 'bin/python3',
                 'ECP5_CLANG': 'zig-' + config['zig']['version'] + '/clang',
                 'ECP5_OSS_CAD_SUITE': ''}
    for name, path in overrides.items():
        environment += f': "${{{name}=$toolchain{("/" + path) if path else ""}}}"\nexport {name}\n'
    (top / 'environment.sh').write_text(environment + 'unset toolchain\n')
    # Keep a copy of the selection/audit for the later corresponding-source job.
    shutil.copy2(MANIFEST, top / 'components.json')
    (top / 'VERSION').write_text(manifest['suite']['version'] + '\n')
    print(f'Staged suite {manifest["suite"]["version"]} and Zig {config["zig"]["version"]}')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('resources', type=Path)
    ap.add_argument('--platform', choices=['macos', 'linux'], required=True)
    ap.add_argument('--finalize', action='store_true', help='sign nested Mach-O and record final hashes')
    args = ap.parse_args()
    if args.finalize:
        top = args.resources.resolve() / 'toolchain'
        if args.platform == 'macos':
            sign(top)
        record(top, args.platform)
    else:
        stage(args.resources, args.platform)


if __name__ == '__main__':
    main()
