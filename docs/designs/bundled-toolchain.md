# Bundled toolchain for the v0.1.0 packages

Status: packaging implemented; Zig 0.17.1 approved on 2026-11-18 after the compiler A/B and
the release proofs recorded below. The macOS proof used the maintainer's sandboxed Mac,
not a pristine Mac.

## Goal

All dependencies, including Yosys and nextpnr, must ship immediately.
A clean Apple Silicon Mac or Linux x86_64 machine must build an HDL project and emulate it,
including fast mode's generated-C compilation, without installing a toolchain or changing PATH.
Ship VHDL/SystemVerilog frontends now; enable their flow only when [slice 2](hdl-languages.md)
passes. This is a plan for self-contained packages, not a claim about the existing release assets.

## What ships

**Measured source:** OSS CAD Suite 2026-10-22, already pinned by CI. nextpnr is exactly
`0.11.1-77-g6b438a1c`; Yosys is `0.70+190`, not the Homebrew `0.70+post` used for recorded
references. Packaged builds have the measured functional behaviour but do not reproduce those
bitstreams/configurations byte for byte. Release notes, provenance and documentation must say so.
Installed copies: `~/FPGA/oss-cad-suite-20261022` on the Mac, `~/apps/oss-cad-suite-20261022` on rserv.
Measured linux-x64 SHA-256: `45b7401d677f1e6cffb19dcaafb8e92c95a49dfc05b332ae4d00b84c94f62f6e`.

Sizes below are MB as reported, with overlapping rows; do not sum them as independent payloads.

| Tool/resource | From | macOS | Linux x86_64 |
|---|---|---|---|
| `libexec/yosys`, `yosys-abc` | Suite | In measured 265 MB subset | 35 + 23 MB in native closure |
| `libexec/nextpnr-ecp5` | Suite GUI build | Five Qt frameworks included | 113 MB in native closure |
| `libexec/ecppack`, `ecpbram` | Suite/Trellis | In 265 MB subset | In 330 MB native closure |
| `libexec/ghdl`, `lib/ghdl` | Suite | In 265 MB subset | Executable 14 MB; libraries/data 23 MB |
| `share/yosys`, including `ghdl.so`, `slang.so` | Suite, ABI-matched to Yosys | In 265 MB subset | Tree 27 MB; slang plugin 12 MB overlaps closure |
| Native library closure | Suite | 35 files, including Qt about 24 MB | 62 files, 330 MB |
| Python 3.11 library and trimmed stdlib | Suite | Stdlib 18 MB | libpython 24 MB in closure; stdlib size pending |
| `zig cc`, libc headers and stubs | Zig 0.17.1, approved | Archive 54.0 MB compressed | Archive 57.4 MB compressed |
| `openFPGALoader`; Python executable for `ECP5_PYTHON` | Suite, proposed additions | Increment pending | Increment pending |

**Measured macOS:** preserve `@executable_path/../lib` and `../Frameworks`. The five frameworks are
QtCore, QtGui, QtWidgets, QtOpenGL and QtOpenGLWidgets. Embedded libpython3.11 needs its stdlib:
without it, nextpnr aborts with "failed to get the Python codec of the filesystem encoding".
The trimmed tree excludes site-packages, tests, tkinter, idlelib, ensurepip, lib2to3, unittest,
pydoc_data, distutils, config-* and __pycache__. With PYTHONHOME at the subset root, an
`env -i PATH=/usr/bin:/bin` run built blinky_cycle through synth_ecp5, nextpnr and ecppack;
Yosys found yosys-abc beside itself. The whole measured macOS subset is 265 MB uncompressed.

**Measured Linux:** the native closure includes ICU, Qt, Python, glibc and libstdc++.
Largest files: nextpnr 113 MB, libicudata 33 MB, Yosys 35 MB, libpython 24 MB, ABC 23 MB,
libghdl 14 MB, ghdl 14 MB and slang.so 12 MB. Add the data trees and trimmed Python, deduplicating
plugins already counted in the closure. The complete packaged union is not yet measured.

**Measured database reuse:** ecppack finds `../share/trellis/database` relative to its executable;
[flow](../../src/flow/pipeline.rs) passes no `--db`. The existing 152-file LFE5U-85F subset
produced the same bitstream as the full database. Propose a relative `share/trellis/database`
symlink to the package's existing database, rather than another 83 MB database copy.

