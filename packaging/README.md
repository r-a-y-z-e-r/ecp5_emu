# ECP5 Emu release packages

macOS arm64: unzip, then use `ECP5 Emu.app`. The CLI entry point is
`ECP5 Emu.app/Contents/Resources/bin/ecp5_emu`. Linux x86_64: unpack and use
`bin/ecp5_emu` or `bin/ecp5_gui`. Keep the package's directory structure intact.
The wrappers respect existing `ECP5_*` overrides, including empty values.
The CLI wrapper supplies the bundled architecture; a later `--architecture PATH`
argument overrides it. Packages, including CI builds, include the 3D ULX3S board model
shipped in the repository through git LFS. Run `git lfs pull` if a clone has pointer files.

For Linux desktop integration, copy the package's `share/icons/` and
`share/applications/` directories into `~/.local/share/` (merging with existing
directories). Put the package's `bin/` on the desktop session's `PATH`, or edit
`~/.local/share/applications/ecp5_emu.desktop` so `Exec` points to the full path
of the package's `bin/ecp5_gui` wrapper (quote the path if it contains spaces).
Keep the rest of the package together so the wrapper can find its resources.

The routed-die icon is original project artwork in `assets/icon/ecp5_emu.svg`;
`ecp5_emu-small.svg` removes pins and simplifies the grid for small displays.
Regenerate with `python3 tools/icon/build_icons.py` on macOS using Python 3's
standard library, `rsvg-convert` on `PATH`, and `/usr/bin/iconutil`. It produces
`assets/icon/ecp5_emu.icns`, eight PNG sizes under `assets/icon/hicolor/`, and
`site/theme/favicon.svg` and `favicon.png`. The 16/32-point macOS slots use the
small design at both scales; Linux sizes 16–32 and the 32 px favicon use it too.
Generated icons are versioned so CI and release packaging need no icon tools.
If `iconutil` reports `Invalid Iconset` during encoding, the script packs the
iconset into lossless ICNS directly (legacy RGB/alpha at 16/32 px, PNG for the
other slots); `iconutil` verifies all ten sizes
by decoding the result before it is installed.
Repeated builds with the same tools produce identical files without timestamps
or text metadata in the PNGs. mdBook reads the favicons directly from `site/theme/`.

The database subset is checked against the repository's SHA-256 inventory before
packaging. The architecture and configuration catalogues are bundled too. Native
Trellis and Boost libraries and their notices accompany the image reader.
The `tour/blinky_cycle` folder bundles the guided tour's project files and prebuilt configuration.
The project's Apache-2.0 `LICENSE` and `THIRD_PARTY_NOTICES.txt` are in `Contents/Resources` on
macOS and at the Linux tarball's top level.

The window finds the bundled architecture catalogues through ECP5_ARCHITECTURE,
which the launcher sets; the packaged window opens and runs designs without a
checkout (checked on macOS from `env -i`: uart_hello loaded and ran at 38.7 MHz).

Build tools and fast mode's compiler ship in the package; no runtime downloads or tool
installation are needed. Fast mode generates/compiles native code at runtime with Zig 0.17.1.
GUI rendering needs a working Metal device on
macOS or X11/Wayland and compatible Vulkan/OpenGL drivers on Linux. Linux keeps
glibc, libstdc++, libgcc and graphics/window-system libraries as host dependencies
for the Rust programs and native image reader (suite tools use their own runtime);
builds target Ubuntu 24.04 (glibc 2.39) or newer compatible systems.

macOS builds are ad hoc signed, not Developer ID signed or notarized. Gatekeeper
may require approval before opening the downloaded app. No Apple trust claim is made.
The packaging checks recorded here ran on macOS; they did not execute the Linux
packaging script on Linux.

## Building

Use Rust 1.92.0, Python 3.11+, the platform C++ tools, and a database matching
9754ff473bfaa1e6edfaf57ce13bd447c76a88f6. Set ECP5_DB to its root. For a local
Trellis installation set ECP5_TRELLIS_SOURCE to a checkout at
a31ff7042e712c0e2ad217054116ce2c4995f5c7 and ECP5_TRELLIS_PREFIX to its install
prefix. macOS also tries the usual $HOME/FPGA/prjtrellis checkout and `brew --prefix`.
Alternatively `bash packaging/build_trellis.sh` fetches and builds the pinned
source under target/packaging (requires CMake, Ninja, Boost and a C++ compiler).
Then run `bash packaging/macos/make_app.sh` or
`bash packaging/linux/make_tarball.sh`. Both build both Rust binaries with
`--release --locked`, generate notices, and place deliverables in target/dist.
Linux additionally requires clang++, patchelf, and the CI graphics dev packages.

