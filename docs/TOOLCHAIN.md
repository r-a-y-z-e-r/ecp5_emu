# Toolchain

Yosys synthesises Verilog; nextpnr-ecp5 places and routes it; Trellis's `ecppack` creates
bitstreams and `ecpbram` replaces block RAM contents. The emulator loads the Trellis database
and checks its files against the hashes in
[model_manifest.json](inventory/LFE5U-85F/model_manifest.json).

The shared libtrellis and its C++ headers are needed to build `trellis_image_reader`, the native
reader for `.config` and `.bit` files used by the window, `run`, `load` and `emulate`. They are
separate from the FPGA tools. Clang builds the reader and fast-mode libraries; Rust 1.92.0 builds
the emulator and window. The reader still needs the shared library when it runs.

| Component | Pinned version |
|---|---|
| Rust | 1.92.0, in [rust-toolchain.toml](../rust-toolchain.toml) |
| Yosys | 0.70+190, git `485b63564`, from OSS CAD Suite 2026-10-22 |
| nextpnr | 0.11.1-77-g6b438a1c; commit `6b438a1c9ab83cfb872fea21d9545d7d0bb8573f` |
| prjtrellis | `a31ff7042e712c0e2ad217054116ce2c4995f5c7` |
| prjtrellis-db | `9754ff473bfaa1e6edfaf57ce13bd447c76a88f6` |
| OSS CAD Suite | [2026-10-22][oss-release], the archive selected by CI |

The database is hash-checked; other tool versions can place a design differently. The critical-path
view parses only nextpnr 0.11.1 logs, checked against the routed JSON's creator version
([timing_path.rs](../src/timing_path.rs), commit `6ea9c44`). The recorded references used Homebrew
Yosys 0.70, reporting `0.70+post`; the suite supplies `0.70+190`. Builds with the suite are
functionally equivalent but not byte-identical to the recorded configurations. The record below
keeps the original tool versions and rechecks.

The commands below create a checkout at `$HOME/fpga/ecp5_emu`; run later commands from its root
unless a command changes directory. Clone the public repository with the HTTPS command below.
The Linux report tested `21c9df2`, with a geometry-cache recheck at `5ac466e`; see the
testing status below. Rustup installs Rust 1.92.0 from `rust-toolchain.toml` on the first Cargo call.
Keep the exported `PATH` and `ECP5_DB` in the shell where you build and run the project.

## Install on macOS (Apple Silicon)

These steps mirror the `macos-15` arm64 job in [ci.yml](../.github/workflows/ci.yml).
Every step is **untested on a clean machine**; see the testing status below.