**Proposed size budget, not measurements:** assume 50–150 MB for the existing app/resources,
150–300 MB for expanded Zig, and about 390–430 MB for the Linux suite/data union before pruning.
These allowances include uncertainty for the additional Python/USB tools; measure the final union.

| Complete package estimate | Uncompressed | Compressed delivery |
|---|---|---|
| macOS app/zip | 0.45–0.75 GB | 0.15–0.30 GB zip |
| Linux tarball | 0.60–0.90 GB | 0.20–0.40 GB gzip |

The packagers prune Zig's `lib/` to host libc headers/stubs and conservative compiler support.
The compiler wrapper passes an explicit host target (`aarch64-macos.15.0.0` or
`x86_64-linux-gnu.2.39`); otherwise Zig discovers an installed native SDK/libc and the
development-machine smoke would not exercise the bundled headers/stubs.

The local macOS packaging smoke now measures 616 MiB for the app, 177 MiB for its zip,
511 MiB for `toolchain/` including 239 MiB for pruned Zig. These are rounded disk sizes.
For release commit `0b9b7c8`, the Linux tarball `ecp5_emu-0.1.0-linux-x86_64.tar.gz` is
226,794,774 bytes (SHA-256 `6264910066a6f2005a266b3c2119d14dff2b12b51498e98355de97940a90311c`),
733 MB unpacked (`du`). The macOS zip of the same commit is 180,200,933 bytes
(SHA-256 `6a4ade9618873ac62511b3d3776ab434493fdf241fead6a82908bcf4f212535c`).
The completed release proofs are below. The current selection excludes the standalone GHDL
executable even though it was included in the earlier subset measurements.

## Compiler for fast mode

**Current code:** [fast host](../../src/fast.rs) invokes `ECP5_CLANG`, falling back to PATH clang; the
[kernel emitter](../../src/kernel/fast.rs) emits C split into units. Flags include `-O3`,
`-march=native`, `-shared -fPIC`, and on x86_64 `-mno-avx512f` unless AVX-512 is opted in.
Generated variants use stdint.h, pthread.h, stdatomic.h, stdio.h and time.h; macOS variants also
need pthread/qos.h, mach/mach_time.h and sys/sysctl.h. A compiler binary alone is insufficient:
it needs libc headers and libc/libSystem support to link the library loaded by the Rust process.

- System clang: unsuitable for the clean-machine requirement; it is not guaranteed installed.
- Official LLVM clang alone: no macOS SDK and no Ubuntu libc6-dev; it does not close that gap.
- Apple clang/SDK: not redistribution inputs for this plan; do not bundle them.
- **Approved choice:** Zig 0.17.1 (2026-11-02, project licence MIT), whose `zig cc` includes Clang,
  libc headers and macOS/glibc link stubs. Preserve its third-party licence texts too.

Provide a wrapper named `clang` that executes the bundled Zig by absolute path as `zig cc`,
forwarding arguments unchanged. Acceptance requires every generated-C variant, shared-library
loading, threads/QoS, cold compile time, life_full speed and byte-identical traces against today's
clang, with AVX-512 off. Target the app's supported OS/libc ABI, not the suite's private glibc:
the resulting library loads into the host Rust process. Separate compiler identities in caches.

**2026-11-18 compiler comparison:** fast's harness, `life_full`, 10 threads, 2 000 000 cycles,
`FAST_CHOOSE=0 FAST_TUNE=0`, three alternating rounds. Speeds are kHz, system clang/Zig per
pair. All six LED-trace hashes on each OS were identical and also equal across OSes:
`69df4ba55198`. Cold compile measurements were not supplied.

| Platform | Compiler size | Cold compile | life_full speed | Traces vs today's clang |
|---|---|---|---|---|
| macOS M3 Max, Apple clang 17 / Zig 0.17.1 | Zig 239 MiB pruned | Not supplied | 68.1/83.9; 81.7/103.0; 104.3/141.4 kHz | All identical: `69df4ba55198` |
| Linux rserv Ubuntu, clang 21.1.8 / Zig 0.17.1, AVX-512 off | Zig archive 57.4 MB compressed; pruned size not recorded | Not supplied | 85.1/81.3; 80.3/90.9; 87.2/68.5 kHz | All identical: `69df4ba55198` |