Set `OSS_CAD_SUITE_ROOT` to the extracted build-platform OSS CAD Suite 2026-10-22
(`VERSION` must be `20261022`). Set `ZIG_ROOT` to extracted host Zig 0.17.1, or leave
it unset to download the platform archive into `target/packaging/`; its pinned SHA-256
is verified before extraction. Release CI passes `$RUNNER_TEMP/oss-cad-suite` and
uses the pinned Zig download. These are build-time inputs only.

Both packages include the committed `examples/` tree: `Contents/Resources/examples/` in the app
and `share/examples/` in the tarball, where the real executables discover it. Packaging
uses `git archive HEAD examples`, so untracked build outputs and working-tree edits are
excluded. For a copied build tree without `.git`, set `ECP5_EXAMPLES_ARCHIVE` to a tar
produced by that command. Relative links remain inside `examples/`; hashes and link
targets and sizes are recorded in `examples-manifest.json` beside the package notices.
Build output reports regular-file bytes and symlink-target bytes separately; their sum is
the stored payload size, without counting a linked target twice or filesystem allocation.
Both `ulx3s/README.md` and `gallery/README.md` must be present. No runtime checkout or
current-directory fallback is needed, and launchers do not export `ECP5_EXAMPLES`.

Example licence inventory (paths below are relative to that packaged examples directory):

| Third-party item | Retained notice |
|---|---|
| PicoRV32, ISC; `ulx3s/picorv32_console/picorv32.v`, shared by `picorv32_flash`, `picorv32_sdram`, `gallery/riscv_shell` and `gallery/riscv_terminal` | Full original source header and `THIRD_PARTY_NOTICES.txt` |
| SERV, ISC; `gallery/serv_shell/serv/` and its shared RTL links | `serv/LICENSE`, per-file copyright headers, and `THIRD_PARTY_NOTICES.txt` |
| SERV compressed decoder, Apache-2.0; `gallery/serv_shell/serv/serv_compdec.v` (lowRISC, ETH Zurich, University of Bologna; adapted to SERV) | Original attribution header plus full Apache-2.0 text in `THIRD_PARTY_NOTICES.txt`; package `LICENSE` is Apache-2.0 too |
| ULX3S constraints, EMARD MIT-style licence; `ulx3s/` and `gallery/` LPFs | Full `assets/ulx3s/LICENSE.md` in `THIRD_PARTY_NOTICES.txt` |

The demo firmware is project code under Apache-2.0. `riscv_terminal/font.hex` contains
original glyphs drawn by `make_font.py` (see its README), not a third-party font. Original
source headers, scripts, tracked firmware hex images and all existing demo notices ship
unchanged. On macOS the aggregate notices are `Contents/Resources/THIRD_PARTY_NOTICES.txt`;
on Linux they are both `share/THIRD_PARTY_NOTICES.txt` and top-level `THIRD_PARTY_NOTICES.txt`.

## Bundled toolchain and smoke test

The resource root is `Contents/Resources` on macOS and `share` on Linux. Its
`toolchain/` retains the suite layout: `libexec` tools, `bin` POSIX wrappers,
`lib` native closure and trimmed Python 3.11 stdlib, complete `lib/ghdl` catalogues
and sources, `share/yosys` with ABI-matched GHDL/slang plugins, and macOS `Frameworks`.
`share/trellis/database -> ../../../database` reuses the verified package database.
The GHDL executable and original suite wrappers are excluded. macOS uses `otool -L`
and Linux `readelf -d` recursively from the tools, plugins and retained Python
extensions; missing suite libraries fail packaging. Linux includes the suite's
loader/glibc and invokes tools with `--inhibit-cache --inhibit-rpath "" --library-path`.
Only each tool child's PATH gets its wrapper directory prepended, so Yosys finds ABC.
Linux also preserves the suite's `lib/` helper symlinks into `../bin/` (currently
`lib/yosys-abc -> ../bin/yosys-abc`), because loader-backed Yosys resolves helpers beside
the loader. Alias targets must have staged wrappers. macOS does not need these aliases.
nextpnr and Python set `PYTHONHOME` and `PYTHONNOUSERSITE`; Qt plugin paths are not needed.
They disable bytecode writes so imports do not modify the signed/read-only package.
Yosys defaults `XDG_STATE_HOME` to `${TMPDIR:-/tmp}/ecp5_emu_yosys` so batch synthesis
does not write history under HOME; an existing `XDG_STATE_HOME` is respected.

