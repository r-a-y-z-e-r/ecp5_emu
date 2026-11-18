# Bundled toolchain: component licence audit

Audit date: 2026-11-18. The source-obligation triage below supersedes the earlier
provenance-blocker list. Sources are exact where established; otherwise the archive supplies an
honest closest-source statement and the recorded distributor patches/recipes. Historical input
precision for permissive components and MacPorts build tooling is not a source-delivery requirement.
Compiler A/B and clean-machine proofs remain operational work, separate from these
copyleft-source classifications. Source-archive assembly does not publish or commit its outputs.

The machine-readable [component manifest](../../packaging/toolchain/components.json) contains one
object per component: versions, exact listed files, SPDX terms, copyleft kind, source references,
patch/build inputs, archive provision and explicit unknowns. Embedded components share a binary;
component records, files and source estimates must not be counted as independent binary payloads.
`NOASSERTION` means the complete licence expression is unknown; observed terms are kept separately.
A named upstream licence does not establish the source identity or modifications of a copied library.

<a id="coordinator-triage-of-the-original-23-entries"></a>

## Source-obligation triage of the original 23 entries

Classifications concern source delivery: exact sources where known, otherwise the closest
identifiable sources with recorded patches and recipes. They do not establish exact historical binary reproducibility. The original reasons/evidence and nonblocking unknowns
remain in `source_assembly.obligation_triage`. Mixed entries retain permissive subfindings while
closing the separately supplied MPL/GPL/LGPL source portion.

| Original entry / component | Class | Reason |
|---|---|---|
| 1 linux: ghdl, ghdl-data, llvm-ghdl | copyleft: satisfied | The c966be324 source, recorded configure diff and suite recipe are supplied with an honest statement that no further distributor change is recorded. |
| 2 macos: ghdl, ghdl-data, llvm-ghdl, gcc-libgcc, gcc-libgnat | copyleft: satisfied | The dd4f9740 source/CI and UUID-matched GNAT 14.2.0-1 closest GCC release, Darwin fork and distributor recipes are supplied with the remaining discrepancy stated. |
| 3 macos: boost | provenance unknown, not required | boost is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 4 macos: bzip2 | provenance unknown, not required | bzip2 is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 5 macos: gettext | copyleft: satisfied | The shipped libintl has its checksum-verified gettext distfile, Portfile and patches; global MacPorts tooling is not a source obligation. |
| 6 macos: libiconv | copyleft: satisfied | The shipped libiconv has its checksum-verified distfile, Portfile and patches; global MacPorts tooling is not a source obligation. |
| 7 macos: libffi | provenance unknown, not required | libffi is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 8 macos: libusb | copyleft: satisfied | The shipped libusb has its verified distfile and Portfile, which declares no port patch; base/PortGroups are tooling. |
| 9 macos: libftdi | copyleft: satisfied | The shipped libftdi C library has its verified distfile, Portfile and GNU Library GPL v2 text; its port declares no additional patch. |
| 10 macos: ncurses | provenance unknown, not required | ncurses is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 11 macos: readline | copyleft: satisfied | Readline has its verified distfile, all three upstream patches and the shipped-version Portfile; MacPorts tooling is not a source blocker. |
| 12 macos: zlib | provenance unknown, not required | zlib is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 13 macos: zstd | provenance unknown, not required | zstd is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 14 macos: hidapi | provenance unknown, not required | hidapi is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 15 macos: xz | provenance unknown, not required | xz is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 16 macos: openssl | provenance unknown, not required | openssl is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 17 macos: sqlite | provenance unknown, not required | sqlite is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 18 macos: realpath | provenance unknown, not required | realpath is permissively licensed under the selected terms; exact MacPorts build-tool provenance is not a source obligation. |
| 19 macos: gdbm | copyleft: satisfied | GDBM has its verified source distfile, shipped-version Portfile and port patch. |
| 20 macos: qt | copyleft: satisfied | Qt 6.10.2 full source, documented arm64 edit/configuration and dynamic-library replacement instructions are supplied. |
| 21 macos: nextpnr, python | copyleft: satisfied | The recorded MacPorts Eigen 3.4.1 source/Portfile/patches close the MPL file-level source requirement; ISC/PSF and other permissive build provenance is not required. |
| 22 linux: nextpnr, python | copyleft: satisfied | The closest Ubuntu-series Eigen 3.4.0-5 source/patch package closes the MPL file-level source requirement with an honest statement; permissive build provenance is not required. |
| 23 both: zig-libc-darwin-headers, zig-libc-glibc, ghdl-data, qt | copyleft: satisfied | Exact retained Zig source/header copies, GHDL/Qt sources, file-level licence elections and the relinking/runtime-exception note are supplied; compiler proofs are separate operational work. |

**Copyleft: open: none** after successful assembly and verification of all registered source items.
The report treats a missing required source/fetch failure as open. Permissive provenance gaps do not
become blockers. The exact GHDL plugin pin, both Ubuntu GCC source packages, glibc and the other
exact Ubuntu copyleft source/patch payloads remain supplied.
This also includes the discovered LGPL libxcrypt runtime, matched to Ubuntu source
`1:4.5.1-1`; it is classified as copyleft satisfied rather than permissive provenance.

### GHDL and the macOS GCC runtime source set

Corresponding source for the Linux GHDL objects is supplied as the closest identifiable upstream tree c966be32400b25310a6d1028ab3a3b8462a8dea5, the complete suite recipe at cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0 (including default/scripts/ghdl.sh), and ghdl-linux-known-recipe-edit/ghdl-linux-recipe-edit.patch. The recorded linux-x64 edit changes every ghdl1-llvm occurrence in configure to ../bin/ghdl1-llvm. No further source modification is recorded by the distributor in the available recipe/report; this is not an assertion that the unavailable historical workspace had no additional changes. The .dirty indication is retained honestly. The exact historical LLVM build is unknown, not a copyleft-source blocker for LLVM under Apache-2.0 WITH LLVM-exception.

The shipped libghdl identifies dd4f9740079b2fe79baf92b43d4a10a16254205a; that complete GHDL source tree is supplied, including .github/workflows/Build-MacOS.yml, Pipeline.yml, Parameters.yml and the repository build/test scripts. The suite ghdl.sh at cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0 records nightly extraction, install-name edits and signing. The original nightly asset ID and complete historical environment remain unrecorded. No additional source diff is recorded in the available distributor inputs. This is the closest identifiable corresponding source plus recorded build/patch inputs, not a claim of exact binary reproducibility. The UUID-matched GNAT 14.2.0-1 runtimes are covered by the separate GCC source items.

The stripped app libgnat-14 and libgcc_s.1.1 contain no full printable GCC release string. Their Mach-O UUIDs match the verified official GNAT-FSF 14.2.0-1 asset (SHA-256 cc5517d65c2074807c8ae97b671e852ed70e307a457e7acde0a848a9394e31c5); that asset's gcc/gnatls strings identify 14.2.0. The binaries' retained object-path fingerprint identifies GNAT-FSF-builds, not Homebrew. GCC 14.2.0 release source, COPYING3 and COPYING.RUNTIME are supplied. The distributor recipe at 70fbb4cbda40f826d166c175340c205f0dd780b0 still selects the gcc-14.1-darwin-r1 fork; that closest identifiable modification tree and the full GNAT-FSF recipe set are also supplied. The reason for the 14.1 recipe versus 14.2 asset discrepancy and any unrecorded producer changes remain unknown. These source snapshots are alternatives/evidence, not instructions to blindly apply a 14.1 patch to 14.2. Under the exact-where-known/otherwise-closest-source rule, this supplies the runtime corresponding source; it does not assert an exact historical rebuild.