1. Install Xcode command line tools (including clang and Git) first and wait for them to finish.
   **Untested on a clean machine.**

   ```sh
   xcode-select --install
   ```

   Then install [Homebrew](https://brew.sh/) following its instructions, if needed, and have Python 3
   available. Install the native dependencies and rustup, then get the source:

   ```sh
   brew install cmake ninja boost
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
   . "$HOME/.cargo/env"
   mkdir -p "$HOME/fpga"
   git clone https://github.com/r-a-y-z-e-r/ecp5_emu "$HOME/fpga/ecp5_emu"
   cd "$HOME/fpga/ecp5_emu"
   ```

2. Download CI's FPGA tools, check the archive, extract it and put its `bin` first on `PATH`.
   **Untested on a clean machine.** Allow about 0.7 GB for the archive and about 2.5 GB unpacked.
   The [upstream installation instructions][oss-readme] say to remove quarantine from the archive
   before extraction if macOS has quarantined it; omit the `xattr` line if the attribute is absent.

   ```sh
   mkdir -p "$HOME/fpga"
   suite_release=https://github.com/YosysHQ/oss-cad-suite-build/releases/download/2026-10-22
   suite_archive="$HOME/fpga/oss-cad-suite-darwin-arm64-20261022.tgz"
   curl --fail --location --retry 3 \
     "$suite_release/oss-cad-suite-darwin-arm64-20261022.tgz" -o "$suite_archive"
   echo "378e87e60db8cb3d077c63613cbed75e52306cf5ebdfeb95596629f343e536c0  $suite_archive" \
     | shasum -a 256 -c
   # Only if the downloaded archive has the quarantine attribute:
   xattr -d com.apple.quarantine "$suite_archive"
   tar -xzf "$suite_archive" -C "$HOME/fpga"
   export PATH="$HOME/fpga/oss-cad-suite/bin:$PATH"
   ```

3. Fetch the database at CI's pinned commit. **Untested on a clean machine.**

   ```sh
   export ECP5_DB="$HOME/fpga/prjtrellis-db"
   TRELLIS_DB_COMMIT=9754ff473bfaa1e6edfaf57ce13bd447c76a88f6
   git init -q "$ECP5_DB"
   git -C "$ECP5_DB" fetch -q --depth 1 https://github.com/YosysHQ/prjtrellis-db "$TRELLIS_DB_COMMIT"
   git -C "$ECP5_DB" checkout -q FETCH_HEAD
   ```

4. Build the pinned shared libtrellis and retain its source headers.
   **Untested on a clean machine.**

   ```sh
   bash packaging/build_trellis.sh
   ```

   The script fetches prjtrellis at the table's commit into `target/packaging/prjtrellis`, builds
   outside that checkout, and installs the shared library under
   `target/packaging/trellis-prefix/lib/trellis`. It disables Python bindings and the Trellis
   executables: the FPGA tools come from the suite. Headers remain in
   `target/packaging/prjtrellis/libtrellis/include`; they are not installed into the prefix.

5. Build the image reader and the two reference designs. **Untested on a clean machine.**

   ```sh
   bash packaging/build_test_fixtures.sh
   ```

   On macOS this calls `tools/build_image_reader.py` with `--trellis` pointing to the source
   checkout, `--prefix` to the installed library, and `CPATH` extended by `$(brew --prefix)/include`.
   It writes `target/trellis_image_reader`, then builds `blinky_cycle` and `uart_hello` under
   `target/reference`. The tour uses the former. Keep the library prefix and source checkout.

6. Build both Rust programs with the lockfile. **Untested on a clean machine.**

   ```sh
   cargo build --release --locked -p ecp5_emu -p ecp5_gui
   ```

7. Check database loading and open the guided tour. **Untested on a clean machine.**

   ```sh
   target/release/ecp5_emu device
   target/release/ecp5_gui --tour
   ```

## Install on Linux (x86_64)

These steps use CI's `ubuntu-24.04` dependencies and pinned tools, with source acquisition and
bootstrap added for a fresh system. The CLI path ran in a fresh Ubuntu 24.04 container on
2026-11-17–18 with the setup deviations described below; the revised bootstrap order and HTTPS
clone have not been rerun. The window was not exercised.

1. Install the native prerequisites before rustup, then get the source.
   **Prerequisite components ran in the container; this corrected sequence was not rerun.**

   ```sh
   sudo apt-get update
   sudo apt-get install -y build-essential git curl python3 clang cmake ninja-build \
     libboost-filesystem-dev libboost-program-options-dev libboost-thread-dev \
     libx11-dev libxext-dev libxrandr-dev libxi-dev libxcursor-dev libxinerama-dev libxfixes-dev \
     libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev wayland-protocols \
     libegl1-mesa-dev libgl1-mesa-dev libvulkan-dev
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
   . "$HOME/.cargo/env"
   mkdir -p "$HOME/fpga"
   git clone https://github.com/r-a-y-z-e-r/ecp5_emu "$HOME/fpga/ecp5_emu"
   cd "$HOME/fpga/ecp5_emu"
   ```

2. Download CI's FPGA tools, check the archive, extract it and put its `bin` first on `PATH`.
   The archive is 748 265 110 bytes and unpacks to about 2.5 GB.
   **Checksum and extraction checked with a seeded archive; no full in-container download timed.**

   ```sh
   mkdir -p "$HOME/fpga"
   suite_release=https://github.com/YosysHQ/oss-cad-suite-build/releases/download/2026-10-22
   suite_archive="$HOME/fpga/oss-cad-suite-linux-x64-20261022.tgz"
   curl --fail --location --retry 3 \
     "$suite_release/oss-cad-suite-linux-x64-20261022.tgz" -o "$suite_archive"
   echo "45b7401d677f1e6cffb19dcaafb8e92c95a49dfc05b332ae4d00b84c94f62f6e  $suite_archive" \
     | sha256sum -c
   tar -xzf "$suite_archive" -C "$HOME/fpga"
   export PATH="$HOME/fpga/oss-cad-suite/bin:$PATH"
   ```

3. Fetch the database at CI's pinned commit. **Ran in the fresh container.**

   ```sh
   export ECP5_DB="$HOME/fpga/prjtrellis-db"
   TRELLIS_DB_COMMIT=9754ff473bfaa1e6edfaf57ce13bd447c76a88f6
   git init -q "$ECP5_DB"
   git -C "$ECP5_DB" fetch -q --depth 1 https://github.com/YosysHQ/prjtrellis-db "$TRELLIS_DB_COMMIT"
   git -C "$ECP5_DB" checkout -q FETCH_HEAD
   ```

4. Build the pinned shared libtrellis and retain its source headers.
   **Ran in the fresh container.**

   ```sh
   bash packaging/build_trellis.sh
   ```

   This uses the same source checkout and library prefix as the macOS steps, with Python bindings
   and the Trellis executables disabled. The shared library is under
   `target/packaging/trellis-prefix/lib/trellis`; headers stay in the source checkout's
   `libtrellis/include`.

5. Build the image reader and the two reference designs. **Ran in the fresh container.**

   ```sh
   bash packaging/build_test_fixtures.sh
   ```

   On Linux the script invokes `clang++` directly with the source headers and the installed shared
   library, recording its directory as an rpath. It writes `target/trellis_image_reader`, then
   builds `target/reference/blinky_cycle` and `target/reference/uart_hello`.

6. Build both Rust programs with the lockfile. **Ran in the fresh container.**

   ```sh
   cargo build --release --locked -p ecp5_emu -p ecp5_gui
   ```

7. Check database loading and emulate the built counter. **Both CLI checks ran in the container.**

   ```sh
   target/release/ecp5_emu device
   target/release/ecp5_emu emulate target/reference/blinky_cycle/top.bit \
     --startup configured --scope observed \
     --clock pin:G2 --cycles 16 --probe pin:B2 --vcd run.vcd
   ```

   B2 is `led[0]`, which toggles each cycle; the command writes `run.vcd`. In a graphical session,
   open the guided tour. **The tour was not validated in the headless container.**

   ```sh
   target/release/ecp5_gui --tour
   ```

For either platform, [`Database::discover`](../src/database.rs) uses `--db`, then `ECP5_DB`.
Without either, it tries `~/FPGA/prjtrellis/database`, then `share/trellis/database` and
`share/trellis` under `/usr/local`, followed by `/opt/homebrew`. An automatic candidate must contain
`ECP5/LFE5U-85F/tilegrid.json`; the loader still checks the inventory hashes.
The [image loader](../src/image.rs) uses `--image-reader`, then `ECP5_IMAGE_READER`, then this
checkout's `target/trellis_image_reader`. The fixture script builds that default location.

## Testing status of these steps

The install commands use [CI's](../.github/workflows/ci.yml) pinned database, FPGA suite and
libtrellis; the [release workflow](../.github/workflows/release.yml) uses the same pins.
The Linux CLI path was tested on **2026-11-17–18 UTC**, starting at `21c9df2`, in a
fresh x86_64 LXD container from `ubuntu:24.04` on rserv (Ubuntu 24.04.5 LTS, user `ubuntu`).
The corrected bootstrap order above and the HTTPS clone command have not been rerun as a sequence.

At the time of this record the repository was private, so the source was a git-bundle checkout.
Slow networking required a seeded apt cache (204 packages, about 320 MB, still checked against the signed apt index) and a
suite tarball downloaded elsewhere, copied to the guide's path and checksum-verified inside the
container. The URL returned the same length, but no complete in-container download was timed.
Step 6 ran before step 5 to use waiting time; separate shells sourced the saved exports.
LXD launch, shell-newline and timeout/lock failures in the harness were corrected and affected
steps rerun. Seeded inputs and overlapping steps prevent a meaningful total install-time claim.

The database fetch, shared libtrellis, reader, both reference designs, both Rust binaries, `device`,
bitstream/named-net emulation, UART fast mode and the SDRAM peripheral example succeeded.
No window session or physical-board loading was tested: the headless `--tour` attempt exited 1
because no display was set. **The macOS install steps were not run**, including its download,
quarantine and Homebrew steps. Earlier helper runs on the maintainer's Mac and a Linux server,
and the server's fresh-copy CI dry run with tools already installed, were not clean-machine tests.

At `21c9df2`, `cargo test --release --locked` failed because the default geometry-cache test needed
an unbuilt picorv32_console fixture. Dev `5ac466e` has that test ignored by default and builds its
fixture on demand when selected. Its same-container default geometry-cache recheck gave **4 passed,
2 ignored**; the ignored on-demand path and the full suite were not rechecked there. Keep the test
command below for dev with that fix; no extra fixture workaround is needed.

CI additionally defines these Rust checks; the report does not establish that all of them passed:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --release --locked
cargo build -p ecp5_gui --release --locked
cargo clippy -p ecp5_gui --all-targets --locked -- -D warnings
```

## Other ways to get the tools

The package versions here are the ones cited by the install research on 2026-11-17.

- **OSS CAD Suite's database:** the suite carries `share/trellis/database`, as described by its
  [build scripts][oss-build]. In the 2026-10-22 archive it is expected to be the pinned database,
  but that copy has not been tested here; the loader's hash check decides. Point `ECP5_DB` at it
  instead of fetching another copy if it passes. The dated suite is selected by CI; the research
  initially inferred nightly source revisions from timestamps; the container test measured the
  Yosys/nextpnr versions above but did not test that database copy. The [suite's Trellis build][oss-build]
  uses `BUILD_SHARED=OFF`: it supplies no shared libtrellis, and it does not supply the C++ headers
  needed for this project's reader.
- **Homebrew:** [Yosys 0.71][brew-yosys] is newer than the pin;
  [prjtrellis 1.4][brew-trellis] is older than the pinned source revision. Homebrew core has no
  nextpnr formula ([formula lookup][brew-nextpnr]). These packages do not replace the pinned suite
  and source build above. Homebrew's Trellis install supplies a shared library and database, but
  not the C++ headers ([formula][brew-trellis-source], [upstream install rules][trellis-cmake]).
- **Distribution packages:** Ubuntu 24.04 has [Yosys `0.33-5build2`][ubuntu-yosys],
  [nextpnr-ecp5 `0.6-3build5`][ubuntu-nextpnr] and [Trellis `1.4-2build4`][ubuntu].
  Debian 13 has Yosys `0.52-2`, nextpnr-ecp5 `0.7-1+b2` on amd64, `fpga-trellis` `1.4-2+b4`
  and its database `1.4-2`
  ([Debian archive][debian], [Trellis package][debian-trellis]). These lag the pins, as do
  Fedora's Trellis `1.2.1` ([Fedora][fedora]) and Arch's `prjtrellis` `1.4-11` ([Arch][arch]).
  None of these Trellis packages installs C++ headers; Fedora's `trellis-devel` supplies the
  Python module and utilities, not the headers ([Fedora][fedora], [Arch file list][arch-files],
  [upstream install rules][trellis-cmake]). Use the source checkout's `libtrellis/include`.

**Building the FPGA tools from source** is another option, separate from the runtime-only
`packaging/build_trellis.sh`. Install the additional dependencies in the upstream instructions
first: [Yosys][yosys-readme] needs a C++20 compiler, CMake 3.28 or later, Bison 3.8 or later, Flex
and Python 3.11 or later; [nextpnr][nextpnr-readme] needs a C++17 compiler, CMake 3.25 or later,
Python development files, Boost and Eigen3. A full [Trellis build][trellis-readme] also needs
Python development files and its submodules. The runtime-only helper disables Python; building
nextpnr's ECP5 database needs the full Trellis install with `pytrellis`.

The commands below follow the upstream build instructions, selecting this project's pins and a
user-owned prefix. They are also **untested on a clean machine**. Configure Trellis inside
`libtrellis`, as its README requires when used with nextpnr; configure nextpnr and Yosys outside
their source directories. These source builds do not change the database selected by `ECP5_DB`.

```sh
cd "$HOME/fpga"
git clone --recursive https://github.com/YosysHQ/prjtrellis
cd prjtrellis
git checkout a31ff7042e712c0e2ad217054116ce2c4995f5c7
git submodule update --init --recursive
cd libtrellis
cmake -DCMAKE_INSTALL_PREFIX="$HOME/fpga/trellis-prefix" .
make
make install

cd "$HOME/fpga"
git clone --recursive https://github.com/YosysHQ/nextpnr.git
cd nextpnr
git checkout 6b438a1c9ab83cfb872fea21d9545d7d0bb8573f
git submodule update --init --recursive
mkdir -p build
cd build
cmake .. -DARCH=ecp5 -DTRELLIS_INSTALL_PREFIX="$HOME/fpga/trellis-prefix" \
  -DCMAKE_INSTALL_PREFIX="$HOME/fpga/trellis-prefix"
make
make install

cd "$HOME/fpga"
git clone https://github.com/YosysHQ/yosys.git
cd yosys
git checkout 485b63564
git submodule update --init --recursive
cmake -B build . -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$HOME/fpga/trellis-prefix"
cmake --build build --config Release --parallel
cmake --install build --strip
export PATH="$HOME/fpga/trellis-prefix/bin:$PATH"
```

These are the [Trellis][trellis-readme], [nextpnr ECP5][nextpnr-readme] and
[Yosys 0.70][yosys-readme] build commands with local paths substituted. On macOS, see also
[Yosys's Homebrew dependency instructions][yosys-install]; the small CI prerequisite list above
is for using the binary FPGA suite, not for compiling all three tools.

[oss-release]: https://github.com/YosysHQ/oss-cad-suite-build/releases/tag/2026-10-22
[oss-readme]: https://github.com/YosysHQ/oss-cad-suite-build/blob/main/README.md
[oss-build]: https://github.com/YosysHQ/oss-cad-suite-build/tree/main/default
[brew-yosys]: https://formulae.brew.sh/formula/yosys
[brew-trellis]: https://formulae.brew.sh/formula/prjtrellis
[brew-trellis-source]: https://github.com/Homebrew/homebrew-core/blob/HEAD/Formula/p/prjtrellis.rb
[brew-nextpnr]: https://formulae.brew.sh/api/formula/nextpnr.json
[ubuntu]: https://launchpad.net/ubuntu/+source/prjtrellis
[ubuntu-yosys]: https://launchpad.net/ubuntu/+source/yosys
[ubuntu-nextpnr]: https://launchpad.net/ubuntu/+source/nextpnr
[debian]: https://api.ftp-master.debian.org/madison?package=yosys+nextpnr+nextpnr-ecp5+fpga-trellis+fpga-trellis-database+python3-pytrellis&f=json
[debian-trellis]: https://packages.debian.org/trixie/fpga-trellis
[fedora]: https://packages.fedoraproject.org/pkgs/trellis/
[arch]: https://archlinux.org/packages/extra/x86_64/prjtrellis/
[arch-files]: https://archlinux.org/packages/extra/x86_64/prjtrellis/files/
[trellis-readme]: https://github.com/YosysHQ/prjtrellis/blob/master/README.md
[trellis-cmake]: https://github.com/YosysHQ/prjtrellis/blob/master/libtrellis/CMakeLists.txt
[nextpnr-readme]: https://github.com/YosysHQ/nextpnr/blob/master/README.md
[yosys-readme]: https://github.com/YosysHQ/yosys/blob/v0.70/README.md
[yosys-install]: https://github.com/YosysHQ/yosys/blob/main/docs/source/getting_started/installation.rst

## Record of the 2026-10-22 update

The local Trellis and nextpnr sources, the Trellis database submodule and the installed Yosys were updated. Trellis and nextpnr were rebuilt for arm64 with the current libraries. The full chain HDL → Yosys → nextpnr → `.config` → ecppack → `.bit` → ecpunpack passed; the configuration memory and INIT matched.

### Installed versions

| Component | Before the update | Now |
|---|---|---|
| Trellis | 73bd411 | a31ff7042e712c0e2ad217054116ce2c4995f5c7, 2026-10-12; 1.4-86-ga31ff70 |
| Trellis database | 015e033 | 9754ff473bfaa1e6edfaf57ce13bd447c76a88f6, 2026-10-12 |
| nextpnr | 58e3b858 | 6b438a1c9ab83cfb872fea21d9545d7d0bb8573f, 2026-10-20; 0.11.1-77-g6b438a1c |
| Yosys | 0.66, 7f8fdfd8 | Homebrew 0.70; the program reports 0.70+post, 0c11ea098be065e585a90a06e7b7da49936cf231 |

Both main repositories are now on `main` and track `origin/main`; `origin/HEAD` was also fixed. The earlier local `master` branches are kept. The database is on the submodule commit pinned by the parent Trellis. There were no tracked user changes before the switch; the existing unrelated files and the old builds are kept.

The executables are in `/opt/homebrew/bin`, the new libtrellis/pytrellis in `/opt/homebrew/lib/trellis`, the installed database in `/opt/homebrew/share/trellis/database`. These programs are selected by the existing PATH. The old `/usr/local` files are kept; calls by their absolute paths still select the earlier installation.

The project's Python bridge, the inventory tool and the native integration check now prefer `/opt/homebrew/lib/trellis`. An explicit `ECP5_TRELLIS_LIB` keeps priority. The four helper programs `target/trellis_*_probe` were rebuilt with the new library; the old copies are kept in `target/toolchain-update/old-probes`.

Trellis and nextpnr are built with Boost 1.92, Python 3.14.7 and an external pybind11 3.1. nextpnr includes the 25k/45k/85k devices, Python and the GUI on Qt 6.11.1; IPO is disabled. The executables start without the earlier Boost loading error. Interactive use of the GUI was not checked separately.

Yosys was updated through Homebrew. The dependencies it needs: readline 8.3.6, tcl-tk 9.1.0, tomlplusplus 3.4.0. Automatic cleanup of old versions was disabled; the installed pybind11 is needed for a compatible build of the Python modules.

### What was added to Trellis

[The database change of October 12](https://github.com/YosysHQ/prjtrellis-db/commit/9754ff473bfaa1e6edfaf57ce13bd447c76a88f6) adds the route `W7_JREFCLKO_EXTREF → N1E14_JSERDESREFCLK0` in DCU7, with the bit F59B1. [Trellis updated the submodule](https://github.com/YosysHQ/prjtrellis/commit/a31ff7042e712c0e2ad217054116ce2c4995f5c7) and added the corresponding fuzzer for the LFE5UM5G-85F.

This is a missing link of the SERDES reference clock in the UM/UM5G variants. The target LFE5U-85F has no DCU. A repeated collection confirmed that all tile types used by this device, their encodings and their connections are unchanged. No new functional block appeared in its catalog.

Since our earlier version, nextpnr has separate ECP5 changes: breaking long carry chains through S0 and re-injecting CIN from the fabric; ALIGNWD for IDDRX2F; PLL placement that takes ECLK into account; handling of several DCSs on one net; the choice of DCC; the placement of the LUT permutation switches. These are changes of the tools, not evidence of new capabilities of the physical chip. Later studies of clocks and carries must rely on the new version.

### What was rechecked

- All 148 types and 14 231 tiles of the LFE5U-85F: the substantive part of `device_inventory.json` did not change, only two revisions in the metadata changed. The names/defaults of the catalog were rechecked against the new pytrellis.
- Main resources: 502 constructors, 96 282 ports matched the native interfaces; the full export still contains 96 474 port records. The decoder catalog of 14 417 fields is byte-identical to before.
- PLC/LUTRAM: all 10 455 positions and 3 136 500 port records, 2 565 bitwise groups, six synthesis circuits and 147 600 model observations. Of 18 files, only the report with the data provenance changed.
- EBR addressing: six Yosys circuits, 12 288 INIT words; 14 substantive files out of 15 matched, the report changed.
- FF/CCU2: 256 FF variants, 14 336 transition observations, 83 968 triples of CCU2 outputs. The model traces and the structural tables matched; the report and the synthesis table changed. A repeated collection of the new 18 files gave a byte-for-byte match.

The updated base metadata and the three named research reports were moved into the main catalog `docs/inventory/LFE5U-85F`. The behavior models of the emulator did not change.

The remaining studies of DSP, IOLOGIC, IO routing, DQS and EBR control were not rerun in this update. Their reports keep the earlier checksums and scope of checking. The overall unchanged state of the LFE5U database does not replace a repeated check of the synthesis or routing behavior of these families.

### Changes of the Yosys probes

In [Yosys 0.70](https://github.com/YosysHQ/yosys/releases/tag/v0.70) the synthesis flow changed. The local probes established:

| Probe | Yosys 0.66 | Installed Yosys 0.70 |
|---|---|---|
| Asynchronous preload without the special flag | Rejected by the legalizer | The same |
| Preload with `-asyncprld` | Non-constant INIT error in ABC9 | The same |
| Preload with `-asyncprld -noabc9` | One TRELLIS_FF | ABC9 still runs, the same INIT error |
| Implicit latch from `always @*` | LUT with feedback | An inferred latch error by default |
| Latch with `-latches info` | Not used | LUT with feedback |
| Explicit SystemVerilog `always_latch` | Not used | LUT with feedback |

The research probe collector now distinguishes these results and keeps the specific reason for a failure. The new table contains 16 cases: 12 successful and four failures for the reasons described. The functions of the feedback LUT and of the additional CE priority LUT were rechecked against the actual INIT and connections. These synthesis limits do not close the hardware questions of PRLD and the level-sensitive FF.

### Checks of the whole installation

18 Rust tests, 15 checks of the portable catalog and three native integration checks passed. For a separate counter example, synthesis, placement/routing, packing and unpacking were run with the updated tools. For the original `.config`, the resulting `.bit` and the unpacked `.config`, all CRAM and EBR INIT data matched; the fresh `.bit` was read by the project decoder. The bench used the constraints of the existing CABGA381 example; this does not establish the model of the physical board.

The board was not programmed. The main update processes are complete; this is not a hardware validation of the emulator.

The summary versions, the hashes of programs/libraries, the file comparisons, the new reports and the check results are saved in [toolchain_update_2026-10-22](inventory/toolchain_update_2026-10-22). The full temporary builds and probes are in `target/toolchain-update`, and the build sources are in the FPGA directories named above.

The previous Trellis libraries, the Yosys model files and the archive of the whole earlier research catalog are saved in `~/FPGA/toolchain-snapshots/2026-10-22-before`. The catalog archive is `project-inventory-LFE5U-85F.tar.gz`; its hash is recorded in `comparison.json`. The old Yosys 0.66 is also left in the Homebrew Cellar. These data make it possible to tell the earlier evidence from the results of the update.

### Later collection of clock/system, PIO and packages

After the update, four research collectors were added: `collect_clock_system.py`, `collect_pio_banks.py`, `collect_package_system.py`, `collect_system_metadata.py`. They do not change the runtime. All 43 output files were repeated with a byte-for-byte match in `target/research/clock_system/repeated-*`. The new reports and the source URLs/hashes are in the family directories of the same names; the overall summary is [RESEARCH_STATUS.md](RESEARCH_STATUS.md).

The native metadata probe is built separately:

```sh
clang++ -std=c++14 \
  -I ~/FPGA/prjtrellis/libtrellis/include \
  -I /opt/homebrew/include tools/trellis_metadata_probe.cpp \
  -L /opt/homebrew/lib/trellis -ltrellis \
  -Wl,-rpath,/opt/homebrew/lib/trellis -o target/trellis_metadata_probe
python3 tools/collect_system_metadata.py
```

The probe reads `target/toolchain-update/smoke/top.bit`, adds synthetic commands only in the process memory and checks the parser/text conversion. No programming bitstream is written to disk, and no hardware transport is opened. Four service values are lost in the `ChipConfig` round-trip; details are in [SYSTEM_STARTUP.md](SYSTEM_STARTUP.md).

Nine clock probes were run with Yosys 0.70 and nextpnr `6b438a1c` after placement, with `--no-route`. They check the written DCS/OSCG/PLL fields. The first attempt to route an unconstrained DCS gave a router1 assertion; the log is saved separately. These results are not declared a successful full DCS routing or a hardware reference.

### Runtime bridge for the new configuration image

The new `load` uses `tools/trellis_image_reader.cpp`, built separately with `python3 tools/build_image_reader.py`. It reads the same source bytes from stdin and dumps the CRAM/EBR/service values directly from the native Chip. The Trellis checkout and the Python bindings are unchanged. For `.config`, the values of the four service registers are not exported from the constructor defaults; the text directives are kept on the Rust side. For `.bit`, the possible origin of a value from the parser defaults is marked. This differs from the research metadata probe and from the old bridge of the `decode`/`run` commands.

A build report with the SHA-256 of the source, the binary and libtrellis lies next to `target/trellis_image_reader`. The catalogs of the new layer are fixed by `python3 tools/build_configuration_manifest.py`. The implemented scope and the commands are in [CONFIGURATION_STATE.md](CONFIGURATION_STATE.md). The regression checks of the plain/compressed `.bit`, EBR/DSP/INIT and the counter passed; the new path does not open a hardware transport.
