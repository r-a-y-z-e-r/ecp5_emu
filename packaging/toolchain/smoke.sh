#!/usr/bin/env bash
# Relocated, empty-environment integration check. Accept an .app or unpacked Linux root.
set -euo pipefail
[[ $# -ge 1 ]] || { echo "Usage: $0 <package root> [--showcase] [--no-sandbox]" >&2; exit 2; }
PACKAGE=$(cd "$1" && pwd)
shift
SHOWCASE=
USE_SANDBOX=1
for option in "$@"; do
    case "$option" in
        --showcase) SHOWCASE=--showcase ;;
        --no-sandbox) USE_SANDBOX=0 ;;
        *) echo "Unknown smoke option: $option" >&2; exit 2 ;;
    esac
done
HOST_USER_ROOT=$(dirname "${HOME:?}")
# Keep scratch paths space-free for comparison with the previous packaging run.
# The relocated application/toolchain path deliberately contains spaces.
WORK=$(mktemp -d /tmp/ecp5-smoke.XXXXXXXX)
finish() {
    status=$?
    trap - EXIT
    echo 'Created under HOME:'
    (cd "$WORK/home" && find . -print | LC_ALL=C sort) | sed 's,^\.,HOME,'
    echo 'Created under TMPDIR:'
    (cd "$WORK/tmp" && find . -print | LC_ALL=C sort) | sed 's,^\.,TMPDIR,'
    rm -rf "$WORK"
    exit "$status"
}
trap finish EXIT
mkdir -p "$WORK/home" "$WORK/tmp" "$WORK/package with spaces"
cd "$WORK" # Every runtime check starts outside the checkout and its ancestors.
cp -R "$PACKAGE" "$WORK/package with spaces/"
COPY="$WORK/package with spaces/${PACKAGE##*/}"
SANDBOX=()
if [[ -d "$COPY/Contents/Resources" ]]; then
    RESOURCES="$COPY/Contents/Resources"
    CLI="$RESOURCES/bin/ecp5_emu"
    ZIG_CACHE="$WORK/home/Library/Caches/ecp5_emu/zig-0.17.1"
    EXAMPLES="$RESOURCES/examples"
    [[ ! -e "$COPY/Contents/examples" ]] || { echo 'Obsolete Contents/examples directory remains' >&2; exit 1; }
    if [[ "$USE_SANDBOX" == 1 ]]; then
        [[ "$HOST_USER_ROOT" != / ]] || { echo 'Cannot sandbox the filesystem root as user folders' >&2; exit 1; }
    cat > "$WORK/sandbox.sb" <<'SB'
(version 1)
(allow default)
(deny file-read* file-write* (subpath (param "USER_ROOT")))
(deny network*)
SB
        SANDBOX=(/usr/bin/sandbox-exec -D "USER_ROOT=$HOST_USER_ROOT" -f "$WORK/sandbox.sb")
        echo 'macOS runtime sandbox: user folders and network denied'
    else
        echo 'macOS runtime sandbox: explicitly disabled; user-folder/network denial NOT VERIFIED'
    fi
else
    RESOURCES="$COPY/share"
    CLI="$COPY/bin/ecp5_emu"
    # clean() leaves XDG_CACHE_HOME unset, so the Linux default is HOME/.cache.
    ZIG_CACHE="$WORK/home/.cache/ecp5_emu/zig-0.17.1"
    EXAMPLES="$RESOURCES/examples"
    [[ ! -e "$COPY/examples" ]] || { echo 'Obsolete root examples directory remains' >&2; exit 1; }
fi
TOP="$RESOURCES/toolchain"
FAST_CACHE="$WORK/tmp/ecp5_emu_fast" # src/fast.rs: std::env::temp_dir().join(...)
clean() {
    if [[ "$USE_SANDBOX" == 1 && -d "$COPY/Contents" ]]; then
        env -i PATH=/usr/bin:/bin HOME="$WORK/home" TMPDIR="$WORK/tmp" "${SANDBOX[@]}" "$@"
    else
        env -i PATH=/usr/bin:/bin HOME="$WORK/home" TMPDIR="$WORK/tmp" "$@"
    fi
}
echo "Asserting packaged examples location: $EXAMPLES"
[[ -f "$EXAMPLES/ulx3s/README.md" && -f "$EXAMPLES/gallery/README.md" && -f "$EXAMPLES/showcase.toml" ]] || {
    echo 'Missing packaged examples or group README markers at the resource location' >&2; exit 1;
}
if [[ -d "$COPY/Contents" ]]; then
    [[ ! -e "$TOP/lib/yosys-abc" ]] || { echo 'Unexpected Linux ABC alias in macOS package' >&2; exit 1; }