Primary evidence: [GHDL workflow at dd4f9740](https://github.com/ghdl/ghdl/blob/dd4f9740079b2fe79baf92b43d4a10a16254205a/.github/workflows/Build-MacOS.yml),
[GNAT-FSF 14.2.0-1 asset](https://github.com/alire-project/GNAT-FSF-builds/releases/download/gnat-14.2.0-1/gnat-aarch64-darwin-14.2.0-1.tar.gz),
[GCC 14.2.0 release and published SHA-512](https://gcc.gnu.org/pub/gcc/releases/gcc-14.2.0/sha512.sum),
[GNAT distributor recipe](https://github.com/alire-project/GNAT-FSF-builds/blob/70fbb4cbda40f826d166c175340c205f0dd780b0/specs/gcc.anod), and
[nearest Darwin fork](https://github.com/iains/gcc-14-branch/tree/0b3ebcb06ad636f5c58f65f404fd1fa2cfcc37db).
Both delivered runtime UUIDs match that asset (including the arm64 slice of its universal libgcc).
`macos-gnat-runtime-evidence/runtime-origin.json` preserves member hashes, UUIDs and the compiler
strings, without copying absolute build-user paths into the manifest. The old recipe mismatch
is stated rather than silently substituting a Homebrew formula.

### MacPorts copyleft libraries and Qt

For readline, gdbm, libintl/gettext, libiconv, libusb and libftdi, the verified upstream source
distfile, shipped-version Portfile, every file in its matching patch directory and original receipt
are the supplied corresponding source. Readline includes readline83-001 through -003. Libusb and
libftdi have no additional port patch in those Portfiles; conditional edits/flags remain in the
Portfiles. MacPorts base/PortGroups are build tooling and their exact historical checkout is not
an open source obligation. The earlier per-port reconstruction evidence remains below.

Qt 6.10.2 corresponding source is supplied as the full official source archive, with the resource builder README at df5b8aba9019f8f44a10796e1c96576ef2750b51 recording configure options and the arm64 edit to qtbase/src/corelib/thread/qyieldcpu.h (the Q_PROCESSOR_ARM_64 guard and arm_acle.h include). The six Qt frameworks are separate dynamically linked libraries. No additional source modification is recorded in the available distributor recipe. This source plus documented edit and relinking note satisfies the LGPL-3 source-delivery criterion used in this audit; unknown historical build logs/cache are nonblocking provenance.

### Eigen compiled into nextpnr

| Platform | Closest identifiable source | Evidence / modification payload |
|---|---|---|
| Linux | Ubuntu eigen3 **3.4.0-5**, the resolute series source | [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/eigen3/3.4.0-5/eigen3_3.4.0-5.dsc); all orig/Debian files and patches are verified |
| macOS | MacPorts eigen3 **3.4.1_0** | [matching Portfile](https://raw.githubusercontent.com/macports/macports-ports/e1682b0b172a5db350e2fdf5aa88f2e11eeb5ea2/math/eigen3/Portfile); upstream SHA-256 `8bb7280b7551bf06418d11a9671fdf998cb927830cf21589b394382d26779821`, full port patch directory and receipt |

Eigen headers are compiled into nextpnr; MPL-2.0 is file-level and does not change nextpnr's ISC licence. Ubuntu 26.04/resolute provides eigen3 3.4.0-5, the closest identifiable source for the recipe's libeigen3-dev build input. The .dsc, original source and Debian patch/build payload are supplied with all notices. No build-image dependency manifest proves the exact historical header installation, and no additional Eigen modification is recorded by the suite. This is the closest identifiable corresponding source under the source-delivery criterion used in this audit.

The Darwin build image explicitly installs eigen3 3.4.1_0. The verified distributor resource contains its matching receipt/Portfile; the upstream distfile and matching port-directory patches are supplied. This is the closest identifiable Eigen source for nextpnr's MPL-2.0 headers. No additional Eigen source modification is recorded by the suite. The exact global MacPorts tree/PortGroups are unknown build tooling, not a source-delivery blocker. MPL obligations apply to the covered files; other Eigen source-file licence notices remain in place.

Covered Eigen source files remain available under MPL-2.0; nextpnr’s separate ISC code is not
relicensed as a whole. This follows the [MPL file-level/source-availability explanation](https://www.mozilla.org/en-US/MPL/2.0/FAQ/).
Optional Eigen files carrying LGPL/Apache/BSD/Minpack notices retain those notices too.

## Relinking and runtime exceptions

Corresponding sources are delivered beside the binary release, not by written offer. Exact sources are used where known; honest closest-source statements and the recorded distributor patch/recipe sets accompany the reconstructed items in SOURCES.md. Historical build-tool provenance is kept separate from source obligations.

## Dynamic library replacement and relinking

The LGPL libraries are separate dynamic libraries, including Qt frameworks; users may replace them with modified, ABI-compatible builds and relink/debug the combined work for those modifications. Preserve library notices and give recipients these sources and licence texts. This is not permission to remove other applicable file notices.

Linux: replace the corresponding files under share/toolchain/lib (keep a coherent glibc/loader set where replacing glibc). The supplied wrappers load these private libraries.

macOS: work on a writable copy of the app. Replace the library under Contents/Resources/toolchain/lib, or the Qt framework under Contents/Resources/toolchain/Frameworks. Re-sign the replaced nested library/framework and the modified app ad hoc. Rebuilding/replacing a library invalidates the prior code signature; an ad hoc signature is sufficient for this replacement workflow. Original distribution signatures are not required to be preserved for a user's modified copy.

## GCC and GHDL runtime exceptions

libgcc and libgnat retain GPL-3.0-or-later WITH GCC-exception-3.1 as applicable to their source files. COPYING3 and COPYING.RUNTIME are included with the GCC sources. The GCC Runtime Library Exception permits covered runtime code to be combined with Independent Modules under its stated Eligible Compilation conditions; it is not a blanket relicensing of GHDL or its GPL Yosys plugin. GHDL GRT's own linking-exception text is separately retained in src/grt/ghdl_main.adb; it is not the GCC exception.

## File-level licence elections and source headers

- zstd: BSD-3-Clause for library files offering BSD/GPL alternatives; other file licences remain as marked.
- HIDAPI: BSD-3-Clause for files offering that alternative; other notices remain intact.
- D-Bus: AFL-2.1 for library files offering AFL/GPL alternatives; BSD/MIT files keep their licences. GPL-only standalone D-Bus tools are not shipped.
- FreeType: FTL for files offering FTL/GPL alternatives, including its attribution; additional per-file terms are preserved.
- Graphite: LGPL-2.1-or-later for library files offering LGPL/GPL/MPL alternatives; the separate dynamic library and exact Ubuntu source package are supplied.
- Eigen: MPL-2.0 applies to covered header files compiled into nextpnr, not to nextpnr's ISC code as a whole. The closest platform source sets and distributor changes are supplied with statements. Other Eigen file licences, including optional LGPL/Apache/BSD/Minpack material, remain in the source tree.
- Zig Darwin headers: files bearing APSL-2.0 retain APSL-2.0, alongside their applicable BSD or combined notices. The exact Zig-modified retained headers are supplied as source; Zig's root MIT licence does not replace their licences.
- Zig glibc headers/stubs: the exact retained, modified files are supplied in source form with LGPL-2.1-or-later and other applicable per-file notices. No blanket MIT or GPL claim is made for all libc files. Linux UAPI syscall-exception notices remain in their source headers.

Permissive Python/ISC/Boost/BSD/MIT/Apache inputs, including LLVM with its exception and MacPorts base/PortGroups used as build tooling, may have unknown historical provenance; this is recorded as provenance unknown, not required, rather than an open source-delivery blocker. All source notices and licence exceptions remain retained.

The notice text is tracked in [THIRD_PARTY_NOTICES-addition.txt](../../packaging/toolchain/THIRD_PARTY_NOTICES-addition.txt).
`packaging/notices.py` appends it in the shared toolchain-notices hook used by `resources.py`,
so both platform packages include it in their generated `THIRD_PARTY_NOTICES.txt`.
The source report also retains it under `third_party_notices_addition`.
Archive `SOURCES.md` separately prints each honest source statement and source/patch hashes.

<a id="coordinator-rerun-verification"></a>

## Source-assembly rerun verification

Eleven offline unit tests pass. Two complete assembler runs on the same private candidate
snapshots produce identical source-part and JSONL-manifest SHA-256 values. Independent
verification rehashes every shipped regular file, checks all literal symlink targets and
all **91** archived source items, reassembles the Qt fragments by hash, and confirms root
`README.md`/`SOURCES.md` plus the required honest statements. The GHDL CI files, known
configure diff, GCC release/exception texts and six MacPorts copyleft patch sets were
checked independently against the pinned input archives. Both nextpnr binaries map to Eigen.

Report status: **copyleft-source-satisfied** under the source-delivery criterion above: **12** original
entries copyleft satisfied, **11** provenance unknown/not required, **0** copyleft open.
Fetch failures and missing required file mappings: **0**. This also supplies libxcrypt and
the exact Zig-modified Linux UAPI headers with their syscall-exception notices.

| Part | Bytes | SHA-256 (identical in both runs) |
|---|---:|---|
| `ecp5_emu-0.1.0-sources-suite.tar.xz` | 152758760 | `703387208aeb7d38c7b59ba287b2af15d0ae98422345a81a45cc5fe14cfbbd4c` |
| `ecp5_emu-0.1.0-sources-ubuntu-runtime.tar.xz` | 570556024 | `698f29489d1a72bc1d53258704b6dedb18ba36f5ded035a28d0f97d517d6a402` |
| `ecp5_emu-0.1.0-sources-macos-runtime.tar.xz` | 1701747316 | `d8f3ece6c4bd3bb53b2f0b756703b413ec5ab92e47851368e5f658d7778f92fc` |

| Platform | Regular files | Symlinks | Manifest records | Missing |
|---|---:|---:|---:|---:|
| linux | 4176 | 2 | 4178 | 0 |
| macos | 2972 | 19 | 2991 | 0 |

Each part is strictly below 2 GiB; the cache measures **4407866708 bytes**, below 8 GiB.
No expanded Git source checkout remains. Outputs are in `out/source-archive/dist/`, the
repeat in `dist-repeat/`, and detailed checks in `verification.json` and
`delta-input-verification.json`. These archive measurements precede the packaging-notice
integration: the notice text is now tracked under `packaging/toolchain/` and appended by
`packaging/notices.py` to both platforms' generated notices.

These are candidate-source proofs; rerun against final relocated/signed packages with the
release commit for generated files. Unknown historical environments/workspaces remain
explicit closest-source statements. PGP/Minisign signatures are retained without independent
signature verification. Compiler A/B and clean-machine proofs remain separate operational work.

<a id="provenance-history-before-the-coordinator-triage"></a>

## Provenance history before the source-obligation triage

The history and initial planning estimates below are retained as evidence. Statements that treated
exact permissive inputs, historical workspaces or all global MacPorts tooling as release blockers
are superseded by the classifications above; they do not reopen those entries.

## Corresponding-source assembly, 2026-11-18

The [assembler](../../packaging/toolchain/source_archive.py) now consumes the explicit
`source_assembly` plan in [components.json](../../packaging/toolchain/components.json).
This resolves all **25** formerly incomplete Linux runtime source-package versions and confirms
all previously inferred versions against historical binary packages. It also records newly reached
OpenSSL, SQLite, XZ, libxcrypt, util-linux and wide-character ncurses dependencies. GDBM libraries
are present in the Mac candidate; the Linux candidate contains neither previously proposed GDBM
library. Historical proposed-file lists below are not final-package inventories.

The build image is Ubuntu **26.04 / resolute**, established by the pinned
[base Dockerfile](https://github.com/YosysHQ/oss-cad-suite-build/blob/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0/docker/cross-base/Dockerfile).
Binary publications were queried for resolute/amd64 through 2026-10-22, including superseded
builds. Each file was matched by its recorded SHA-256 or its retained ELF GNU build ID from the
candidate after relocation. `source.linux.binary_identity` preserves binary-package versions,
build/publication/deb URLs, deb SHA-256, member paths, file hashes and build IDs. Source-package
versions retain epochs and repack suffixes. `dsc_url`, `dsc_sha256` and `dsc_checksums` preserve the
full source identity and every referenced file hash. Where available, the source upload’s `.changes`
corroborates the `.dsc` hash; otherwise its SHA-256 was measured from Launchpad HTTPS bytes.
Embedded PGP and upstream Minisign signatures are retained, without independent signature
verification. All strong and weaker file hashes listed by a `.dsc` are checked during assembly.

| Linux component(s) | Exact Ubuntu source version | Evidence |
|---|---|---|
| `boost` | `boost1.90 1.90.0-6ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/boost1.90/1.90.0-6ubuntu1/+build/32762356); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/boost1.90/1.90.0-6ubuntu1/boost1.90_1.90.0-6ubuntu1.dsc) |
| `brotli` | `brotli 1.2.0-3build1` | [binary build](https://launchpad.net/ubuntu/+source/brotli/1.2.0-3build1/+build/32452738); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/brotli/1.2.0-3build1/brotli_1.2.0-3build1.dsc) |
| `bzip2` | `bzip2 1.0.8-6build2` | [binary build](https://launchpad.net/ubuntu/+source/bzip2/1.0.8-6build2/+build/32182661); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/bzip2/1.0.8-6build2/bzip2_1.0.8-6build2.dsc) |
| `dbus` | `dbus 1.16.2-2ubuntu4` | [binary build](https://launchpad.net/ubuntu/+source/dbus/1.16.2-2ubuntu4/+build/32334469); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/dbus/1.16.2-2ubuntu4/dbus_1.16.2-2ubuntu4.dsc) |
| `double-conversion` | `double-conversion 3.4.0-1` | [binary build](https://launchpad.net/ubuntu/+source/double-conversion/3.4.0-1/+build/31601637); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/double-conversion/3.4.0-1/double-conversion_3.4.0-1.dsc) |
| `expat` | `expat 2.7.4-1` | [binary build](https://launchpad.net/ubuntu/+source/expat/2.7.4-1/+build/32212246); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/expat/2.7.4-1/expat_2.7.4-1.dsc) |
| `fontconfig` | `fontconfig 2.17.1-3ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/fontconfig/2.17.1-3ubuntu1/+build/32171343); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/fontconfig/2.17.1-3ubuntu1/fontconfig_2.17.1-3ubuntu1.dsc) |
| `freetype` | `freetype 2.14.2+dfsg-1ubuntu0.1` | [binary build](https://launchpad.net/~ubuntu-security-proposed/+archive/ubuntu/ppa/+build/33405403); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/freetype/2.14.2+dfsg-1ubuntu0.1/freetype_2.14.2+dfsg-1ubuntu0.1.dsc) |
| `gcc-libgnat` | `gcc-14 14.3.0-14ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/gcc-14/14.3.0-14ubuntu1/+build/32378193); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-14/14.3.0-14ubuntu1/gcc-14_14.3.0-14ubuntu1.dsc) |
| `gcc-libgcc`, `gcc-libstdcxx`, `gcc-libatomic`, `gcc-libgomp` | `gcc-16 16-20260322-1ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/gcc-16/16-20260322-1ubuntu1/+build/32706076); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-16/16-20260322-1ubuntu1/gcc-16_16-20260322-1ubuntu1.dsc) |
| `glib` | `glib2.0 2.88.0-1` | [binary build](https://launchpad.net/ubuntu/+source/glib2.0/2.88.0-1/+build/32385064); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/glib2.0/2.88.0-1/glib2.0_2.88.0-1.dsc) |
| `glibc` | `glibc 2.43-2ubuntu2.3` | [binary build](https://launchpad.net/~ubuntu-security-proposed/+archive/ubuntu/ppa/+build/33437580); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/glibc/2.43-2ubuntu2.3/glibc_2.43-2ubuntu2.3.dsc) |
| `graphite2` | `graphite2 1.3.14-11ubuntu1.1` | [binary build](https://launchpad.net/~ubuntu-security-proposed/+archive/ubuntu/ppa/+build/32974941); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/graphite2/1.3.14-11ubuntu1.1/graphite2_1.3.14-11ubuntu1.1.dsc) |
| `harfbuzz` | `harfbuzz 12.3.2-2` | [binary build](https://launchpad.net/ubuntu/+source/harfbuzz/12.3.2-2/+build/32297210); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/harfbuzz/12.3.2-2/harfbuzz_12.3.2-2.dsc) |
| `hidapi` | `hidapi 0.15.0-2` | [binary build](https://launchpad.net/ubuntu/+source/hidapi/0.15.0-2/+build/31417613); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/hidapi/0.15.0-2/hidapi_0.15.0-2.dsc) |
| `icu` | `icu 78.2-2ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/icu/78.2-2ubuntu1/+build/32332270); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/icu/78.2-2ubuntu1/icu_78.2-2ubuntu1.dsc) |
| `libb2` | `libb2 0.98.1-1.1build2` | [binary build](https://launchpad.net/ubuntu/+source/libb2/0.98.1-1.1build2/+build/31571369); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libb2/0.98.1-1.1build2/libb2_0.98.1-1.1build2.dsc) |
| `libffi` | `libffi 3.5.2-4` | [binary build](https://launchpad.net/ubuntu/+source/libffi/3.5.2-4/+build/32741120); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libffi/3.5.2-4/libffi_3.5.2-4.dsc) |
| `libftdi` | `libftdi1 1.6~rc1-1build1` | [binary build](https://launchpad.net/ubuntu/+source/libftdi1/1.6~rc1-1build1/+build/32703562); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libftdi1/1.6~rc1-1build1/libftdi1_1.6~rc1-1build1.dsc) |
| `libglvnd` | `libglvnd 1.7.0-3` | [binary build](https://launchpad.net/ubuntu/+source/libglvnd/1.7.0-3/+build/32092483); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libglvnd/1.7.0-3/libglvnd_1.7.0-3.dsc) |
| `libpng` | `libpng1.6 1.6.57-1` | [binary build](https://launchpad.net/ubuntu/+source/libpng1.6/1.6.57-1/+build/32764090); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libpng1.6/1.6.57-1/libpng1.6_1.6.57-1.dsc) |
| `libusb` | `libusb-1.0 2:1.0.29-2build1` | [binary build](https://launchpad.net/ubuntu/+source/libusb-1.0/2:1.0.29-2build1/+build/32217540); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libusb-1.0/2:1.0.29-2build1/libusb-1.0_1.0.29-2build1.dsc) |
| `libx11` | `libx11 2:1.8.13-1` | [binary build](https://launchpad.net/ubuntu/+source/libx11/2:1.8.13-1/+build/32728394); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libx11/2:1.8.13-1/libx11_1.8.13-1.dsc) |
| `libxau` | `libxau 1:1.0.11-1build2` | [binary build](https://launchpad.net/ubuntu/+source/libxau/1:1.0.11-1build2/+build/32217588); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxau/1:1.0.11-1build2/libxau_1.0.11-1build2.dsc) |
| `libxcb` | `libxcb 1.17.0-2ubuntu1` | [binary build](https://launchpad.net/ubuntu/+source/libxcb/1.17.0-2ubuntu1/+build/32194472); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxcb/1.17.0-2ubuntu1/libxcb_1.17.0-2ubuntu1.dsc) |
| `libxcrypt` | `libxcrypt 1:4.5.1-1` | [binary build](https://launchpad.net/ubuntu/+source/libxcrypt/1:4.5.1-1/+build/31473341); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxcrypt/1:4.5.1-1/libxcrypt_4.5.1-1.dsc) |
| `libxdmcp` | `libxdmcp 1:1.1.5-2` | [binary build](https://launchpad.net/ubuntu/+source/libxdmcp/1:1.1.5-2/+build/32084060); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxdmcp/1:1.1.5-2/libxdmcp_1.1.5-2.dsc) |
| `xkbcommon` | `libxkbcommon 1.13.1-1` | [binary build](https://launchpad.net/ubuntu/+source/libxkbcommon/1.13.1-1/+build/32215073); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxkbcommon/1.13.1-1/libxkbcommon_1.13.1-1.dsc) |
| `zstd` | `libzstd 1.5.7+dfsg-3` | [binary build](https://launchpad.net/ubuntu/+source/libzstd/1.5.7+dfsg-3/+build/32112592); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libzstd/1.5.7+dfsg-3/libzstd_1.5.7+dfsg-3.dsc) |
| `md4c` | `md4c 0.5.2-2build1` | [binary build](https://launchpad.net/ubuntu/+source/md4c/0.5.2-2build1/+build/32177086); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/md4c/0.5.2-2build1/md4c_0.5.2-2build1.dsc) |
| `ncurses` | `ncurses 6.6+20251231-1` | [binary build](https://launchpad.net/ubuntu/+source/ncurses/6.6+20251231-1/+build/32107162); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/ncurses/6.6+20251231-1/ncurses_6.6+20251231-1.dsc) |
| `openssl` | `openssl 3.5.5-1ubuntu3.2` | [binary build](https://launchpad.net/~ubuntu-security/+archive/ubuntu/ppa/+build/32935698); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/openssl/3.5.5-1ubuntu3.2/openssl_3.5.5-1ubuntu3.2.dsc) |
| `pcre2` | `pcre2 10.46-1build1` | [binary build](https://launchpad.net/ubuntu/+source/pcre2/10.46-1build1/+build/32217812); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/pcre2/10.46-1build1/pcre2_10.46-1build1.dsc) |
| `qt` | `qt6-base 6.10.2+dfsg-7` | [binary build](https://launchpad.net/ubuntu/+source/qt6-base/6.10.2+dfsg-7/+build/32760364); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/qt6-base/6.10.2+dfsg-7/qt6-base_6.10.2+dfsg-7.dsc) |
| `readline` | `readline 8.3-4` | [binary build](https://launchpad.net/ubuntu/+source/readline/8.3-4/+build/32273751); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/readline/8.3-4/readline_8.3-4.dsc) |
| `sqlite` | `sqlite3 3.46.1-9ubuntu0.2` | [binary build](https://launchpad.net/~ubuntu-security-proposed/+archive/ubuntu/ppa/+build/33405975); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/sqlite3/3.46.1-9ubuntu0.2/sqlite3_3.46.1-9ubuntu0.2.dsc) |
| `systemd` | `systemd 259.5-0ubuntu3` | [binary build](https://launchpad.net/ubuntu/+source/systemd/259.5-0ubuntu3/+build/32771808); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/systemd/259.5-0ubuntu3/systemd_259.5-0ubuntu3.dsc) |
| `tcl` | `tcl8.6 8.6.17+dfsg-1build1` | [binary build](https://launchpad.net/ubuntu/+source/tcl8.6/8.6.17+dfsg-1build1/+build/32182697); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/tcl8.6/8.6.17+dfsg-1build1/tcl8.6_8.6.17+dfsg-1build1.dsc) |
| `util-linux` | `util-linux 2.41.3-3ubuntu2` | [binary build](https://launchpad.net/ubuntu/+source/util-linux/2.41.3-3ubuntu2/+build/32367518); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/util-linux/2.41.3-3ubuntu2/util-linux_2.41.3-3ubuntu2.dsc) |
| `xz` | `xz-utils 5.8.3-1` | [binary build](https://launchpad.net/ubuntu/+source/xz-utils/5.8.3-1/+build/32752765); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/xz-utils/5.8.3-1/xz-utils_5.8.3-1.dsc) |
| `zlib` | `zlib 1:1.3.dfsg+really1.3.1-1ubuntu3` | [binary build](https://launchpad.net/ubuntu/+source/zlib/1:1.3.dfsg+really1.3.1-1ubuntu3/+build/32182685); [.dsc](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/zlib/1:1.3.dfsg+really1.3.1-1ubuntu3/zlib_1.3.dfsg+really1.3.1-1ubuntu3.dsc) |

The actual Linux candidate EQY/SBY reports also supply their previously missing checkout pins:
`fb8b21160297b3e66bf94f3c24f8a8bf1cd3fd51` and `79fc9eddca5e33a28cc2f2cd19c9825efd9a7255`.
Their report hashes and the retained EQY plugins/SBY scripts are now recorded for Linux.

### GHDL source changes and nightly inputs

On **linux-x64**, the pinned `ghdl.sh` changes only `configure` before configuration:

```sh
sed -i 's,ghdl1-llvm,../bin/ghdl1-llvm,g' configure
```

The source target declares no patch files. The recovered exact unified diff is stored as
`ghdl-linux-known-recipe-edit` in the assembly plan, with its SHA-256 and original configure
file hash. The Linux-arm64 and Windows Makefile edits do not apply to this x86_64 package.
The default Linux invocation requests `llvm-config`; its exact LLVM source/version and the
retained linkage remain unresolved and require the original dependency manifest/link map.
The source archive contains the pristine `c966be32400b25310a6d1028ab3a3b8462a8dea5` tree, the known
diff, and the entire suite recipe; it does **not** claim the historical dirty state is exhausted
without the original build checkout/diff. Both source and known patch hashes are mapped explicitly.

For Mac `dd4f9740079b2fe79baf92b43d4a10a16254205a`, matching
[workflow job](https://github.com/ghdl/ghdl/actions/runs/34788715372/job/103808872353)
and [earlier job](https://github.com/ghdl/ghdl/actions/runs/34757621770/job/103724669825)
identify macos-14/aarch64, LLVM backend and **GNAT 14.2.0-1**. The pinned
[workflow](https://github.com/ghdl/ghdl/blob/dd4f9740079b2fe79baf92b43d4a10a16254205a/.github/workflows/Build-MacOS.yml)
uses unpinned `brew install llvm coreutils`, statically links Homebrew zstd and LLVM, and copies
GNAT runtime dylibs. Both job-log API requests returned **403**. The moving nightly release no
longer proves the historical asset ID/hash. The suite’s preferred pattern implies
`ghdl-llvm-7.0.0-dev-macos14-aarch64.tar.gz`; that name remains an inference, not recovered bytes.
The [GNAT release tag](https://github.com/alire-project/GNAT-FSF-builds/tree/70fbb4cbda40f826d166c175340c205f0dd780b0)
is archived as evidence, but its `specs/gcc.anod` selects **14.1.0**, conflicting with the
14.2.0-1 asset name. It is not treated as proof of the runtime’s corresponding source.

### macOS resource sources

The [v4 resource asset](https://github.com/mmicko/macos-resources/releases/tag/v4)
`macports-darwin-arm64.tar.gz` was verified against its published SHA-256
`3fd58dcd396da482b570e53991c92c3322e78bc17c63386d60ffabdfcf44897c`.
It contains the original `+PORTFILE`, `+STATE` and `+CONTENTS`. **24** shipped dylibs match the
resource members by Mach-O `LC_UUID` across relocation/signing. Their member hashes and UUIDs
are in JSON. Receipts identify MacPorts 2.12.5, variants and build-platform metadata.
The stored Portfiles match the following historical port-directory commits byte-for-byte;
all files/patches in those directories and the checksum-verified distfiles are archived.
Readline includes all three upstream 8.3 patches. These are matching directory snapshots:
the original **global ports-tree checkout and effective PortGroup/base inputs remain unresolved**.
`source_date_epoch` is supporting receipt evidence, not a global tree commit.

| Mac component | Recorded MacPorts release | Matching Portfile/tree evidence |
|---|---|---|
| `boost` | `boost176-1.76.0_15+no_single.darwin_25.arm64` | [a2c1cdffe96d](https://raw.githubusercontent.com/macports/macports-ports/a2c1cdffe96d169098ee9f3cce4cef0b03aab537/devel/boost176/Portfile) |
| `readline` | `readline-8.3.003_0.darwin_25.arm64` | [6d7de444cef1](https://raw.githubusercontent.com/macports/macports-ports/6d7de444cef10af39282b40a71baf3b5e76875f9/devel/readline/Portfile) |
| `libiconv` | `libiconv-1.18_0.darwin_25.arm64` | [a393aeeb96ee](https://raw.githubusercontent.com/macports/macports-ports/a393aeeb96eec86beba798192d9058a55a135111/textproc/libiconv/Portfile) |
| `gettext` | `gettext-runtime-1.0_0.darwin_25.arm64` | [24c1a4825e4d](https://raw.githubusercontent.com/macports/macports-ports/24c1a4825e4de9e86296ac87cdc73f936dc56eec/devel/gettext/Portfile) |
| `ncurses` | `ncurses-6.6_0.darwin_25.arm64` | [a940cd16e718](https://raw.githubusercontent.com/macports/macports-ports/a940cd16e718bff57c985954b8fdc766a8e57544/devel/ncurses/Portfile) |
| `bzip2` | `bzip2-1.0.8_0.darwin_25.arm64` | [640b7e0aa871](https://raw.githubusercontent.com/macports/macports-ports/640b7e0aa8710d26cd0af15bde29bb6290ad6b20/archivers/bzip2/Portfile) |
| `libffi` | `libffi-3.4.8_0.darwin_25.arm64` | [313ed2317d13](https://raw.githubusercontent.com/macports/macports-ports/313ed2317d1360afb4a960cba3b51d8beb7464f5/devel/libffi/Portfile) |
| `xz` | `xz-5.8.3_0.darwin_25.arm64` | [ff42455114d5](https://raw.githubusercontent.com/macports/macports-ports/ff42455114d515d01e8a164f5e5715eaf1e2628c/archivers/xz/Portfile) |
| `zlib` | `zlib-1.3.2_0.darwin_25.arm64` | [0320cd2bb6e5](https://raw.githubusercontent.com/macports/macports-ports/0320cd2bb6e5df05cda3f82822b54aad01f8e296/archivers/zlib/Portfile) |
| `zstd` | `zstd-1.5.7_0.darwin_25.arm64` | [1a6e6add29e1](https://raw.githubusercontent.com/macports/macports-ports/1a6e6add29e1dac24fb2a177d855b7fe28386397/archivers/zstd/Portfile) |
| `libusb` | `libusb-1.0.30_0.darwin_25.arm64` | [14aba96ae045](https://raw.githubusercontent.com/macports/macports-ports/14aba96ae045fcbbc5f658230d072bc319f11d77/devel/libusb/Portfile) |
| `libftdi` | `libftdi1-1.5_2.darwin_25.arm64` | [6661876dae72](https://raw.githubusercontent.com/macports/macports-ports/6661876dae72047671ade6a7eb1c8a886d7feb13/devel/libftdi/Portfile) |
| `hidapi` | `hidapi-0.15.0_0.darwin_25.arm64` | [3ed8dfa08ddc](https://raw.githubusercontent.com/macports/macports-ports/3ed8dfa08ddc563e08d515610db6ce73c4c3086f/comms/hidapi/Portfile) |
| `realpath` | `realpath-1.0.2_0.darwin_25.arm64` | [4bb54856ac0f](https://raw.githubusercontent.com/macports/macports-ports/4bb54856ac0fd78074ebd6abcc07385a9d1f6e87/sysutils/realpath/Portfile) |
| `openssl` | `openssl3-3.6.3_0.darwin_25.arm64` | [1f635e27620d](https://raw.githubusercontent.com/macports/macports-ports/1f635e27620d209183c8982006685aeeeaff57a0/devel/openssl3/Portfile) |
| `sqlite` | `sqlite3-3.53.3_0.darwin_25.arm64` | [2ca5f2a3c4fe](https://raw.githubusercontent.com/macports/macports-ports/2ca5f2a3c4fe55c9aeae0ca67a0d0fc0d2020e12/databases/sqlite3/Portfile) |
| `gdbm` | `gdbm-1.26_0.darwin_25.arm64` | [ae121b17c716](https://raw.githubusercontent.com/macports/macports-ports/ae121b17c7165d4110b2aa8324df93dd4b3f0611/databases/gdbm/Portfile) |

Qt is a separately built resource, not a MacPorts package. The
[pinned resource README](https://github.com/mmicko/macos-resources/blob/df5b8aba9019f8f44a10796e1c96576ef2750b51/README.md)
gives its configure invocation and the arm64 `arm_acle.h` edit in
`qtbase/src/corelib/thread/qyieldcpu.h`. The complete official Qt 6.10.2 source tarball is
checked against its [published SHA-256](https://download.qt.io/official_releases/qt/6.10/6.10.2/single/qt-everywhere-src-6.10.2.tar.xz.sha256)
and archived with that recipe tree. Its original build diff, CMakeCache and complete input record
are still needed to exclude additional changes.

### Zig 0.17.1

The [official download index](https://ziglang.org/download/index.json) identifies both:

| Item | Official source | Published SHA-256 |
|---|---|---|
| `zig-source` | [zig-0.17.1.tar.xz](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz) | `1e89bf06610b769a8c77676c6c8d54b093a0db60f4891f2f91014829012f0e24` |
| `zig-bootstrap` | [zig-bootstrap-0.17.1.tar.xz](https://ziglang.org/download/0.17.1/zig-bootstrap-0.17.1.tar.xz) | `58c099060ad9237496c7e7f57235d9eb9e5a3713e7df5f90d8ccc06391a359dd` |

Both tarballs and their published `.minisig` files are retained. The bootstrap payload supplies
its vendored LLVM/compiler dependencies; the source payload supplies the exact Zig-modified libc
files and update scripts. The old GitHub tag lookup is superseded by these canonical release
archives. This resolves source delivery, not the separate per-file libc/SDK licence review or
compiler/clean-machine proof conditions.

### Archive and manifest semantics

Items are grouped into `suite`, `ubuntu-runtime` and `macos-runtime`. Git checkouts include all
recursive submodules, including relative submodule URLs. Tar member order, uid/gid/names,
mtime and modes are normalized; mutable git-describe labels are excluded. Expanded checkouts
are removed after packing. Raw source tarballs and complete `.dsc` payloads are retained without
recompressing their internal source layout. `SOURCES.md` lists versions, URLs and SHA-256 values;
each `METADATA.json` records the complete inventory and nested notice paths.

A Git item’s source SHA-256 hashes its normalized source tar. A multifile item hashes the
canonical JSON list of `[filename, sha256]` pairs in the recorded order. Patch-set hashes cover
the deterministic inventory of all pinned suite build/patch inputs, distro patch payloads,
recovered MacPorts port/receipt files, and declared additional build inputs where applicable.
The full suite tree retains every audited script, patch, Dockerfile and workflow.

Every final regular toolchain file and symlink receives one JSONL record using package-relative
paths (`Contents/Resources/toolchain/...` or `share/toolchain/...`). Each component mapping adds
`source_part` and `source_parts`; unresolved source correspondence is explicit even when known
base sources were archived. Symlinks retain their literal target and a separate target-text hash.
Generated wrappers, environment, inventories, signing metadata and selection files reference
**this repository at the release commit**, with no repository archive. `--repository-revision`
can set that commit explicitly; the default is the assembler checkout HEAD. The source report
contains the complete unresolved list, fetch failures, coverage counts and measured part hashes.
Final packages must be rerun through the assembler. A completed audit run is not release clearance.

The original inspection methodology and size allowances below are retained as historical context;
actual final-package inventory and measured output replace those initial estimates.

<a id="initial-candidate-verification-superseded-by-the-coordinator-rerun"></a>

## Initial candidate verification (superseded by the source-assembly rerun)

Nine offline unit tests pass. Two complete fresh assembler runs produce identical source-part
and JSONL-manifest SHA-256 values. Independent verification rehashes every shipped regular file,
checks every literal symlink target, validates all **86** source items and their archived payloads,
and verifies the reconstructed Qt source SHA-256. Every part contains root `SOURCES.md`; each
asset is strictly below 2 GiB. The 1,315,359,412-byte Qt source tar is stored as two documented
fragments in the Mac part; concatenate their numeric suffixes and check the original checksum.

| Part | Bytes | SHA-256 (identical in both runs) |
|---|---:|---|
| `ecp5_emu-0.1.0-sources-suite.tar.xz` | 152756100 | `8c901c44327ce69efc1e27f2107367f8022edc17fb44fff01452c2f2ad711c7f` |
| `ecp5_emu-0.1.0-sources-ubuntu-runtime.tar.xz` | 568385596 | `4092f1448c6a8c6a3f042fc736c115c8fc7314b766cf1a14054af3e148944165` |
| `ecp5_emu-0.1.0-sources-macos-runtime.tar.xz` | 1518011308 | `bf55a4c035eb14b7c6afae6194872da3a1c00d4c797b2cff057b7a68dff24309` |

| Platform | Regular files | Symlinks | Manifest records | Missing |
|---|---:|---:|---:|---:|
| linux | 4176 | 2 | 4178 | 0 |
| macos | 2972 | 19 | 2991 | 0 |

The original requested Linux candidate disappeared during concurrent packaging work after the first
successful run. Its original **4,176-record** Linux manifest is preserved in
`out/source-archive/original-candidate-manifest.jsonl` (with the original Mac records). The
replacement `release/0.1.0/candidates/0b9b7c8` was checked against that inventory: every third-party
file is byte-identical, two generated files changed (`zig-0.17.1/clang` and `bundle-manifest.json`),
and it adds `clang-version.txt` plus `lib/yosys-abc`. The two final verification runs use stable
private snapshots of this Linux candidate and the originally requested Mac app. Input bytes are
now inventoried at startup, before long downloads/compression; cleanup after inventory is covered
by a regression test. These are candidate proofs, not a final release-commit/package proof.

The initial source cache measured 4106709130 bytes before deleting expanded
resolver trees, below the 8 GiB limit. Fetch failures were **0**. That initial report had
**23 explicit unresolved entries**, reclassified above. Current outputs
replace the initial files in `out/source-archive/dist/`; the current verification/report and
rerun measurements govern their status. PGP/Minisign signatures were retained without independent
signature verification; final packaging and compiler/clean-machine proofs remain separate operational work.

## Inputs and method (initial audit)

Only metadata, recipes, licence texts and individual source files were read over the network.
At that initial inspection stage, no binaries or source archives were downloaded. Local binaries were inspected with `otool -L`,
`strings`, property lists and version output. Local payload hashes are in `macos_input_files`;
`linux-facts.txt` supplies sizes and SHA-256 for all 67 native Linux files, now in
`linux_input_files` and each component's `file_metadata.linux`. These are input hashes, before any
package rewriting/signing; final package and source hashes are still required.

- macOS: `out/bundle-audit/closure-macos.txt`, 646 entries from
  `~/FPGA/oss-cad-suite-20261022`. The inventory includes data, directories and symlinks, not just
  executables. It contains **six** Qt frameworks, including QtDBus. Their Info.plists report 6.10.2.
- Linux: `out/bundle-audit/closure-linux.txt` has **68** entries, rather than the supplied description
  of 67: 67 paths under `lib/`, `libexec/` or `share/`, plus an unqualified `openFPGALoader` entry.
  All are retained; the last entry's package-relative location/hash is still unknown. The
  reports `out/bundle-audit/linux-facts.txt` and `linux-versions.txt` record `sha256sum` and embedded
  version strings from the suite's own `~/apps/oss-cad-suite-20261022/lib/*.so` and `libexec/*`,
  with tool `--version` output run through its private loader. No host-installed package metadata
  was used; the server's OS is irrelevant. The second report covers the whole suite, not just the
  selected closure. The reports' hashes are recorded in JSON.
- The Mac's suite `VERSION` is `20261022`. Its 53 `license/LICENSE.*` files identify tool sources,
  revisions and nested notices; they also cover tools outside this subset. They are not one licence
  covering every copied library. The Linux facts also report VERSION `20261022` and 56 licence
  filenames. This is a listing, not the contents of all 56 reports; suite-built components retain
  the upstream report/recipe revisions already recorded, corroborated by tool version lines where
  supplied. Library source-package mappings are inferred from the bundled files' version strings
  and the suite recipe's Ubuntu 26.04 build image.
- The trimmed Python 3.11 stdlib is absent from both lists. Linux's full `share/yosys` and `lib/ghdl`
  trees are absent too. Patterns for these required additions are explicit, not invented file lists.
- Zig candidate: `/private/tmp/ecp5_zig/zig-aarch64-macos-0.17.1`; `zig version` says `0.17.1`.
  All 16 681 regular files under `lib/libc` are inventoried as proposed files, without implying that
  other-platform headers will ship or that the final pruning decision has been made.

The [release API](https://api.github.com/repos/YosysHQ/oss-cad-suite-build/releases/tags/2026-10-22)
names tag `2026-10-22`; its
[tag ref](https://api.github.com/repos/YosysHQ/oss-cad-suite-build/git/ref/tags/2026-10-22) resolves to
`cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0`. Every suite recipe/patch reference below is relative to
that commit, never a current branch head. The release's `target_commitish: main` alone was not used
as a source pin. Moving source branches are resolved with the installed component reports where
available; a report is not silently treated as proof for a different platform or nightly payload.

### macOS openFPGALoader extension

Recursive `otool -L` from `libexec/openFPGALoader` adds these four entries to the Mac inventory:

```text
libexec/openFPGALoader
lib/libusb-1.0.0.dylib
lib/libftdi1.2.5.0.dylib
lib/libhidapi.0.dylib
```

Its `lib/libz.1.3.2.dylib` was already listed, giving 650 entries in the extended union. The loader
also uses host libc++/libSystem; USB libraries use libobjc, CoreFoundation, IOKit and Security.
Those system libraries/frameworks are not copied. The input text files are left unchanged; the
extended list and hashes are recorded in the JSON. This checks the loader's dependency closure,
not the completeness of the as-yet-unlisted Python extension-module closure.

## Tools, plugins and copied data

The report/recipe revisions below are retained for suite-built tools, not replaced by Ubuntu
packages merely because a compiler string names Ubuntu. The Linux version output confirms Yosys
`485b63564`, nextpnr `6b438a1c`, ecppack `a31ff70`, GHDL `c966be324` and openFPGALoader `v1.2.0`.
Python 3.11 and plugin revisions remain the suite recipe/report selections. Each archive item
supplies the full selected tree, nested dependencies, notices and exact build/patch inputs.

| Component | Version/evidence | SPDX; copyleft kind | Files | Exact source / unresolved source |
|---|---|---|---|---|
| `yosys` | macos: 0.70+190 (485b63564); linux: 0.70+190 (485b63564) | ISC; none | macos: 322 exact entries in JSON; linux: `libexec/yosys` | macos: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4); linux: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `nextpnr` | macos: 0.11.1-77-g6b438a1c; linux: 0.11.1-77-g6b438a1c | ISC; none | macos: `libexec/nextpnr-ecp5`; linux: `libexec/nextpnr-ecp5` | macos: [6b438a1c9ab83cfb872fea21d9545d7d0bb8573f](https://github.com/YosysHQ/nextpnr/tree/6b438a1c9ab83cfb872fea21d9545d7d0bb8573f); linux: [6b438a1c9ab83cfb872fea21d9545d7d0bb8573f](https://github.com/YosysHQ/nextpnr/tree/6b438a1c9ab83cfb872fea21d9545d7d0bb8573f) |
| `prjtrellis` | macos: commit in suite report; linux: ecppack 1.4-86-ga31ff70 | ISC; none | macos: `libexec/ecppack`, `libexec/ecpbram`; linux: `libexec/ecppack`, `libexec/ecpbram` | macos: [a31ff7042e712c0e2ad217054116ce2c4995f5c7](https://github.com/YosysHQ/prjtrellis/tree/a31ff7042e712c0e2ad217054116ce2c4995f5c7); linux: [a31ff7042e712c0e2ad217054116ce2c4995f5c7](https://github.com/YosysHQ/prjtrellis/tree/a31ff7042e712c0e2ad217054116ce2c4995f5c7) |
| `ghdl` | macos: 7.0.0-dev (6.0.0.r504.gdd4f97400), GNAT compiler 14.2.0, LLVM backend 23.1.0; linux: 7.0.0-dev (6.0.0.r548.gc966be324.dirty) | GPL-2.0-or-later; GPL | macos: `libexec/ghdl`, `lib/libghdl-7_0_0_dev.dylib`; linux: `libexec/ghdl`, `lib/libghdl-7_0_0_dev.so` | macos: [dd4f9740079b2fe79baf92b43d4a10a16254205a](https://github.com/ghdl/ghdl/tree/dd4f9740079b2fe79baf92b43d4a10a16254205a); linux: [c966be32400b25310a6d1028ab3a3b8462a8dea5](https://github.com/ghdl/ghdl/tree/c966be32400b25310a6d1028ab3a3b8462a8dea5) |
| `ghdl-yosys-plugin` | macos: commit in suite report; linux: suite report commit 6a145898ea21c558b9c1da0c13ac82c91e0c69a7 | GPL-3.0-or-later; GPL | macos: `share/yosys/plugins/ghdl.so`; linux: `share/yosys/plugins/ghdl.so` | macos: [6a145898ea21c558b9c1da0c13ac82c91e0c69a7](https://github.com/ghdl/ghdl-yosys-plugin/tree/6a145898ea21c558b9c1da0c13ac82c91e0c69a7); linux: [6a145898ea21c558b9c1da0c13ac82c91e0c69a7](https://github.com/ghdl/ghdl-yosys-plugin/tree/6a145898ea21c558b9c1da0c13ac82c91e0c69a7) |
| `yosys-slang-plugin` | macos: commit in suite report; linux: suite report commit 3e152133b9c22b7883a6a03d483ed35e8c74aba5 | ISC; none | macos: `share/yosys/plugins/slang.so`; linux: `share/yosys/plugins/slang.so` | macos: [3e152133b9c22b7883a6a03d483ed35e8c74aba5](https://github.com/povik/yosys-slang/tree/3e152133b9c22b7883a6a03d483ed35e8c74aba5); linux: [3e152133b9c22b7883a6a03d483ed35e8c74aba5](https://github.com/povik/yosys-slang/tree/3e152133b9c22b7883a6a03d483ed35e8c74aba5) |
| `openfpgaloader` | macos: commit in suite report; linux: v1.2.0 | Apache-2.0; none | macos: `libexec/openFPGALoader`; linux: `libexec/openFPGALoader` | macos: [26f96b4294b83108ee66fbfbe4d73bc35d9634a8](https://github.com/trabucayre/openFPGALoader/tree/26f96b4294b83108ee66fbfbe4d73bc35d9634a8); linux: [26f96b4294b83108ee66fbfbe4d73bc35d9634a8](https://github.com/trabucayre/openFPGALoader/tree/26f96b4294b83108ee66fbfbe4d73bc35d9634a8) |
| `python` | macos: 3.11.6; linux: 3.11.6 (suite recipe; ABI 3.11 in hashed library) | PSF-2.0; none | macos: `lib/libpython3.11.dylib`; linux: `lib/libpython3.11.so.1.0` | macos: [8b6ee5ba3bd08368ba3c763cea28ce1464a88e62](https://github.com/python/cpython/tree/8b6ee5ba3bd08368ba3c763cea28ce1464a88e62); linux: [8b6ee5ba3bd08368ba3c763cea28ce1464a88e62](https://github.com/python/cpython/tree/8b6ee5ba3bd08368ba3c763cea28ce1464a88e62) |
| `eqy` | macos: commit in suite report; linux: not enumerated in supplied Linux input | ISC; none | macos: 4 exact entries in JSON | macos: [fb8b21160297b3e66bf94f3c24f8a8bf1cd3fd51](https://github.com/YosysHQ/eqy/tree/fb8b21160297b3e66bf94f3c24f8a8bf1cd3fd51); linux: [unknown](https://github.com/YosysHQ/eqy) |
| `sby` | macos: commit in suite report; linux: not enumerated in supplied Linux input | ISC; none | macos: 16 exact entries in JSON | macos: [79fc9eddca5e33a28cc2f2cd19c9825efd9a7255](https://github.com/YosysHQ/sby/tree/79fc9eddca5e33a28cc2f2cd19c9825efd9a7255); linux: [unknown](https://github.com/YosysHQ/sby) |
| `ghdl-data` | macos: bundled lib/ghdl tree; source correspondence to nightly unknown; linux: not enumerated in Linux closure; required addition | NOASSERTION; GPL | macos: 233 exact entries in JSON | macos: [dd4f9740079b2fe79baf92b43d4a10a16254205a](https://github.com/ghdl/ghdl/tree/dd4f9740079b2fe79baf92b43d4a10a16254205a); linux: [c966be32400b25310a6d1028ab3a3b8462a8dea5](https://github.com/ghdl/ghdl/tree/c966be32400b25310a6d1028ab3a3b8462a8dea5) |
| `llvm-ghdl` | macos: 23.1.0 (--version); exact source/build unknown; linux: unknown whether/which LLVM code retained in GHDL binary | Apache-2.0 WITH LLVM-exception; none | embedded/proposed patterns in JSON | macos: [unknown](https://github.com/llvm/llvm-project) |
| `realpath` | macos: 1.0.2 (strings and Docker image); linux: not in supplied closure | BSD-2-Clause; none | macos: `libexec/realpath` | macos: [v1.0.2](https://github.com/user454322/realpath/tree/v1.0.2) |
| `suite-wrappers` | both: cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0 | ISC; none | linux: `openFPGALoader` | both: [cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0](https://github.com/YosysHQ/oss-cad-suite-build/tree/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0) |
| `trellis-database` | both: package pin 9754ff473bfaa1e6edfaf57ce13bd447c76a88f6 | CC0-1.0; none | macos: `share/trellis/database` | both: [9754ff473bfaa1e6edfaf57ce13bd447c76a88f6](https://github.com/YosysHQ/prjtrellis-db/tree/9754ff473bfaa1e6edfaf57ce13bd447c76a88f6) |

### GHDL report versus shipped nightly

The Mac's `LICENSE.ghdl` names `c966be32400b25310a6d1028ab3a3b8462a8dea5`, but the actual executable
reports `7.0.0-dev (6.0.0.r504.gdd4f97400)`, GNAT compiler 14.2.0 and LLVM backend 23.1.0.
The full `dd4f9740079b2fe79baf92b43d4a10a16254205a` appears in libghdl strings too.
[The recipe][recipe-ghdl] fetches a moving upstream nightly asset on macOS and changes install
names/signatures. Its nominal source checkout does not establish that asset's source provenance.
Recover the asset name/hash, upstream workflow revision, GCC/LLVM inputs, patches and build scripts.
The Linux executable instead reports `7.0.0-dev (6.0.0.r548.gc966be324.dirty)`, matching report
commit `c966be32400b25310a6d1028ab3a3b8462a8dea5` with a dirty tree. Keep that source and the suite's
patch/inline-edit set, including `default/scripts/ghdl.sh` configure edits; capture any additional
source diff before archive assembly. The version line does not prove that recipe edits account for
all changes. Its GCC strings include 14.3.0 and 15.2.0 compiler builds; those are not the version
of the separately bundled libgnat, nor evidence for an LLVM backend version.

Do not label all `lib/ghdl` files GPL: inspected IEEE sources and `ieee2008/LICENSE` use Apache-2.0;
Synopsys files have their own permissive notice; `std/v93/textio.vhdl` is GPL-2.0-or-later.
VITAL/UPF and every compiled library object's originating licence need a complete map.
[GRT's linking exception][ghdl-runtime] is a custom exception, not the GCC runtime exception;
retain the text with the GPL source. The main program and libghdl still require their source.

### Embedded Yosys and slang dependencies

These are additional component records, including the separate ABC executable. Gitlink revisions
come from the pinned Yosys/plugin trees and their `.gitmodules`; source directories without a
separate gitlink are preserved verbatim in the pinned Yosys source item. Keep all nested notices
from `LICENSE.yosys` and `LICENSE.yosys-slang-plugin`, not just their top-level ISC texts.

| Component | Version/evidence | SPDX; copyleft kind | Files | Exact source / unresolved source |
|---|---|---|---|---|
| `abc` | both: gitlink 6c5104934c90eb3cdef9ea063318f7588908a185 | LicenseRef-ABC-permissive; none | macos: `libexec/yosys-abc`; linux: `libexec/yosys-abc` | both: [6c5104934c90eb3cdef9ea063318f7588908a185](https://github.com/YosysHQ/abc/tree/6c5104934c90eb3cdef9ea063318f7588908a185) |
| `boost-regex` | both: gitlink 2b3ac0834f31086c6e3c0e0ceb8516e427d5c39d | BSL-1.0; none | embedded/proposed patterns in JSON | both: [2b3ac0834f31086c6e3c0e0ceb8516e427d5c39d](https://github.com/MikePopoloski/regex/tree/2b3ac0834f31086c6e3c0e0ceb8516e427d5c39d) |
| `cxxopts` | both: gitlink 4bf61f08697b110d9e3991864650a405b3dd515d | MIT; none | embedded/proposed patterns in JSON | both: [4bf61f08697b110d9e3991864650a405b3dd515d](https://github.com/jarro2783/cxxopts/tree/4bf61f08697b110d9e3991864650a405b3dd515d) |
| `fmt-yosys` | both: gitlink a00f82c6b66dcae15503ba19c1598b2733057dea | MIT; none | embedded/proposed patterns in JSON | both: [a00f82c6b66dcae15503ba19c1598b2733057dea](https://github.com/fmtlib/fmt/tree/a00f82c6b66dcae15503ba19c1598b2733057dea) |
| `slang` | both: gitlink 545f88b2ed3a2d20325fbd17413af177a6bca72f | MIT; none | embedded/proposed patterns in JSON | both: [545f88b2ed3a2d20325fbd17413af177a6bca72f](https://github.com/MikePopoloski/slang/tree/545f88b2ed3a2d20325fbd17413af177a6bca72f) |
| `symfpu` | both: gitlink 5d5e50867437fe5cabd6ab5aad3b3808ec86701f | BSD-3-Clause; none | embedded/proposed patterns in JSON | both: [5d5e50867437fe5cabd6ab5aad3b3808ec86701f](https://github.com/YosysHQ/symfpu/tree/5d5e50867437fe5cabd6ab5aad3b3808ec86701f) |
| `tomlplusplus` | both: gitlink a43ad3787293f4a46b1d70c0924b5a25d10e79fc | MIT; none | embedded/proposed patterns in JSON | both: [a43ad3787293f4a46b1d70c0924b5a25d10e79fc](https://github.com/marzer/tomlplusplus/tree/a43ad3787293f4a46b1d70c0924b5a25d10e79fc) |
| `sv-elab` | both: gitlink 8aa302273bb97bd7d839c005f94dab1523f3c432 | ISC; none | embedded/proposed patterns in JSON | both: [8aa302273bb97bd7d839c005f94dab1523f3c432](https://github.com/povik/sv-elab/tree/8aa302273bb97bd7d839c005f94dab1523f3c432) |
| `fmt-plugin` | both: 1be298e1bd68957e4cd352e1f676f00e07dcfb57 | MIT; none | embedded/proposed patterns in JSON | both: [1be298e1bd68957e4cd352e1f676f00e07dcfb57](https://github.com/fmtlib/fmt/tree/1be298e1bd68957e4cd352e1f676f00e07dcfb57) |
| `yosys-bigint` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | LicenseRef-public-domain; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `yosys-flex` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | BSD-3-Clause; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `yosys-libfst` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | MIT; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `yosys-json11` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | MIT; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `yosys-minisat` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | MIT; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |
| `yosys-sha1` | both: vendored at Yosys 485b635644b853bec391faa3e41293079726cac4 | LicenseRef-public-domain; none | embedded/proposed patterns in JSON | both: [485b635644b853bec391faa3e41293079726cac4](https://github.com/YosysHQ/yosys/tree/485b635644b853bec391faa3e41293079726cac4) |

The slang source gitlink is the same in both inspected parents; fmt has two different revisions.
Do not deduplicate them by name alone. Yosys's `dlfcn-win32` notice was read, but that Windows-only
component is marked not shipped on these platforms. ABC's permissive text and the public-domain
statements are retained with explicit `LicenseRef` identifiers rather than guessed SPDX matches.
Yosys's embedded libfst notice also credits LZ4; nested notice/source completeness remains to check.
nextpnr's compiled-in/header dependencies, including Eigen, Boost headers and pybind11 where used,
need the historical build dependency manifest; its short ISC report does not close that audit.

## Runtime libraries

The initial inferred runtime mappings are superseded by the exact Launchpad binary/source matches
and MacPorts receipt recovery above. Ubuntu sources include their complete distro patch/build
payload. MacPorts reconstruction includes matching Portfiles/patches and verified distfiles,
while the global ports-tree/PortGroup gap remains open. Qt source and documented resource-build
instructions are archived separately.

The following SPDX entries identify upstream component terms, not proof that every byte of an
unknown distributor version has been audited. Alternatives remain unselected where noted.

| Component | Version/evidence | SPDX; copyleft kind | Files | Exact source / unresolved source |
|---|---|---|---|---|
| `boost` | macos: boost176-1.76.0_15+no_single.darwin_25.arm64; linux: 1.90.0-6ubuntu1 | BSL-1.0; none | macos: 7 exact entries in JSON; linux: `lib/libboost_filesystem.so.1.90.0`, `lib/libboost_program_options.so.1.90.0`, `lib/libboost_thread.so.1.90.0` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/a2c1cdffe96d169098ee9f3cce4cef0b03aab537/devel/boost176/Portfile); linux: [1.90.0-6ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/boost1.90/1.90.0-6ubuntu1/boost1.90_1.90.0-6ubuntu1.dsc) |
| `qt` | macos: 6.10.2; linux: 6.10.2+dfsg-7 | LGPL-3.0-only; LGPL | macos: 42 exact entries in JSON; linux: 6 exact entries in JSON | macos: [6.10.2](https://download.qt.io/official_releases/qt/6.10/6.10.2/single/qt-everywhere-src-6.10.2.tar.xz); linux: [6.10.2+dfsg-7](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/qt6-base/6.10.2+dfsg-7/qt6-base_6.10.2+dfsg-7.dsc) |
| `glibc` | linux: 2.43-2ubuntu2.3 | LGPL-2.1-or-later; LGPL | linux: `lib/ld-linux-x86-64.so.2`, `lib/libc.so.6`, `lib/libm.so.6` | linux: [2.43-2ubuntu2.3](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/glibc/2.43-2ubuntu2.3/glibc_2.43-2ubuntu2.3.dsc) |
| `gcc-libgcc` | macos: unknown (GCC ABI 1; GHDL compiler 14.2.0 is not runtime provenance); linux: 16-20260322-1ubuntu1 | GPL-3.0-or-later WITH GCC-exception-3.1; runtime exception | macos: `lib/libgcc_s.1.1.dylib`; linux: `lib/libgcc_s.so.1` | macos: [unknown (GCC ABI 1; GHDL compiler 14.2.0 is not runtime provenance)](https://github.com/gcc-mirror/gcc); linux: [16-20260322-1ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-16/16-20260322-1ubuntu1/gcc-16_16-20260322-1ubuntu1.dsc) |
| `gcc-libstdcxx` | linux: 16-20260322-1ubuntu1 | GPL-3.0-or-later WITH GCC-exception-3.1; runtime exception | linux: `lib/libstdc++.so.6` | linux: [16-20260322-1ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-16/16-20260322-1ubuntu1/gcc-16_16-20260322-1ubuntu1.dsc) |
| `gcc-libgnat` | macos: unknown (major ABI 14); linux: 14.3.0-14ubuntu1 | GPL-3.0-or-later WITH GCC-exception-3.1; runtime exception | macos: `lib/libgnat-14.dylib`; linux: `lib/libgnat-14.so` | macos: [unknown (major ABI 14)](https://github.com/gcc-mirror/gcc); linux: [14.3.0-14ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-14/14.3.0-14ubuntu1/gcc-14_14.3.0-14ubuntu1.dsc) |
| `gcc-libatomic` | linux: 16-20260322-1ubuntu1 | GPL-3.0-or-later WITH GCC-exception-3.1; runtime exception | linux: `lib/libatomic.so.1` | linux: [16-20260322-1ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-16/16-20260322-1ubuntu1/gcc-16_16-20260322-1ubuntu1.dsc) |
| `gcc-libgomp` | linux: 16-20260322-1ubuntu1 | GPL-3.0-or-later WITH GCC-exception-3.1; runtime exception | linux: `lib/libgomp.so.1` | linux: [16-20260322-1ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/gcc-16/16-20260322-1ubuntu1/gcc-16_16-20260322-1ubuntu1.dsc) |
| `readline` | macos: readline-8.3.003_0.darwin_25.arm64; linux: 8.3-4 | GPL-3.0-or-later; GPL | macos: `lib/libreadline.8.3.dylib`; linux: `lib/libreadline.so.8` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/6d7de444cef10af39282b40a71baf3b5e76875f9/devel/readline/Portfile); linux: [8.3-4](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/readline/8.3-4/readline_8.3-4.dsc) |
| `libiconv` | macos: libiconv-1.18_0.darwin_25.arm64 | LGPL-2.1-or-later; LGPL | macos: `lib/libiconv.2.dylib` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/a393aeeb96eec86beba798192d9058a55a135111/textproc/libiconv/Portfile) |
| `gettext` | macos: gettext-runtime-1.0_0.darwin_25.arm64 | LGPL-2.1-or-later; LGPL | macos: `lib/libintl.8.dylib` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/24c1a4825e4de9e86296ac87cdc73f936dc56eec/devel/gettext/Portfile) |
| `ncurses` | macos: ncurses-6.6_0.darwin_25.arm64; linux: 6.6+20251231-1 | MIT; none | macos: `lib/libncurses.6.dylib`; linux: `lib/libtinfo.so.6` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/a940cd16e718bff57c985954b8fdc766a8e57544/devel/ncurses/Portfile); linux: [6.6+20251231-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/ncurses/6.6+20251231-1/ncurses_6.6+20251231-1.dsc) |
| `bzip2` | macos: bzip2-1.0.8_0.darwin_25.arm64; linux: 1.0.8-6build2 | bzip2-1.0.6; none | macos: `lib/libbz2.1.0.8.dylib`; linux: `lib/libbz2.so.1.0` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/640b7e0aa8710d26cd0af15bde29bb6290ad6b20/archivers/bzip2/Portfile); linux: [1.0.8-6build2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/bzip2/1.0.8-6build2/bzip2_1.0.8-6build2.dsc) |
| `libffi` | macos: libffi-3.4.8_0.darwin_25.arm64; linux: 3.5.2-4 | MIT; none | macos: `lib/libffi.8.dylib`; linux: `lib/libffi.so.8` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/313ed2317d1360afb4a960cba3b51d8beb7464f5/devel/libffi/Portfile); linux: [3.5.2-4](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libffi/3.5.2-4/libffi_3.5.2-4.dsc) |
| `xz` | macos: xz-5.8.3_0.darwin_25.arm64; linux: 5.8.3-1 | 0BSD; none | macos: `lib/liblzma.5.dylib` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/ff42455114d515d01e8a164f5e5715eaf1e2628c/archivers/xz/Portfile); linux: [5.8.3-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/xz-utils/5.8.3-1/xz-utils_5.8.3-1.dsc) |
| `zlib` | macos: zlib-1.3.2_0.darwin_25.arm64; linux: 1:1.3.dfsg+really1.3.1-1ubuntu3 | Zlib; none | macos: `lib/libz.1.3.2.dylib`; linux: `lib/libz.so.1` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/0320cd2bb6e5df05cda3f82822b54aad01f8e296/archivers/zlib/Portfile); linux: [1:1.3.dfsg+really1.3.1-1ubuntu3](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/zlib/1:1.3.dfsg+really1.3.1-1ubuntu3/zlib_1.3.dfsg+really1.3.1-1ubuntu3.dsc) |
| `zstd` | macos: zstd-1.5.7_0.darwin_25.arm64; linux: 1.5.7+dfsg-3 | BSD-3-Clause; none | macos: `lib/libzstd.1.5.7.dylib`; linux: `lib/libzstd.so.1` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/1a6e6add29e1dac24fb2a177d855b7fe28386397/archivers/zstd/Portfile); linux: [1.5.7+dfsg-3](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libzstd/1.5.7+dfsg-3/libzstd_1.5.7+dfsg-3.dsc) |
| `libusb` | macos: libusb-1.0.30_0.darwin_25.arm64; linux: 2:1.0.29-2build1 | LGPL-2.1-or-later; LGPL | macos: `lib/libusb-1.0.0.dylib`; linux: `lib/libusb-1.0.so.0` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/14aba96ae045fcbbc5f658230d072bc319f11d77/devel/libusb/Portfile); linux: [2:1.0.29-2build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libusb-1.0/2:1.0.29-2build1/libusb-1.0_1.0.29-2build1.dsc) |
| `libftdi` | macos: libftdi1-1.5_2.darwin_25.arm64; linux: 1.6~rc1-1build1 | LGPL-2.0-only; LGPL | macos: `lib/libftdi1.2.5.0.dylib`; linux: `lib/libftdi1.so.2` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/6661876dae72047671ade6a7eb1c8a886d7feb13/devel/libftdi/Portfile); linux: [1.6~rc1-1build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libftdi1/1.6~rc1-1build1/libftdi1_1.6~rc1-1build1.dsc) |
| `hidapi` | macos: hidapi-0.15.0_0.darwin_25.arm64; linux: 0.15.0-2 | BSD-3-Clause; none | macos: `lib/libhidapi.0.dylib`; linux: `lib/libhidapi-libusb.so.0` | macos: [matching Portfile; global inputs unresolved](https://raw.githubusercontent.com/macports/macports-ports/3ed8dfa08ddc563e08d515610db6ce73c4c3086f/comms/hidapi/Portfile); linux: [0.15.0-2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/hidapi/0.15.0-2/hidapi_0.15.0-2.dsc) |
| `icu` | linux: 78.2-2ubuntu1 | Unicode-3.0; none | linux: `lib/libicudata.so.78`, `lib/libicui18n.so.78`, `lib/libicuuc.so.78` | linux: [78.2-2ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/icu/78.2-2ubuntu1/icu_78.2-2ubuntu1.dsc) |
| `libglvnd` | linux: 1.7.0-3 | NOASSERTION; none | linux: 4 exact entries in JSON | linux: [1.7.0-3](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libglvnd/1.7.0-3/libglvnd_1.7.0-3.dsc) |
| `libx11` | linux: 2:1.8.13-1 | MIT; none | linux: `lib/libX11.so.6` | linux: [2:1.8.13-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libx11/2:1.8.13-1/libx11_1.8.13-1.dsc) |
| `libxau` | linux: 1:1.0.11-1build2 | NOASSERTION; unknown | linux: `lib/libXau.so.6` | linux: [1:1.0.11-1build2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxau/1:1.0.11-1build2/libxau_1.0.11-1build2.dsc) |
| `libxdmcp` | linux: 1:1.1.5-2 | NOASSERTION; unknown | linux: `lib/libXdmcp.so.6` | linux: [1:1.1.5-2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxdmcp/1:1.1.5-2/libxdmcp_1.1.5-2.dsc) |
| `libxcb` | linux: 1.17.0-2ubuntu1 | NOASSERTION; unknown | linux: `lib/libxcb.so.1` | linux: [1.17.0-2ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxcb/1.17.0-2ubuntu1/libxcb_1.17.0-2ubuntu1.dsc) |
| `libb2` | linux: 0.98.1-1.1build2 | CC0-1.0; none | linux: `lib/libb2.so.1` | linux: [0.98.1-1.1build2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libb2/0.98.1-1.1build2/libb2_0.98.1-1.1build2.dsc) |
| `brotli` | linux: 1.2.0-3build1 | MIT; none | linux: `lib/libbrotlicommon.so.1`, `lib/libbrotlidec.so.1` | linux: [1.2.0-3build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/brotli/1.2.0-3build1/brotli_1.2.0-3build1.dsc) |
| `dbus` | linux: 1.16.2-2ubuntu4 | AFL-2.1 OR GPL-2.0-or-later; unknown | linux: `lib/libdbus-1.so.3` | linux: [1.16.2-2ubuntu4](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/dbus/1.16.2-2ubuntu4/dbus_1.16.2-2ubuntu4.dsc) |
| `double-conversion` | linux: 3.4.0-1 | BSD-3-Clause; none | linux: `lib/libdouble-conversion.so.3` | linux: [3.4.0-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/double-conversion/3.4.0-1/double-conversion_3.4.0-1.dsc) |
| `expat` | linux: 2.7.4-1 | MIT; none | linux: `lib/libexpat.so.1` | linux: [2.7.4-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/expat/2.7.4-1/expat_2.7.4-1.dsc) |
| `fontconfig` | linux: 2.17.1-3ubuntu1 | MIT; none | linux: `lib/libfontconfig.so.1` | linux: [2.17.1-3ubuntu1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/fontconfig/2.17.1-3ubuntu1/fontconfig_2.17.1-3ubuntu1.dsc) |
| `freetype` | linux: 2.14.2+dfsg-1ubuntu0.1 | FTL OR GPL-2.0-or-later; unknown | linux: `lib/libfreetype.so.6` | linux: [2.14.2+dfsg-1ubuntu0.1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/freetype/2.14.2+dfsg-1ubuntu0.1/freetype_2.14.2+dfsg-1ubuntu0.1.dsc) |
| `glib` | linux: 2.88.0-1 | LGPL-2.1-or-later; LGPL | linux: `lib/libglib-2.0.so.0` | linux: [2.88.0-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/glib2.0/2.88.0-1/glib2.0_2.88.0-1.dsc) |
| `graphite2` | linux: 1.3.14-11ubuntu1.1 | LGPL-2.1-or-later OR MPL-1.1 OR GPL-2.0-or-later; unknown | linux: `lib/libgraphite2.so.3` | linux: [1.3.14-11ubuntu1.1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/graphite2/1.3.14-11ubuntu1.1/graphite2_1.3.14-11ubuntu1.1.dsc) |
| `harfbuzz` | linux: 12.3.2-2 | MIT; none | linux: `lib/libharfbuzz.so.0` | linux: [12.3.2-2](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/harfbuzz/12.3.2-2/harfbuzz_12.3.2-2.dsc) |
| `md4c` | linux: 0.5.2-2build1 | MIT; none | linux: `lib/libmd4c.so.0` | linux: [0.5.2-2build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/md4c/0.5.2-2build1/md4c_0.5.2-2build1.dsc) |
| `pcre2` | linux: 10.46-1build1 | NOASSERTION; none | linux: `lib/libpcre2-16.so.0`, `lib/libpcre2-8.so.0` | linux: [10.46-1build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/pcre2/10.46-1build1/pcre2_10.46-1build1.dsc) |
| `libpng` | linux: 1.6.57-1 | Libpng-2.0; none | linux: `lib/libpng16.so.16` | linux: [1.6.57-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libpng1.6/1.6.57-1/libpng1.6_1.6.57-1.dsc) |
| `systemd` | linux: 259.5-0ubuntu3 | LGPL-2.1-or-later; LGPL | linux: `lib/libsystemd.so.0`, `lib/libudev.so.1` | linux: [259.5-0ubuntu3](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/systemd/259.5-0ubuntu3/systemd_259.5-0ubuntu3.dsc) |
| `tcl` | linux: 8.6.17+dfsg-1build1 | TCL; none | linux: `lib/libtcl8.6.so` | linux: [8.6.17+dfsg-1build1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/tcl8.6/8.6.17+dfsg-1build1/tcl8.6_8.6.17+dfsg-1build1.dsc) |
| `xkbcommon` | linux: 1.13.1-1 | NOASSERTION; none | linux: `lib/libxkbcommon.so.0` | linux: [1.13.1-1](https://launchpad.net/ubuntu/+archive/primary/+sourcefiles/libxkbcommon/1.13.1-1/libxkbcommon_1.13.1-1.dsc) |

- **Runtime exceptions:** retain GCC `COPYING3` and `COPYING.RUNTIME`. Linux libgcc_s, libstdc++,
  libatomic and libgomp map to `gcc-16` `16-20260322-1ubuntu1`; libgnat maps separately to `gcc-14`
  `14.3.0-14ubuntu1`. Include both Ubuntu source/patch sets. Mac runtime provenance remains separate
  and unresolved. Verify exception conditions and GNAT-specific notices from the actual sources.
- **Qt:** the six Mac frameworks are dynamically linked. Preserve qtbase's third-party notices,
  exact distributor changes, source/build information and the applicable LGPL replacement/relinking
  provisions. Linux Core/Gui strings report 6.10.2; exact binary matching now establishes
  Ubuntu `qt6-base` `6.10.2+dfsg-7`, whose full `.dsc` payload is archived.
- **USB:** local `LICENSE.libusb` records `c6a35c56016ea2ab2f19115d2ea1e85e0edae155` for a source
  target at 1.0.24, while the Darwin image names the linked library as 1.0.30. Do not use the former
  as provenance for the latter. `LICENSE.libftdi` records `5c2c58e03ea999534e8cb64906c8ae8b15536c30`
  and says GNU Library GPL version 2 for the C library. Confirm its actual source headers and
  COPYING.LIB; do not substitute the C++ wrapper's separate GPL/exception terms or assume LGPL-2.1.
  HIDAPI offers alternatives; the audited proposal selects its BSD-3-Clause option.
- **Other alternatives:** D-Bus, FreeType and Graphite licence choices remain unresolved until their
  exact sources and distributor selections are inspected. Preserve every applicable embedded notice.
- **Absent versus unlisted:** no `libssl`/`libcrypto`, `liblzma` or `libintl` Linux path appears in
  the input list. That does not prove the final Python extension set needs none. Inspect its
  retained `_ssl`, `_hashlib`, `_lzma`, `_ctypes`, `_sqlite3`, `_bz2`, pyexpat and other modules;
  add dependencies and source items if retained. The wider suite report identifies OpenSSL,
  Mesa and util-linux versions, but does not add those libraries to this selected payload:

| Observed outside the selected closure | Ubuntu source package/version | Scope |
|---|---|---|
| `libssl.so.3`, `libcrypto.so.3` | [openssl 3.5.5-1ubuntu3.2](https://launchpad.net/ubuntu/+source/openssl/3.5.5-1ubuntu3.2) | Version report only; no selected-file hash or archive allowance |
| `libgallium`, `libgbm`, `libGLX_mesa` | [mesa 26.0.3-1ubuntu1](https://launchpad.net/ubuntu/+source/mesa/26.0.3-1ubuntu1) | Version report only; selected EGL/GLX/GLdispatch/OpenGL libraries map to `libglvnd` |
| `libblkid`, `libmount`, `libuuid` | [util-linux 2.41.3-3ubuntu2](https://launchpad.net/ubuntu/+source/util-linux/2.41.3-3ubuntu2) | Version report only; outside the selected closure |

These source package/version pairs were confirmed in Launchpad metadata and are stored as
`linux_suite_observations_outside_closure`, not as additional shipped component/file records.

## Proposed Zig compiler and libc support

The root [Zig licence][zig-home] in the inspected copy is MIT; it is not a licence for all of
`lib/libc`. Zig's local README requires LLVM/Clang/LLD 22.x for source builds, which does not
identify the exact sources embedded in this binary. The initial GitHub `0.17.1` lookup returned 404; the canonical official source and bootstrap
archives above now resolve source delivery. No Git commit is inferred from the directory name.

The initial compiler libc inventory is grouped below, including non-host targets. Every Zig row
now maps to the official 0.17.1 source/bootstrap archives above; remaining upstream-lineage and
per-file licence uncertainties do not mean the exact Zig-modified files are missing. Each group has
an exact candidate file list in JSON. All remain proposed: prune only after the compiler tests,
then regenerate the final shipped-file/hash manifest. Group licence expressions are incomplete
where `NOASSERTION` appears; the JSON preserves observed terms separately.

| Component | Version/evidence | SPDX; copyleft kind | Files | Exact source / unresolved source |
|---|---|---|---|---|
| `zig` | macos: 0.17.1 (local zig version); linux: 0.17.1 proposed; binary not inspected | MIT; none | proposed: `zig`, `LICENSE`, `README.md` | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-llvm` | proposed: 22.x required by local README; actual binary source revision unknown | Apache-2.0 WITH LLVM-exception; none | embedded/proposed patterns in JSON | both: [official 0.17.1 bootstrap](https://ziglang.org/download/0.17.1/zig-bootstrap-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-darwin` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; unknown | proposed: `lib/libc/darwin/SDKSettings.json`, `lib/libc/darwin/libSystem.tbd` | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-darwin-headers` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; APSL; per-file audit incomplete | proposed: 574 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-freebsd` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; none | proposed: 1910 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-glibc` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; LGPL | proposed: 1078 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-linux-uapi` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; syscall exception; mixed per-file terms | proposed: 1365 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-mingw` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; unknown | proposed: 4791 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-musl` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | MIT AND BSD-2-Clause; none | proposed: 2671 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-netbsd` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; none | proposed: 2180 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-openbsd` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | NOASSERTION; none | proposed: 1759 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |
| `zig-libc-wasi` | proposed: vendored in inspected Zig 0.17.1; upstream revision unknown | Apache-2.0 WITH LLVM-exception AND Apache-2.0 AND MIT AND BSD-2-Clause AND CC0-1.0; none | proposed: 351 exact entries in JSON | both: [official 0.17.1 modified source](https://ziglang.org/download/0.17.1/zig-0.17.1.tar.xz); published SHA-256 in JSON |

The inspection read directory COPYING/COPYRIGHT/LICENSE files and file headers. In particular:

- `include/generic-glibc/time.h` and `features.h` state LGPL-2.1-or-later; preserve glibc's mixed
  notices in `glibc/LICENSES`, original sources and Zig's modifications/update scripts.
- `include/any-darwin-any/stdio.h` contains APSL-2.0 and a BSD notice;
  `mach/mach_time.h` contains APSL-2.0 and Apple's OS-reference condition. These are not covered
  by Zig's MIT text. `darwin/libSystem.tbd` and `SDKSettings.json` have no licence header;
  the latter says 27.0, which does not establish the stub's upstream licence or source revision.
- Linux UAPI headers include GPL/LGPL expressions with `Linux-syscall-note`, plus alternative
  permissive licences. Preserve each exact expression and exception; do not treat them as the
  unrestricted MIT licence or assume they impose the ordinary GPL on generated applications.
- Musl's COPYRIGHT identifies MIT and BSD portions. MinGW's COPYING names ZPL-2.1 with public-domain,
  BSD and LGPL exceptions. FreeBSD's COPYRIGHT includes BSD notices and the Berkeley advertising
  clause rescission. NetBSD/OpenBSD headers carry their own BSD/ISC notices. WASI's LICENSE lists
  Apache/LLVM, Apache, MIT and component-specific BSD/CC0 terms. A directory heading does not
  establish a complete per-file expression for all these files.

## Not shipped as standalone components

Rust/Cargo are build-time tools, not copied into the package. Rust standard-library and crate code
is embedded in the existing programs; it remains in the existing [licence ledger](../../licenses/README.md)
and generated Rust notices. This audit does not declare that code absent or replace that ledger.
Apple's libSystem, libc++, libobjc and system frameworks are host-provided and are not copied;
Zig's proposed Darwin headers/stubs are distinct shipped files needing their own audit.
The unrelated Windows `dlfcn-win32` source is also excluded from this platform subset.

## Source archive contents

**Release decision, 2026-11-18:** publish exact corresponding sources beside each binary release,
not a written offer. The original assembly plan follows; the implementation and resolution evidence above supersede
its previously unknown source identities. Every component's JSON `source_archive` says how it will be provided. Exact or honestly identified closest source snapshots and recorded patch/recipe sets are supplied
under the source-delivery criterion above; mutable branch links alone are not the source delivery.

Include:

1. Full suite recipe tree at `cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0`, including `src/base.py`,
   `default/rules`, `default/scripts`, `default/patches`, `scripts`, Dockerfiles and relevant build
   workflows. Retain all referenced patch files, inline substitutions and generated wrapper inputs.
2. Full pinned tool trees and all nested sources, including git submodules which `git archive`
   would otherwise omit. Keep distinct Yosys/plugin fmt revisions. Include both GHDL source
   identities: Mac `dd4f9740079b2fe79baf92b43d4a10a16254205a` and Linux
   `c966be32400b25310a6d1028ab3a3b8462a8dea5` plus its dirty source changes. The Mac report's nominal
   checkout is not enough for that binary.
3. Exact Ubuntu/MacPorts library sources, kept separately by platform/version. For Ubuntu include
   the `.dsc`, every referenced original tarball and Debian/Ubuntu diff or `debian.tar` payload;
   preserve `debian/patches/series`, all patches and `debian/rules`/configuration. Verify their
   checksums and retain the suite's packaging/relocation scripts and historical build metadata.
   In particular, Linux GCC 16 and GCC 14 need separate source packages, in addition to unresolved
   Mac runtimes. Boost 1.90/1.76 and Linux/macOS zlib are also separate source inputs. Never replace
   the recorded Ubuntu source with a similarly numbered bare upstream release.
4. Upstream GHDL nightly build scripts and exact LLVM/GCC dependencies; candidate Zig source,
   bootstrap/compiler sources and libc provenance only if Zig passes its approval conditions.
5. Project wrapper, patching, pruning, signing and packaging scripts at their release revisions,
   with complete build invocations/configuration. Record generated files and local modifications.
   Include all needed licence and exception texts; provide LGPL replacement/relinking information
   and applicable installation information rather than assuming a source URL alone is sufficient.

### Patch and build inputs at the suite commit

| Source item | Applied files or inline operations to retain |
|---|---|
| Yosys / ABC | `default/rules/yosys.py`, `default/scripts/yosys.sh`; generated compiler configuration, build flags, nested gitlinks |
| nextpnr / Trellis | `default/rules/nextpnr.py`, `default/scripts/nextpnr-ecp5.sh`, `prjtrellis.sh`, `prjtrellis-bba.sh`, `nextpnr-bba.sh`; inline CMake minimum-version edit in `prjtrellis.sh` |
| GHDL | `default/rules/ghdl.py`, `default/scripts/ghdl.sh`; Linux `c966be324` dirty diff and configure edits; Mac `dd4f9740` nightly extraction, install-name edits and signing; missing upstream nightly build inputs |
| GHDL plugin | `default/scripts/ghdl-yosys-plugin.sh`; yosys-config and Mac Makefile substitutions |
| slang plugin | `default/scripts/yosys-slang-plugin.sh`; Windows.h spelling, yosys-config paths and `ConstraintAnalysis.h` edits |
| Python | `default/rules/python.py`, `default/scripts/python3.sh`; `default/patches/python3.11.6-darwin.diff` on Mac, inline setup.py/configure edits; no Linux patch file applied there |
| openFPGALoader | `default/rules/programmers.py`, `default/scripts/openfpgaloader.sh`; `ENABLE_UDEV=OFF` |
| EQY / SBY | `default/rules/eqy.py`, `formal.py`; `default/scripts/eqy.sh`, `sby.sh`; copied plugins/scripts included by full share tree |
| Closure libraries | Exact Ubuntu `.dsc`, orig and Debian/Ubuntu patch/build payloads at inferred versions; unknown versions still need bundled-file identity checks; MacPorts inputs remain separate |
| All payloads | `scripts/package-darwin.sh`, `scripts/package-linux.sh`, `default/scripts/system-resources.sh` and associated resources; wrappers, rpaths, strip/sign operations |
| Zig | Exact release/update/bootstrap scripts and vendored modifications still unknown; preserve local headers and their notices |

### Size allowances by source item

These are planning estimates in decimal MB **compressed**, not measured source download sizes.
This describes the initial audit; source recovery and downloads are recorded above. They assume duplicate source items are stored once only after
identity is established; extra platform-specific GCC/LLVM/distributor trees can exceed the range.
A zero entry means included in another item or excluded, not that the source is unnecessary.

| Source item | Components/contents | Estimated compressed MB |
|---|---|---|
| `yosys` | `yosys` | 10–25 |
| `nextpnr` | `nextpnr` | 10–30 |
| `prjtrellis` | `prjtrellis` | 5–20 |
| `ghdl` | `ghdl`, `ghdl-data` | 20–50 |
| `ghdl-yosys-plugin` | `ghdl-yosys-plugin` | 1–3 |
| `yosys-slang-plugin` | `yosys-slang-plugin` | 1–5 |
| `openfpgaloader` | `openfpgaloader` | 2–10 |
| `python` | `python` | 20–35 |
| `eqy` | `eqy` | 1–5 |
| `sby` | `sby` | 1–5 |
| `abc` | `abc` | 5–15 |
| `boost-regex` | `boost-regex` | 1–10 |
| `cxxopts` | `cxxopts` | 1–10 |
| `fmt-yosys` | `fmt-yosys` | 1–10 |
| `slang` | `slang` | 5–20 |
| `symfpu` | `symfpu` | 1–10 |
| `tomlplusplus` | `tomlplusplus` | 1–10 |
| `sv-elab` | `sv-elab` | 1–10 |
| `fmt-plugin` | `fmt-plugin` | 1–5 |
| `yosys-bigint` | `yosys-bigint` | 1–5 |
| `yosys-flex` | `yosys-flex` | 1–5 |
| `yosys-libfst` | `yosys-libfst` | 1–5 |
| `yosys-json11` | `yosys-json11` | 1–5 |
| `yosys-minisat` | `yosys-minisat` | 1–5 |
| `yosys-sha1` | `yosys-sha1` | 1–5 |
| `dlfcn-win32` | `dlfcn-win32` | 0–0 |
| `ubuntu-boost1.90-1.90.0-6ubuntu1` | `boost` | 80–150 |
| `macos-boost` | `boost` | 80–150 |
| `ubuntu-qt6-base-unknown` | `qt` | 50–100 |
| `macos-qt` | `qt` | 50–100 |
| `ubuntu-glibc-2.43-2ubuntu2.3` | `glibc` | 20–45 |
| `ubuntu-gcc-16-16-20260322-1ubuntu1` | `gcc-libgcc`, `gcc-libstdcxx`, `gcc-libatomic`, `gcc-libgomp` | 90–160 |
| `macos-gcc-runtimes-unknown` | `gcc-libgcc`, `gcc-libgnat` | 90–160 |
| `ubuntu-gcc-14-14.3.0-14ubuntu1` | `gcc-libgnat` | 90–160 |
| `ubuntu-readline-unknown` | `readline` | 3–6 |
| `macos-readline` | `readline` | 3–6 |
| `libiconv` | `libiconv` | 5–10 |
| `gettext` | `gettext` | 15–35 |
| `ubuntu-ncurses-unknown` | `ncurses` | 3–6 |
| `macos-ncurses` | `ncurses` | 3–6 |
| `ubuntu-bzip2-unknown` | `bzip2` | 1–2 |
| `macos-bzip2` | `bzip2` | 1–2 |
| `ubuntu-libffi-unknown` | `libffi` | 1–3 |
| `macos-libffi` | `libffi` | 1–3 |
| `xz` | `xz` | 1–4 |
| `ubuntu-zlib-1:1.3.dfsg+really1.3.1-1ubuntu3` | `zlib` | 1–3 |
| `macos-zlib` | `zlib` | 1–3 |
| `ubuntu-libzstd-unknown` | `zstd` | 2–5 |
| `macos-zstd` | `zstd` | 2–5 |
| `ubuntu-libusb-1.0-unknown` | `libusb` | 1–3 |
| `macos-libusb` | `libusb` | 1–3 |
| `ubuntu-libftdi1-unknown` | `libftdi` | 1–3 |
| `macos-libftdi` | `libftdi` | 1–3 |
| `ubuntu-hidapi-unknown` | `hidapi` | 1–3 |
| `macos-hidapi` | `hidapi` | 1–3 |
| `ubuntu-icu-78.2-2ubuntu1` | `icu` | 25–50 |
| `ubuntu-libglvnd-unknown` | `libglvnd` | 1–3 |
| `ubuntu-libx11-unknown` | `libx11` | 2–5 |
| `ubuntu-libxau-unknown` | `libxau` | 1–2 |
| `ubuntu-libxdmcp-unknown` | `libxdmcp` | 1–2 |
| `ubuntu-libxcb-1.17.0-2ubuntu1` | `libxcb` | 1–3 |
| `ubuntu-libb2-unknown` | `libb2` | 1–2 |
| `ubuntu-brotli-unknown` | `brotli` | 1–5 |
| `ubuntu-dbus-1.16.2-2ubuntu4` | `dbus` | 1–5 |
| `ubuntu-double-conversion-unknown` | `double-conversion` | 1–3 |
| `ubuntu-expat-unknown` | `expat` | 1–3 |
| `ubuntu-fontconfig-2.17.1-3ubuntu1` | `fontconfig` | 1–5 |
| `ubuntu-freetype-unknown` | `freetype` | 2–5 |
| `ubuntu-glib2.0-unknown` | `glib` | 5–12 |
| `ubuntu-graphite2-1.3.14-11ubuntu1.1` | `graphite2` | 2–6 |
| `ubuntu-harfbuzz-unknown` | `harfbuzz` | 10–30 |
| `ubuntu-md4c-unknown` | `md4c` | 1–3 |
| `ubuntu-pcre2-unknown` | `pcre2` | 2–5 |
| `ubuntu-libpng1.6-unknown` | `libpng` | 1–3 |
| `ubuntu-systemd-259.5-0ubuntu3` | `systemd` | 10–30 |
| `ubuntu-tcl8.6-unknown` | `tcl` | 5–15 |
| `ubuntu-libxkbcommon-unknown` | `xkbcommon` | 1–5 |
| `llvm-ghdl` | `llvm-ghdl` | 150–300 |
| `realpath` | `realpath` | 1–1 |
| `suite-wrappers` | `suite-wrappers` | 1–5 |
| `trellis-database` | `trellis-database` | 20–90 |
| `zig` | `zig`, `zig-libc-darwin`, `zig-libc-darwin-headers`, `zig-libc-freebsd`, `zig-libc-glibc`, `zig-libc-linux-uapi`, `zig-libc-mingw`, `zig-libc-musl`, `zig-libc-netbsd`, `zig-libc-openbsd`, `zig-libc-wasi` | 25–60 |
| `llvm-zig (deduplicate only after exact identity known)` | `zig-llvm` | 0–200 |

The revised working allowance is **966–2318 MB (about 1.0–2.3 GB) compressed**, replacing
642–1714 MB. Separate Ubuntu GCC 16/GCC 14 sources and Mac runtime inputs, plus distinct platform
library source packages, account for the increase. The table's `ubuntu-*` items carry complete
Ubuntu source-package versions where known; `unknown` is not a source version to publish.
No archives were downloaded or measured. Expanded size and actual compressed size remain unknown;
deduplicate only after source hashes establish identity. Further dirty-tree inputs, runtime
variants or additional Python dependencies may increase the allowance. Out-of-closure OpenSSL,
Mesa and util-linux observations are not included unless the selected payload is expanded.

### Release manifest

Use JSON Lines with one record for every final regular payload file and explicit records for
symlinks. Paths are relative to the package root; never store a build user's absolute paths.
Hash the actual shipped bytes after relocation/signing, not just the original suite files.
A binary may map to several component/source items when code is statically embedded:

```json
{"platform":"macos-arm64","binary_path":"toolchain/libexec/ghdl","sha256":"<final-binary-sha256>","components":[{"component":"ghdl","source_item":"ghdl-macos","source_part":"ecp5_emu-<version>-sources-suite.tar.xz","source_revision":"dd4f9740079b2fe79baf92b43d4a10a16254205a","source_sha256":"<source-tree-or-tarball-sha256>","patch_set_sha256":"<patches-and-build-scripts-sha256>"}],"recipe_revision":"cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0","licence_files":["<archive-relative-notice-path>"]}
```

The example has placeholders, not release hashes. Add all other embedded GHDL components before
using it. Hash every source item and a deterministic inventory of its patches/build scripts; record
upstream archive checksums where a recipe supplies one. For symlinks record the literal target and
hash its text separately. Directory markers do not have a regular-file content hash.
The machine-readable input manifest hashes the closure lists, both new Linux reports, readable Mac
payloads and all 67 native Linux files. Component `file_metadata.linux` records size, SHA-256 and
strings; `linux_input_files` also maps every path to its component. The unqualified wrapper marker,
final packaged bytes and source archives still need their own hashes.

## Current obligation and provenance status

- Copyleft source supply: satisfied under the exact/closest-source criterion above, conditional on
  successful checksum-verified inclusion of the listed items; the generated report records failures.
- Historical GHDL workspace/nightly identity and the GNAT 14.1 recipe versus 14.2 asset discrepancy
  remain stated provenance facts with closest sources and recorded build/patch inputs supplied.
- Exact LLVM/permissive build inputs and MacPorts base/PortGroups are provenance unknown, not required.
- File-level elections and LGPL replacement/runtime-exception information are above and in every
  archive README; the proposed notices addition is in the report, without editing notices.py.
- Rerun for final signed/relocated packages and the release commit. Binary/compiler clean-machine
  proofs remain separate operational work, not provenance source-delivery blockers.

[recipe-ghdl]: https://github.com/YosysHQ/oss-cad-suite-build/blob/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0/default/scripts/ghdl.sh
[ghdl-runtime]: https://github.com/ghdl/ghdl/blob/dd4f9740079b2fe79baf92b43d4a10a16254205a/src/grt/ghdl_main.adb
[docker-linux]: https://github.com/YosysHQ/oss-cad-suite-build/blob/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0/docker/cross-linux-x64/Dockerfile
[docker-mac]: https://github.com/YosysHQ/oss-cad-suite-build/blob/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0/docker/cross-darwin-arm64/Dockerfile
[package-linux]: https://github.com/YosysHQ/oss-cad-suite-build/blob/cc0058b9e9f0a0301c0e8eb72f51f2200b637fe0/scripts/package-linux.sh
[zig-home]: https://ziglang.org/

[ubuntu-zlib]: https://launchpad.net/ubuntu/+source/zlib/1:1.3.dfsg+really1.3.1-1ubuntu3