macOS load average was 39 at the start, run `run-mwlipmdk-bg972`; the watchdog classified
the result as inconclusive, so compare within pairs. Linux load was 7.9, run
`run-mwliyh8h-giz12`. The traces agree and speed differences are within the measurement noise;
these results do not establish a steady-speed winner. Together with the release proofs below,
they support bundling Zig. The packagers retain host libc headers/stubs,
compiler-rt, Clang headers and Zig compiler/std support, and prune other-platform libc,
C++ libraries, sanitizers, debugger support, examples and documentation. The relocated smoke
test proves a cold compile of the actual generated C through that pruned compiler.

## Wiring

**Implemented:** keep a suite-shaped root at `Contents/Resources/toolchain` on macOS and
`share/toolchain` on Linux: `bin/` wrappers, `libexec/` binaries, `lib/`, `Frameworks/` and `share/`.
The database symlink is `share/trellis/database -> ../../../database` within that root.

Packaged launchers set bundled defaults only for the launched process:

- `ECP5_YOSYS`, `ECP5_NEXTPNR`, `ECP5_ECPPACK`, `ECP5_ECPBRAM`, `ECP5_OPENFPGALOADER`;
- `ECP5_IMAGE_READER`, `ECP5_DB`, `ECP5_PYTHON`, `ECP5_CLANG` and `ECP5_OSS_CAD_SUITE`.

`ECP5_EXAMPLES` overrides bundled demo discovery; an invalid folder fails without fallback.

[Flow tool overrides](../../src/flow/tools.rs), `ECP5_CLANG` and the suite/Python launcher
defaults are implemented. The native image reader handles packaged decoding without pytrellis.
Retain explicit user overrides with recorded provenance; invalid overrides must fail visibly.
No user-installed executable or calling-shell PATH change is needed. A tool wrapper may prepend
its own `bin/` to its child's private PATH so Linux Yosys invokes the loader-backed yosys-abc.

**Measured Linux mechanism:** invoke each suite executable with its own
`lib/ld-linux-x86-64.so.2 --inhibit-cache --inhibit-rpath "" --library-path lib`, using absolute
bundle paths. Preserve this for Python, plugins and helper executables, not just top-level Yosys.
This isolates the suite from host glibc; it does not remove the Rust app's own host ABI baseline.
Set PYTHONHOME to the subset root per tool call; include a loader-backed Python executable for
`ECP5_PYTHON`, beyond the embedded libpython dependency. Derive GHDL_PREFIX per call from the
selected `ECP5_OSS_CAD_SUITE` root. Load ghdl.so/slang.so only with that root's Yosys; no silent
fallback to Homebrew or another frontend. Keep normal developer tool selection unchanged.

## Licences and source compliance

**Local read-only inspection:** the Mac suite has VERSION `20261022` and 53 `license/` files.
They are component reports with source revisions and licence text, including unshipped tools;
they are not a blanket suite licence or proof that every closure library is covered.
Linux inspection is pending; record VERSION/manifests, binary hashes and exact source revisions.

**Proposal:** add one [ledger](../../licenses/README.md) row per bundled tool and full applicable
texts in THIRD_PARTY_NOTICES.txt. Audit all libraries, plugins, Zig support files and USB libraries.
openFPGALoader is Apache-2.0. Copyleft inventory must include GHDL/libghdl (GPL-2.0+), the GHDL
Yosys plugin (local report carries GPL-3.0), libgnat (GPL-3.0 with GCC runtime library exception),
GNU readline (GPL-3.0), libiconv/libintl and glibc (LGPL), Qt (LGPL-3.0, dynamic), and
libstdc++/libgcc_s (GPL with runtime exceptions); verify exact terms per shipped version.

Propose publishing a third-party corresponding-source archive beside each binary release,
instead of a written offer: exact GHDL/plugin, GCC runtime, readline, glibc, libiconv/gettext,
Qt and any other GPL/LGPL closure sources, with patches, build/configuration scripts and notices.
Use suite source revisions and upstream tags, not links to moving branch heads. Keep a hash
manifest mapping binaries to source archives. This is a compliance proposal,
not a conclusion that a download link alone satisfies every licence: check LGPL replacement/
relinking and installation-information obligations and each runtime exception.
Estimate a few hundred MB compressed (0.2–0.6 GB), about 1–3 GB unpacked, separate from binary
package sizes; exact source versions and measured archive size remain pending.