else
    [[ -L "$TOP/lib/yosys-abc" && "$TOP/lib/yosys-abc" -ef "$TOP/bin/yosys-abc" ]] || {
        echo 'Missing loader-relative lib/yosys-abc alias to staged bin/yosys-abc' >&2; exit 1;
    }
fi
clean "$TOP/bin/python3" - "$EXAMPLES" "$RESOURCES/examples-manifest.json" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import sys
root = Path(sys.argv[1])
manifest = json.loads(Path(sys.argv[2]).read_text())
records = manifest['files']
actual = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file() or p.is_symlink()}
assert actual == {r['path'] for r in records}, 'examples differ from archive manifest'
for record in records:
    path = root / record['path']
    if 'symlink' in record:
        assert path.is_symlink() and os.readlink(path) == record['symlink']
        assert not os.path.isabs(record['symlink'])
        assert path.resolve(strict=True).is_relative_to(root.resolve())
    else:
        assert hashlib.sha256(path.read_bytes()).hexdigest() == record['sha256']
file_bytes = sum((root / r['path']).stat().st_size for r in records if 'sha256' in r)
symlink_bytes = sum(len(os.fsencode(r['symlink'])) for r in records if 'symlink' in r)
assert manifest['size'] == {'entries': len(records), 'files': sum('sha256' in r for r in records),
                            'symlinks': sum('symlink' in r for r in records), 'file_bytes': file_bytes,
                            'symlink_bytes': symlink_bytes, 'bytes': file_bytes + symlink_bytes}
print(f'Packaged examples: {len(records)} tracked entries, {file_bytes} file bytes '
      f'+ {symlink_bytes} link-target bytes verified at {root}')
PY
mkdir -p "$WORK/invalid-examples/ulx3s" "$WORK/invalid-examples/gallery"
if clean env ECP5_EXAMPLES="$WORK/invalid-examples" "$CLI" new "$WORK/rejected-override" \
    --from-demo gallery/led_tennis > "$WORK/invalid-examples.log" 2>&1; then
    echo 'Invalid ECP5_EXAMPLES unexpectedly fell back to bundled examples' >&2; exit 1
fi
grep -Fq 'ECP5_EXAMPLES is set but invalid (no fallback)' "$WORK/invalid-examples.log"
[[ ! -e "$WORK/rejected-override" ]]
echo 'Expected failure: invalid ECP5_EXAMPLES (missing group README markers), no fallback: PASS'
mv "$EXAMPLES" "$WORK/hidden-examples"
if clean "$CLI" new "$WORK/rejected-missing-examples" --from-demo gallery/led_tennis \
    > "$WORK/missing-examples.log" 2>&1; then
    mv "$WORK/hidden-examples" "$EXAMPLES"
    echo 'Release executable unexpectedly found checkout examples after packaged tree was hidden' >&2; exit 1
fi
mv "$WORK/hidden-examples" "$EXAMPLES"
grep -Fq 'Demo sources unavailable' "$WORK/missing-examples.log"
[[ ! -e "$WORK/rejected-missing-examples" ]]
echo 'Expected failure: packaged examples hidden, no release compile-time fallback: PASS'
cat > "$WORK/check-caches.py" <<'PY'
import hashlib
import json
import os
from pathlib import Path
import sys

zig, fast, state = map(Path, sys.argv[1:4])
home = Path(os.environ['HOME']).resolve()
zig, fast = zig.resolve(), fast.resolve()
assert zig.is_dir(), f'missing app Zig cache: {zig}'
assert fast == Path(os.environ['TMPDIR']).resolve() / 'ecp5_emu_fast'
assert fast.is_dir(), f'missing fast program cache: {fast}'
assert not (home / '.cache/zig').exists(), 'unexpected default Zig cache under HOME'
for path in home.rglob('*'):
    # Only the app cache and its necessary parent directories may exist in HOME.
    assert path.is_relative_to(zig) or (not path.is_symlink() and path.is_dir() and path in zig.parents), (
        f'unexpected HOME write: {path.relative_to(home)}')
    assert path.resolve().is_relative_to(zig) or path in zig.parents, f'cache link escapes: {path}'