`toolchain/zig-0.17.1/clang` invokes `zig cc` using host libc headers/stubs and
compiler-rt sources. Other-platform libc trees, C++/sanitizer libraries, debugger,
examples and documentation are removed; compiler/std support is retained conservatively.
The wrapper supplies an explicit `aarch64-macos.15.0.0` or `x86_64-linux-gnu.2.39`
target to select bundled libc support rather than discovering a build host's SDK/headers.
Its plain `--version` reads `clang-version.txt`, captured from that compiler at package
time without the build-machine `InstalledDir` line. Fast mode uses this identity in its
cache key; a warm process can query it without starting Zig or changing compiler caches.
Zig caches default to `$HOME/Library/Caches/ecp5_emu/zig-0.17.1` on macOS or
`${XDG_CACHE_HOME:-$HOME/.cache}/ecp5_emu/zig-0.17.1` on Linux, respecting existing
`ZIG_GLOBAL_CACHE_DIR` and `ZIG_LOCAL_CACHE_DIR` settings.

All launchers default `ECP5_YOSYS`, `ECP5_NEXTPNR`, `ECP5_ECPPACK`, `ECP5_ECPBRAM`,
`ECP5_OPENFPGALOADER`, `ECP5_OSS_CAD_SUITE`, `ECP5_CLANG` and `ECP5_PYTHON` to bundled
paths. The native image reader handles packaged bitstreams without pytrellis/site-packages;
the Python executable also supports the existing `ECP5_PYTHON` interface.
ABC's wrapper translates `--version` to `-c version`; ecpbram has no upstream version flag,
so its wrapper reports the pinned suite/Trellis identity for that flag only.

`packaging/toolchain/components.json` selects roots, data trees, pruning, downloads and
notices, and retains the component audit for the later source-archive job.
`toolchain/bundle-manifest.json` records final file hashes and relative symlinks (after
nested ad-hoc signing on macOS). `THIRD_PARTY_NOTICES.txt` includes suite component reports,
Zig's licence and pinned runtime licence texts. Corresponding sources are published beside
the release in separate archives; building those archives is a separate job. The audit's
unresolved distributor-source identities remain release follow-up work.

Run `packaging/toolchain/smoke.sh 'target/dist/ECP5 Emu.app'` on macOS or
`packaging/toolchain/smoke.sh target/dist/ecp5_emu-VERSION-linux-x86_64` on Linux.
It copies the package into a path with spaces and runs under `env -i PATH=/usr/bin:/bin`
with fresh HOME/TMPDIR: tool versions, Python imports, frontend plugin loading,
packaged-example manifest checks, a new Verilog project build, and copy/build checks
for `gallery/led_tennis`, `ulx3s/blinky_vhdl` and `ulx3s/sv_package`, followed by
`ecp5_emu build` on the bundled `blinky_cycle` source, and 4096 cycles of `emulate --fast`
with two threads and all eight LED pins observed to exercise pthread/atomic/platform compiler support.
The compiler include search is checked for host SDK/development headers.
An instrumented wrapper in the disposable copy verifies the cold generated-C compile
uses bundled Zig; it prints package/toolchain/compiler sizes. This development-host
smoke complements the pristine-machine proofs described in the design.
The smoke prints the exact asserted examples location. Negative checks require an invalid
`ECP5_EXAMPLES` to fail without fallback, and a release executable to fail discovery while
the packaged tree is hidden (the compile-time checkout is a debug-only fallback).
It also times an identical warm run, requires unchanged program-library and Zig-cache
snapshots with no native compiler invocation, validates the app's cache locations, and
lists everything created under the fresh HOME and TMPDIR before removing them.
Every runtime step uses a neutral temporary cwd. On macOS `sandbox-exec` denies access
to the original user folders and the network, preventing the checkout from masking a
missing dependency. Add `--showcase` to copy/build every name from `new --list-demos`
as well; each row reports the command result and wall seconds for creation plus build.
TMPDIR remains space-free for comparison with the previous packaging run; the merged build
flow also supports temporary paths containing spaces.
If an enclosing execution sandbox rejects `sandbox_apply`, `--no-sandbox` permits a
scoped neutral-cwd check and explicitly reports that user-folder/network denial was not
verified. This is not a substitute for the clean-machine proof.

Release workflows build artifacts and create **draft** releases only on v* tags;
publishing a release, creating/pushing tags, enabling Pages and Apple signing or
notarization are manual steps. Site output carries mdBook theme notices.