## macOS signing

**Required by the measured packaging constraint:** sign every nested Mach-O inside-out, including
executables, dylibs, .so frontend plugins and framework binaries, then ad-hoc sign the app and
verify it deeply. Signing only the outer app is insufficient to launch this nested toolchain.
This does not add Developer ID signing or notarisation.

## Proof

**Findings, 2026-11-18:** the Linux clean proof (fresh Ubuntu 24.04, no FPGA tools or C
compiler, network down, relocated package path with spaces) found two packaging defects.
Loader-backed Yosys resolves its helper directory beside `ld-linux` in `lib/`, but the package
omitted the suite's `lib/yosys-abc -> ../bin/yosys-abc` alias. Both packages also omitted
`examples/`, so `new --from-demo` reported "Demo sources unavailable". The earlier Mac sandbox
proof had masked the missing examples through the compile-time checkout and the cwd. Linux
staging now derives all `lib/` aliases into staged `bin/` wrappers from the suite, failing on
unstaged targets; macOS remains unchanged. The packagers stage `git archive HEAD examples`
(or `ECP5_EXAMPLES_ARCHIVE` for a copied tree), preserving internal relative links and adding
an examples hash manifest and third-party notices. Examples live at `share/examples` in the
tarball and `Contents/Resources/examples` in the app before signing. Both group README files
are required and every relative symlink must resolve inside the tree.
The layout adjustment uses the merged release discovery code; launchers need no `ECP5_EXAMPLES`
export, and the compile-time checkout fallback is debug-only. The shipped tree has 432 regular
files and 31 symlinks (463 tracked entries): 1,630,745 stored bytes, comprising 1,630,052 file
bytes and 693 link-target bytes, without counting linked targets twice. Required clean-machine reruns use a neutral
cwd with user folders denied; the macOS smoke applies that denial by default and builds a new
Verilog project and copied gallery, VHDL and SystemVerilog demos. The showcase mode times
each demo's own creation/build commands.
The supplied Linux failures are evidence of the defects. The later post-fix Linux proof was
supplied separately; its success is recorded under Release decisions, not inferred from this Mac run.

The local macOS resource-layout rerun used release binaries from merged tree `bf7b7b5` and
passed with 463 archived entries (432 files, 31 symlinks), a generated Verilog project,
all 11 showcase demos and the two additional frontend demos below. Every runtime
command used a neutral temporary cwd, an empty environment with fresh HOME/TMPDIR, a relocated
app path with spaces and a space-free TMPDIR. The smoke asserted
`Contents/Resources/examples` and both README markers. Two negative checks failed as expected:
an invalid `ECP5_EXAMPLES` did not fall back, and hiding the packaged tree made discovery fail
even while the disposable compile-time build checkout remained available. Cold/warm fast runs
each completed 4096 cycles, with wall times 5.129/0.706 seconds; the warm library inode/mtime/hash
and Zig cache entries were unchanged.
The rebuilt CLI now probes compiler identity before cache lookup, so the packaged wrapper
serves its captured `clang-version.txt` for plain `--version` without launching Zig. Both
the rebuilt app and its exercised copy passed `codesign --verify --deep --strict`.

This is a scoped development-host result: this execution environment rejected the nested
`sandbox-exec` attempt with `sandbox_apply: Operation not permitted`. The successful run used
the explicit `--no-sandbox` option and did not verify user-folder or network denial. Those
denied-folder and Linux proofs were subsequently supplied for the release, as recorded under
Release decisions; a pristine-Mac proof is not claimed. Local personal-identity scanning found zero files; a broader scan still found upstream
build/sample home-path prefixes in 127 unchanged suite/Zig files. These vendor inputs were not
rewritten. The guard now checks HOME and path-shaped user directories, not bare usernames or
neutral checkout paths. An upstream path is allowed only when the same complete string exists
in the corresponding suite/Zig source file recorded by staging; generated files get no allowance.