def snapshot(root):
    result = {}
    for path in [root, *sorted(root.rglob('*'))]:
        stat = path.lstat()
        entry = {'mtime_ns': stat.st_mtime_ns, 'inode': stat.st_ino, 'size': stat.st_size}
        if path.is_symlink():
            entry['symlink'] = os.readlink(path)
        elif path.is_file():
            entry['sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
        result[str(path.relative_to(root))] = entry
    return result


libraries = sorted(fast.glob('*.so'))
assert libraries, 'no compiled fast library under TMPDIR/ecp5_emu_fast'
current = {'zig': snapshot(zig), 'fast': snapshot(fast)}
if state.exists():
    previous = json.loads(state.read_text())
    assert current['fast'] == previous['fast'], 'fast cache/library changed on warm run'
    assert current['zig'] == previous['zig'], 'Zig cache entries changed on warm run'
    print('Warm cache hit: library inode/mtime/hash unchanged; no new or changed Zig cache entries')
else:
    log = Path(os.environ['TMPDIR']).parent / 'compiler-invocations.log'
    calls = log.read_text().splitlines()
    assert any(line.startswith('clang\t') for line in calls), 'cold run did not invoke clang'
    zig_calls = [line.split('\t') for line in calls if line.startswith('zig\t')]
    assert zig_calls, 'cold run did not invoke bundled Zig'
    for _, global_cache, local_cache in zig_calls:
        assert Path(global_cache).resolve() == zig, f'wrong Zig global cache: {global_cache}'
        assert Path(local_cache).resolve() == zig / 'local', f'wrong Zig local cache: {local_cache}'
    state.write_text(json.dumps(current, sort_keys=True))
    print(f'Zig caches: {zig} (global), {zig / "local"} (local); no other HOME writes')
    print(f'Fast program cache: {fast}; libraries: {", ".join(p.name for p in libraries)}')
PY
for TOOL in yosys yosys-abc nextpnr-ecp5 ecppack ecpbram openFPGALoader python3; do
    clean "$TOP/bin/$TOOL" --version
done
clean "$TOP/bin/ecpbram" --help > "$WORK/ecpbram.log" 2>&1
clean "$TOP/zig-0.17.1/clang" --version
clean "$TOP/zig-0.17.1/clang" -E -v -x c /dev/null > "$WORK/includes.log" 2>&1
if grep -Eq '/Applications/.*Xcode|/Library/Developer/| /usr/include' "$WORK/includes.log"; then
    cat "$WORK/includes.log" >&2
    echo 'Compiler used host development headers' >&2
    exit 1
fi
grep -q '/lib/libc/include/' "$WORK/includes.log"
echo 'Compiler headers: bundled libc (no host SDK/development headers)'
clean "$TOP/bin/python3" -c 'import encodings, ctypes, json, hashlib, ssl, sqlite3; print("Python stdlib: OK")'
[[ $(clean "$TOP/zig-0.17.1/zig" version) == 0.17.1 ]]
clean "$TOP/bin/yosys" -Q -T -m ghdl -m slang -p 'help ghdl; help read_slang' > "$WORK/plugins.log"
echo 'GHDL/slang plugins: OK'
[[ -f "$TOP/lib/ghdl/std/v08/std-obj08.cf" ]]
[[ -f "$TOP/lib/ghdl/src/ieee2008/std_logic_1164.vhdl" ]]
[[ -L "$TOP/share/trellis/database" && -f "$TOP/share/trellis/database/devices.json" ]]
cp -R "$RESOURCES/tour/blinky_cycle" "$WORK/blinky_cycle"
rm -rf "$WORK/blinky_cycle/prebuilt"
cd "$WORK"
clean "$CLI" device > device.log
clean "$CLI" build "$WORK/blinky_cycle" --out "$WORK/build" > build.log 2>&1 || {
    cat build.log >&2; exit 1;
}
echo 'blinky_cycle build: OK (packaged Yosys, ABC, nextpnr, ecppack)'
if clean "$CLI" new "$WORK/new-verilog" --resources clock,leds > new-verilog.log 2>&1 \
    && clean "$CLI" build "$WORK/new-verilog" >> new-verilog.log 2>&1; then
    echo 'New Verilog project + build: PASS (ABC exercised)'
else
    cat new-verilog.log >&2
    exit 1
fi
if [[ "$SHOWCASE" == --showcase ]]; then
    clean "$CLI" new --list-demos > "$WORK/showcase.txt"
    [[ -s "$WORK/showcase.txt" ]]
else
    : > "$WORK/showcase.txt"
fi
printf '%s\n' gallery/led_tennis ulx3s/blinky_vhdl ulx3s/sv_package >> "$WORK/showcase.txt"
LC_ALL=C sort -u "$WORK/showcase.txt" > "$WORK/demo-list.txt"
demo_failures=0
printf 'Demo\tResult\tSeconds (new + build)\n'
while IFS= read -r demo; do
    project="$WORK/demos/$demo"
    log="$WORK/demo-${demo//\//-}.log"
    TIMEFORMAT='%3R'
    if { time { clean "$CLI" new "$project" --from-demo "$demo" > "$log" 2>&1 \
        && clean "$CLI" build "$project" >> "$log" 2>&1; }; } 2> "$WORK/demo-time.txt"; then
        printf '%s\tPASS\t%s\n' "$demo" "$(cat "$WORK/demo-time.txt")"
    else
        printf '%s\tFAIL\t%s\n' "$demo" "$(cat "$WORK/demo-time.txt")"
        cat "$log" >&2
        demo_failures=$((demo_failures + 1))
    fi
done < "$WORK/demo-list.txt"
[[ "$demo_failures" == 0 ]] || { echo "$demo_failures demo build(s) failed" >&2; exit 1; }
# Observe the launcher's actual ECP5_CLANG default and arguments without overriding it.
# This modifies only the disposable copy. Its exec still reaches the packaged Zig.
CLANG="$TOP/zig-0.17.1/clang"
ZIG="$TOP/zig-0.17.1/zig"
mv "$CLANG" "$CLANG.real"
cat > "$CLANG" <<'SH'
#!/bin/sh
set -eu
if [ "$#" -eq 1 ] && [ "$1" = --version ]; then
    printf 'cached compiler identity\n' >> "$TMPDIR/../identity-probes.log"
else
    printf 'clang\t%s\n' "$*" >> "$TMPDIR/../compiler-invocations.log"
fi
exec "$(dirname "$0")/clang.real" "$@"
SH
chmod +x "$CLANG"
mv "$ZIG" "$ZIG.real"
cat > "$ZIG" <<'SH'
#!/bin/sh
set -eu
printf 'zig\t%s\t%s\n' "${ZIG_GLOBAL_CACHE_DIR-}" "${ZIG_LOCAL_CACHE_DIR-}" >> "$TMPDIR/../compiler-invocations.log"
exec "$(dirname "$0")/zig.real" "$@"
SH
chmod +x "$ZIG"
fast_run() {
    clean env FAST_CHOOSE=0 FAST_TUNE=0 FAST_THREADS=2 FAST_THREAD_MIN=1 "$CLI" emulate "$WORK/build/top.bit" \
        --fast --startup configured --scope observed --clock pin:G2 --period-ns 40 \
        --cycles 4096 --probe pin:B2 --probe pin:C2 --probe pin:C1 --probe pin:D2 \
        --probe pin:D1 --probe pin:E2 --probe pin:E1 --probe pin:H3
}
timed_run() {
    local phase=$1
    TIMEFORMAT='%3R'
    if { time fast_run > "$WORK/emulate-$phase.log" 2>&1; } 2> "$WORK/time-$phase.txt"; then
        cat "$WORK/emulate-$phase.log"
    else
        cat "$WORK/emulate-$phase.log" >&2
        exit 1
    fi
}
[[ ! -e "$FAST_CACHE" ]] # The first fast run must compile into a fresh program cache.
timed_run cold
grep -q 'pthread.h' "$FAST_CACHE/"*.c
clean "$TOP/bin/python3" "$WORK/check-caches.py" "$ZIG_CACHE" "$FAST_CACHE" "$WORK/cold-caches.json"
cp "$WORK/compiler-invocations.log" "$WORK/cold-invocations.log"
timed_run warm # Exact same launcher, compiler path, arguments and environment.
printf 'Fast run wall timings: cold %s s; warm %s s\n' "$(cat "$WORK/time-cold.txt")" "$(cat "$WORK/time-warm.txt")"
cmp "$WORK/cold-invocations.log" "$WORK/compiler-invocations.log"
clean "$TOP/bin/python3" "$WORK/check-caches.py" "$ZIG_CACHE" "$FAST_CACHE" "$WORK/cold-caches.json"
echo 'Warm native compiler processes: none (compile/Zig invocation logs unchanged; cached wrapper identity probes tracked separately)'
cat "$WORK/compiler-invocations.log"
mv "$CLANG.real" "$CLANG"
mv "$ZIG.real" "$ZIG"
echo 'fast emulate: OK (4096 cycles each, cold bundled-Zig compile and warm library reuse)'
if [[ -d "$COPY/Contents" ]]; then
    codesign --verify --deep --strict "$COPY"
    echo 'macOS signatures after smoke: OK'
fi
du -sh "$COPY" "$TOP" "$TOP/zig-0.17.1"
echo 'Relocated package smoke: PASS (empty environment, path with spaces)'