## Release builds

v0.1.0 used a release-commit checkout outside home: a macOS worktree under `/private/tmp`, a
Linux copy under `/var/tmp`. Rust embeds absolute registry
paths (`$HOME/.cargo/registry/…`) for panics and `env!("CARGO_MANIFEST_DIR")` for defaults in
`src/{main,image,live,sources}.rs` and `src/kernel/fast.rs`; remapping fixes only the former.
Export these in the shell before `packaging/build_trellis.sh` and the packaging script:
`RUSTFLAGS` covers the Rust programs; `CFLAGS`/`CXXFLAGS` cover libtrellis (configured in a fresh
build directory). The image reader is compiled without those flags; the v0.1.0 scan found no path
in it.

```sh
export RUSTFLAGS="--remap-path-prefix=$HOME/.cargo=cargo \
--remap-path-prefix=$HOME/.rustup=rustup --remap-path-prefix=$PWD=ecp5_emu"
export CFLAGS="-ffile-prefix-map=$PWD=ecp5_emu" CXXFLAGS="-ffile-prefix-map=$PWD=ecp5_emu"
```

Inputs: `ECP5_DB` at the pinned database; `target/reference/blinky_cycle` built for the tour;
Linux `patchelf` on `PATH` for library bundling. `resources.py` looks for the board model at
`ECP5_BOARD_MODEL`, then `out/ulx3s_3d/ulx3s_v3.0.8.glb` in this checkout and the main
checkout, then `assets/ulx3s_3d/ulx3s_v3.0.8.glb`. It skips LFS pointers with a warning;
release CI fetches LFS objects. The launchers set `ECP5_BOARD_MODEL` to the bundled model.
Rebuild with `tools/ulx3s_3d` using KiCad 10, Blender 5.1, `rsvg-convert`, Pillow and numpy.

Pre-upload: scan every `.app` and extracted-tarball file, including binaries, for `/(Users|home)/`
and user names: zero hits; `LICENSE` byte-identical, `THIRD_PARTY_NOTICES.txt` present, and the
packaged CLI runs `device` under `env -i`.

Linux inherits the host's glibc: v0.1.0 used Ubuntu 26.04; `ecp5_gui` needs `GLIBC_2.43`
(`objdump -T`). An older build host lowers that; the release workflow uses `ubuntu-24.04`.

## Corresponding-source archives

Run from the release checkout after package relocation/signing. Supply either or both
package inputs; an unpacked Linux root is also accepted (`--version` if renamed):

```sh
python3 packaging/toolchain/source_archive.py --app "$APP" --tarball "$LINUX_PACKAGE"
python3 -B -m unittest discover -s packaging/toolchain/tests -v
```

`--cache` and `--out` default to `out/source-archive/cache` and
`out/source-archive/dist`; both must remain under `out/source-archive/`.
Python 3.11+ standard library and Git suffice. Expanded Git trees are removed after
packing, downloads are verified before reuse, and the cache has an 8 GiB guard.
`--repository-revision` can explicitly name the release commit used for generated
wrappers/manifests; it defaults to checkout HEAD. The repository itself is referenced,
not archived.

The three groups are `suite`, `ubuntu-runtime`, and `macos-runtime`. Every part is a
deterministic `.tar.xz` below 2 GiB with `SOURCES.md` and a `README.md` covering
dynamic-library replacement, runtime exceptions and per-file licence choices.
Oversized source files are fragmented with offsets and reassembly hashes. Each item's `METADATA.json` retains
its complete checksum/notice inventory; `!` in manifest notice paths enters a nested
tar. A multifile item's source SHA-256 hashes the canonical JSON inventory of
`[filename, sha256]` pairs; a Git item's hash covers its normalized source tar.

Package bytes are inventoried at startup, before long source downloads/compression,
so later input cleanup does not invalidate a completed inventory.
The JSONL manifest inventories the actual package toolchain, including generated
files, symlinks and embedded components. The adjacent `sources-report.json` records
coverage counts, measured part hashes/sizes, fetch failures, the original 23-item
obligation triage, and proposed third-party notice text. `copyleft-source-satisfied`
uses exact sources where known and honest closest-source statements plus recorded
distributor patches/recipes otherwise. Historical permissive-component and MacPorts
tooling provenance is recorded as `provenance unknown, not required`.
An exit status of zero means assembly finished; check the report's `status` before
release. An `incomplete` report is not release clearance. Rerun on the final packages;
compare the part hashes from two output directories to verify deterministic output.