| Demo | Result | Wall seconds, new + build |
|---|---|---:|
| `gallery/clock_domains` | PASS | 15.780 |
| `gallery/led_tennis` | PASS | 5.774 |
| `gallery/mandelbrot` | PASS | 7.446 |
| `gallery/oled_bricks` | PASS | 9.283 |
| `gallery/riscv_shell` | PASS | 17.443 |
| `gallery/riscv_terminal` | PASS | 20.905 |
| `gallery/sdram_bench` | PASS | 5.571 |
| `gallery/serv_shell` | PASS | 6.960 |
| `gallery/video_life` | PASS | 8.860 |
| `ulx3s/blinky_cycle` | PASS | 1.376 |
| `ulx3s/life` | PASS | 35.614 |
| `ulx3s/blinky_vhdl` (additional) | PASS | 1.407 |
| `ulx3s/sv_package` (additional) | PASS | 1.392 |

- Linux: fresh ubuntu:24.04 LXD container, no yosys, nextpnr, clang or build-essential; unpack the
  tarball, build a new project and a gallery demo, then emulate both with `--fast` and no downloads.
- macOS: pristine supported system/VM without Homebrew, OSS CAD Suite or Xcode CLT/SDK installed;
  run with `PATH=/usr/bin:/bin`, build the same projects and emulate in fast mode. PATH isolation
  alone is insufficient if an installed SDK or absolute library fallback remains accessible.
- After slice 2, also build/emulate VHDL and slang-dependent SystemVerilog and check source maps,
  diagnostics and tool provenance. Exercise ecpbram firmware replacement and the packaged tour.
- Relocate packages, including a path with spaces; check ABC, Python imports, GHDL libraries,
  frontend plugins, private loader paths, the shared database and nested signatures.
- Record commands, outputs, timings, tool/archive hashes and trace comparisons in `out/`;
  promote reproducible scoped evidence to `docs/inventory/`. Distinguish byte equality within
  the same toolchain from behavioural agreement with older recorded configurations.
- Recheck home-path scans, LICENSE and complete notices; never depend on a warm compiler cache.

<a id="owner-decisions"></a>

## Release decisions

On 2026-11-18 two release decisions were approved:

- Publish a corresponding-source archive beside each binary release, rather than a written offer,
  after a per-component audit records exact source revisions, patches/build scripts and a hash
  manifest mapping binaries to sources. The [licence audit](bundled-toolchain-licences.md) records
  the remaining unknowns; this decision does not clear them for release.
  Later on 2026-11-18 the audited estimate of 1.0–2.3 GB
  compressed was accepted, up from the initially approved 0.2–0.6 GB; GCC sources drive the increase. Nothing
  is cut. To meet GitHub's 2 GiB limit per release asset, split the archive by component group:
  suite-built tools, Ubuntu runtime sources for Linux, and macOS runtime sources, each part under
  2 GiB. Use one manifest mapping each bundled binary's SHA-256 to its source part and item.
- Accept the package size with the suite's GUI nextpnr; do not rebuild it without the GUI.

Bundling **Zig 0.17.1** as the fast-mode C compiler was approved on 2026-11-18.
The compiler A/B in "Compiler for fast mode" produced
identical traces `69df4ba55198` on macOS and Linux, with speed differences within noise,
and the clean-machine proofs of the `0b9b7c8` packages passed:

- Linux: a fresh Ubuntu 24.04 container without FPGA tools, a C compiler or network; every
  step exited 0, including a new project, a gallery demo copied from the package, `--fast`
  through Zig, VHDL via GHDL, SystemVerilog via slang, and a TMPDIR containing a space.
- macOS: a `sandbox-exec` proof on the maintainer's Mac, not a pristine Mac, with `/Users`,
  Xcode/CLT, Homebrew, `/usr/local`, the build checkout and the suite copy denied; all 13 of
  13 steps exited 0.

Logs are kept with the release evidence in `out/release/0.1.0/proofs/`, outside Git. These
proofs and the release approval settle Zig; they do not turn the earlier PATH-isolated smoke
into a pristine-Mac test or clear the separate source/licence audit items.

## Not in scope

No SDK or tool auto-download at runtime; no whole-suite redistribution without a closure audit.
No claim of complete VHDL/SystemVerilog conformance or byte reproduction of Homebrew references.
OS kernels, display servers, graphics drivers and USB access policy remain host responsibilities;
this note does not promise an arbitrary Linux distribution or a headless container can show a GUI.
