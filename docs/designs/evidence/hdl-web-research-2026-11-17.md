# Open-Source SystemVerilog and VHDL Frontends for Yosys (Target: Lattice ECP5, `synth_ecp5`) — Late 2026 State

## Summary

For Lattice ECP5 synthesis (`synth_ecp5`) in late 2026, open-source HDL frontend support in Yosys (`YosysHQ/yosys`) has consolidated around three main paths for SystemVerilog and two for VHDL, all converging into Yosys's internal RTLIL representation before running `synth_ecp5` [1, 3, 6]:

1. **Built-in `read_verilog -sv`**: Native to Yosys (ISC license), supporting a narrow synthesizable SystemVerilog subset (`logic`, `bit`, `always_comb`/`always_ff`/`always_latch`, `typedef`, `enum`, packed `struct`/`union`, basic `package`/`import`, and immediate assertions), while lacking interfaces, type casts, nested packages, unpacked structs, and arrays of packed structs/unions [1, 2].
2. **`yosys-slang` (`povik/yosys-slang`, rebranded/integrated upstream as `povik/sv-elab` in Yosys v0.67+)**: The dominant modern open-source SystemVerilog frontend for Yosys in 2026, powered by Michael Popoloski's `slang` IEEE 1800-2017/2023 parser/elaborator under the ISC license [3, 4, 5]. It is actively maintained, shipped pre-compiled in the YosysHQ OSS CAD Suite (`linux-x64`, `darwin-arm64`, etc.), supports packages/imports, interfaces/modports, packed and unpacked structs, and functions with `return`, and preserves exact `file:line.col-line.col` Yosys `src` attributes [3, 4, 8].
3. **`synlig` (`chipsalliance/synlig`, formerly `yosys-f4pga-plugins` / Surelog+UHDM)**: Licensed under Apache-2.0, `chipsalliance/synlig` remains a public, non-archived repository on GitHub in 2026 with standalone Linux release tarballs, but ecosystem momentum and OSS CAD Suite packaging have shifted decisively to `yosys-slang` / `sv-elab` [5, 8].
4. **VHDL via `ghdl-yosys-plugin` vs. `ghdl synth --out=verilog`**: Both rely on GHDL (`ghdl/ghdl`, GPL-3.0) and its internal `libghdl` synthesis engine (`ghdl synth`), which works across all GHDL backends (`mcode`, `llvm`, and `gcc`, as synthesis runs in the frontend/netlist generator prior to simulation codegen) [6, 7]. `ghdl-yosys-plugin` (`yosys -m ghdl`) translates GHDL's netlist directly into Yosys RTLIL in-memory and attaches native `\src "file.vhdl:line"` attributes to cells and wires [6, 7]. By contrast, `ghdl synth --out=verilog` emits Verilog-2005 text where VHDL locations are emitted only as inline comments (`/* file.vhdl:line:col */`) rather than Verilog `(* src = ... *)` attributes, meaning subsequent `read_verilog` in Yosys loses original VHDL `src` attribute mapping [7].
5. **Packaging (OSS CAD Suite vs. Homebrew)**: YosysHQ's OSS CAD Suite (including late-2026 builds such as `2026-10-22`) bundles `yosys`, `yosys-slang` (`slang` plugin), `ghdl`, and `ghdl-yosys-plugin` (`linux-x64` and `darwin-arm64`), while omitting `synlig` [8]. On macOS Homebrew, `yosys` is in `homebrew-core` (without `yosys-slang` or `ghdl-yosys-plugin`), and the `ghdl` cask (at v6.0.0 for Sonoma+/ARM64) was explicitly disabled by Homebrew on `2026-09-01` (`because: :fails_gatekeeper_check`) due to unsigned upstream binaries and Homebrew's removal of `--no-quarantine` [9, 10, 11].

---

## Findings

### 1. Yosys Built-in `read_verilog -sv`: Documented SystemVerilog Subset

According to the official Yosys documentation (*"Notes on Verilog support in Yosys"* and the `read_verilog` command reference on `yosyshq.readthedocs.io`), Yosys natively aims for full synthesizable Verilog-2005 compliance and exposes experimental/partial SystemVerilog parsing when invoked with `read_verilog -sv` [1, 2].

| Category | Supported in `read_verilog -sv` | Documented Limitations / Unsupported |
| :--- | :--- | :--- |
| **Data Types & Variables** | `logic` and `bit` data types; `enum` declarations (not strongly typed); `typedef` definitions (including inside packages); `rand` and `rand const` free variables [1]. | Type casts are **not** supported [1]. |
| **Structures & Unions** | Packed `struct` and packed `union` types [1]. | **Unpacked** structs/unions and **arrays of packed structs or unions** are **not** supported [1]. |
| **Procedural Blocks** | `always_comb`, `always_ff`, `always_latch` [1]. | Advanced procedural constructs and non-synthesizable dynamic constructs [1]. |
| **Packages & Hierarchy** | `package ... endpackage` declarations and `import` statements (imported packages remain visible to subsequent files read into the design); portless `checker` blocks acting as named blocks [1]. | **Nested packages** (`import` inside a `package` definition) are **not** supported; `interface` and `modport` constructs are not supported by the native AST frontend [1]. |
| **Verification / Formal** | Concurrent/immediate-style `assert`, `assume`, `restrict`, `cover`; `$past`, `$stable`, `$rose`, `$fell`, `$anyconst`, `$anyseq`, `$allconst`, `$allseq` (with `-sv` / `-formal`) [1, 2]. | Full IEEE 1800 SVA sequences/properties require commercial Verific or external frontends [1]. |

*Note on ECP5 target*: Once parsed by `read_verilog -sv` (or any frontend below) into Yosys RTLIL, running `synth_ecp5` performs standard device mapping (`TRELLIS_FF`, `TRELLIS_COMB`, `DP16KD`, `MULT18X18D`) identically regardless of the frontend used [2, 8].

---

### 2. `yosys-slang` (`povik/yosys-slang` / `povik/sv-elab`)

* **Maturity & Late-2026 Status**: Created by Martin Povišer (`povik`) on top of Michael Popoloski's `slang` compiler library (`MikePopoloski/slang`), `yosys-slang` is actively developed in 2026 and has evolved into `povik/sv-elab`, which was also integrated directly upstream into Yosys core starting around Yosys v0.67+ while maintaining the standalone plugin repository [3, 4]. Exact commit timestamps for late 2026 on `povik/yosys-slang` / `povik/sv-elab` track ongoing `slang` and Yosys master synchronization (exact day-of-commit in November 2026 is unverified due to dynamic GitHub commit history, though it is included in late-October 2026 OSS CAD Suite builds) [3, 8].
* **Installation Channels**:
  * **Build from source against Yosys**: Cloned recursively (`git clone --recursive https://github.com/povik/yosys-slang`) and built with CMake/Make against `yosys-config` headers to produce `slang.so`, loaded via `yosys -m slang` (`read_slang`) [3].
  * **OSS CAD Suite**: Yes—shipped pre-built inside YosysHQ's `oss-cad-suite-build` releases [3, 8].
  * **Homebrew**: **Not packaged** in `Homebrew/homebrew-core` or `Homebrew/homebrew-cask` as a standalone formula; the `yosys` formula in `homebrew-core` builds standard Yosys without the `yosys-slang` plugin [3, 11].
* **Supported Constructs**:
  * **Packages & `import`**: Fully supported, including wildcard (`import pkg::*;`) and explicit package imports as well as scope resolution (`pkg::symbol`), validated against complex IP cores such as OpenTitan and Ibex [3, 4].
  * **Interfaces & Modports**: Supported (`interface`, `modport`, interface ports, and hierarchical member access), flattened into word-level RTLIL structures during elaboration [3].
  * **Structs**: Both **packed** and **unpacked** structs (and nested arrays/structs) are supported via word-level flattening into Yosys wires/memories [3].
  * **Functions with `return`**: Supported for both constant evaluation and synthesizable combinational/procedural logic generation (including automatic functions and early `return` statements lowered into multiplexed procedural trees) [3, 4].
* **License**: **ISC License** (matching Yosys; underlying `slang` and `{fmt}` libraries are MIT-licensed) [3, 4].

---

### 3. `synlig` (`chipsalliance/synlig`)

* **Maintenance Status in 2026**: `chipsalliance/synlig` is **not archived** on GitHub as of late 2026; the repository remains public under the CHIPS Alliance organization [5]. However, development velocity has slowed substantially compared to 2023–2024 as the Yosys ecosystem and YosysHQ shifted focus toward `yosys-slang` / `sv-elab` (`slang`'s monolithic AST/elaborator avoids the heavyweight Surelog $\rightarrow$ UHDM $\rightarrow$ Yosys AST serialization pipeline used by Synlig) [3, 5, 8].
* **Installation**:
  * **Pre-built Linux binaries**: Distributed as `.tar.gz` archives on the GitHub Releases page (`https://github.com/chipsalliance/synlig/releases`), installed by extracting the tarball and adding the `synlig` binary/plugin directory to `PATH` [5].
  * **From source**: Built via git submodules (`Surelog`, `UHDM`, `yosys`) using CMake/Make; can be built either as a standalone `synlig` binary (wrapping Yosys) or as the `systemverilog.so` (`read_systemverilog`) plugin for Yosys [5].
  * **Homebrew / OSS CAD Suite**: Neither in Homebrew nor bundled in late-2026 YosysHQ OSS CAD Suite builds [8, 11].
* **License**: **Apache License 2.0** [5].

---

### 4. VHDL: `ghdl-yosys-plugin` vs. `ghdl synth --out=verilog`

#### How Each Workflow Operates and Stability
* **`ghdl-yosys-plugin` (`ghdl/ghdl-yosys-plugin`)**: Loaded into Yosys via `yosys -m ghdl` (command `ghdl [options] <files> -e <top>`). It links dynamically against `libghdl`, invoking GHDL's internal synthesizer (`ghdl synth`) in-process and walking the resulting `netlists` data structures in C++ (`src/ghdl.cc`) to emit Yosys RTLIL cells (`$add`, `$dff`, `$mem`, `$procmux`, etc.) directly into Yosys memory [6, 7]. It is mature and stable for VHDL-87/93/2008 synthesis, including generics, records, arrays, and IEEE `numeric_std` [6, 7].
* **`ghdl synth --out=verilog` piped into Yosys**: Runs standalone GHDL (`ghdl synth --std=08 --out=verilog files.vhd -e top > top.v`) and feeds the generated structural/behavioral Verilog-2005 file into `yosys -p "read_verilog top.v; synth_ecp5 ..."` [7]. This decouples GHDL and Yosys versions completely (avoiding C++ ABI mismatches between `ghdl.so` and `yosys`), making it a popular fallback when `ghdl-yosys-plugin` cannot be compiled or dynamically loaded on a host OS [6, 7].

#### Which GHDL Backends Support `synth`
GHDL has three simulation code-generation backends: **`mcode`** (in-memory x86/aarch64 codegen), **`llvm`** (LLVM IR codegen), and **`gcc`** (GNAT/GCC backend, deprecated in recent GHDL releases) [7]. Because `ghdl synth` and `libghdl` operate on GHDL's IIR (Internal Intermediate Representation) and internal `netlists` synthesis engine *before* any simulation machine-code backend is invoked, **all three backends (`mcode`, `llvm`, and `gcc`) support `ghdl synth` and `ghdl-yosys-plugin`** [6, 7].

#### Homebrew `ghdl` Cask (`6.0.0`) and macOS Code-Signing / Gatekeeper Issues
* In `Homebrew/homebrew-cask` (`Casks/g/ghdl.rb`), GHDL is packaged at version **`6.0.0`** for macOS Sonoma and newer (`on_sonoma :or_newer`, `aarch64` / `x86_64` tarballs from `ghdl/ghdl` releases, with `5.1.1` on older macOS) [9].
* **Gatekeeper / Quarantine Failure**: Effective **September 1, 2026**, Homebrew explicitly disabled the `ghdl` cask with:
  ```ruby
  disable! date: "2026-09-01", because: :fails_gatekeeper_check
  ```
  Following Homebrew 5.0+/6.0 policy changes that removed the `--no-quarantine` flag and enforced mandatory macOS Gatekeeper validation for all casks, unsigned/unnotarized upstream `ghdl` release tarballs trigger macOS `com.apple.quarantine` blocks (`zsh: killed` or Gatekeeper "developer cannot be verified" dialogs) on macOS Sequoia/Tahoe (ARM64 requires valid ad-hoc or Developer ID signatures on Mach-O binaries and `.dylib` files) [9, 10].
* Users installing GHDL on macOS outside OSS CAD Suite must manually strip quarantine attributes (`xattr -dr com.apple.quarantine`) and ad-hoc sign (`codesign --force --deep -s -`) the binaries, or use the pre-signed/bundled `oss-cad-suite` archive [8, 9, 10].

#### License
Both **GHDL** (`ghdl/ghdl`) and **`ghdl-yosys-plugin`** (`ghdl/ghdl-yosys-plugin`) are licensed under the **GNU General Public License v3.0 (GPL-3.0-or-later)** [6, 7]. When `ghdl-yosys-plugin` is linked into Yosys, the combined binary/distribution is governed by GPLv3 [6].

---

### 5. YosysHQ OSS CAD Suite (Release around `2026-10-22`) Packaging Matrix

YosysHQ's `oss-cad-suite-build` repository publishes rolling multi-platform nightly/dated builds (including late-October 2026 builds such as `2026-10-22`) [8].

| Component | Shipped in OSS CAD Suite (`~2026-10-22`)? | Platform & Packaging Notes |
| :--- | :--- | :--- |
| **`yosys-slang`** (`slang` plugin) | **Yes** [8] | Bundled as a pre-compiled Yosys plugin (`yosys -m slang` / `read_slang`) across supported platforms (`linux-x64`, `linux-arm64`, `darwin-arm64`, `windows-x64`) [3, 8]. |
| **`ghdl`** | **Yes** [8] | Bundled with VHDL-87/93/2008 simulation and `ghdl synth` support on supported platforms (`linux-x64`, `darwin-arm64`, etc.) [8]. |
| **`ghdl-yosys-plugin`** | **Yes** [8] | Bundled as the `ghdl` Yosys plugin (`yosys -m ghdl`) on platforms where GHDL shared libraries are supported (specifically `linux-x64` and `darwin-arm64`) [8]. |
| **`synlig`** | **No** [8] | Not shipped in `oss-cad-suite-build`; YosysHQ standardizes on `yosys-slang` for open-source SystemVerilog in OSS CAD Suite [5, 8]. |

---

### 6. Source Location (`src` Attribute) Preservation Across Frontends

Yosys tracks HDL source locations on modules, wires, processes, and cells via the RTLIL `\src` attribute (`(* src = "file:line..." *)`), which downstream tools like `nextpnr-ecp5` and timing/error reporters use for diagnostic tracebacks [2, 3, 7]:

1. **`yosys-slang` (`read_slang`)**: **Preserves original `file:line` (and column range)**. `yosys-slang` translates `slang::SourceLocation` directly into Yosys `\src` attributes formatted as `"filename.sv:line.col"` or `"filename.sv:start_line.start_col-end_line.end_col"` (e.g., `attribute \src "counter.sv:6.5-8.8"`) on modules, wires, and cells [3].
2. **`ghdl-yosys-plugin` (`yosys -m ghdl`)**: **Preserves original `file:line`**. As `src/ghdl.cc` converts GHDL netlist instances into RTLIL cells and wires, it queries `libghdl`'s location table and attaches `attribute \src "filename.vhdl:line"` (or `filename.vhdl:line:col`) directly onto the generated RTLIL objects [6, 7].
3. **`ghdl synth --out=verilog` piped into `read_verilog`**: **Does NOT populate Yosys `src` attributes with the original VHDL `file:line`**. When GHDL dumps Verilog via `--out=verilog`, it emits original VHDL source coordinates solely as **plain Verilog comments** (e.g., `/* /path/to/design.vhdl:20:5 */`) above declarations and instances, rather than `(* src = "/path/to/design.vhdl:20:5" *)` attributes [7]. When Yosys `read_verilog` parses that intermediate Verilog output, it strips comments and records `\src` attributes pointing to the **intermediate `.v` file/pipe line numbers** (`<<stdin>>:line` or `top.v:line`), losing programmatic `src` attribute linkage back to `.vhdl` unless a custom post-processor converts GHDL's `/* file:line:col */` comments into `(* src = "..." *)` attributes before `read_verilog` [2, 7].

---

## Risks and Counterarguments

* **ABI Coupling of Yosys Plugins (`yosys-slang` and `ghdl-yosys-plugin`)**: Because Yosys C++ plugins link against internal `libyosys` symbols without a stable C ABI, a plugin compiled against one Yosys commit will fail to load (`undefined symbol`) if paired with a Homebrew or system `yosys` binary from a different release [3, 6]. Using the monolithic OSS CAD Suite distribution avoids this mismatch [8].
* **GPLv3 Contamination in Embedded/Commercial Tool Distributions**: While Yosys (`read_verilog`), `yosys-slang`, and `nextpnr` are ISC/MIT licensed and Synlig is Apache-2.0, `ghdl` and `ghdl-yosys-plugin` are strictly GPLv3 [3, 5, 6, 7]. Embedding `ghdl-yosys-plugin` in a proprietary binary distribution triggers GPLv3 obligations, whereas invoking `ghdl synth --out=verilog` as a separate subprocess isolates process boundaries (at the cost of losing native Yosys `\src` attributes unless post-processed) [6, 7].
* **Elaboration Memory & Edge-Case Differences (`yosys-slang` vs. Verific)**: While `slang` parses 100% of IEEE 1800-2017, `yosys-slang`'s synthesis lowering (`sv-elab`) flattens complex interfaces and unpacked structs into word-level operations, which can occasionally hit edge cases on deeply nested dynamic interface arrays or complex non-constant functions compared to commercial Verific (`Tabby CAD`) [1, 3].

---

## Evidence Gaps

* **Exact Late-October/November 2026 Commit Hashes**: While the architectural state, OSS CAD Suite inclusion (`~2026-10-22`), and Homebrew `ghdl` cask disable date (`2026-09-01`) are verified from repository records, the exact git commit SHA of `povik/yosys-slang` (`povik/sv-elab`) and `chipsalliance/synlig` on `2026-11-17` could not be inspected line-by-line via live git clone and is marked unverified [3, 5, 8, 9].
* **GHDL v6.0.0+** `--out=verilog` **Attribute Flag**: As of documented GHDL synthesis netlist exporters (`Netlists.Dump_Verilog`), VHDL locations are emitted as `/* file:line:col */` comments [7]; whether an undocumented or late-2026 experimental flag was added in GHDL `master` to emit `(* src = ... *)` Verilog attributes directly is unverified.

---

## References

1. [YosysHQ — Notes on Verilog Support in Yosys (Official Documentation)](https://yosyshq.readthedocs.io/projects/yosys/en/latest/using_yosys/synthesis/verilog.html)
2. [YosysHQ — `read_verilog` Command Reference (Official Documentation)](https://yosyshq.readthedocs.io/projects/yosys/en/latest/cmd/read_verilog.html)
3. [Martin Povišer (`povik`) — `yosys-slang` / `sv-elab` SystemVerilog Frontend for Yosys (GitHub Repository & Feature Support)](https://github.com/povik/yosys-slang)
4. [Michael Popoloski — `slang`: SystemVerilog Compiler and Language Services (GitHub Repository)](https://github.com/MikePopoloski/slang)
5. [CHIPS Alliance — `synlig`: SystemVerilog Synthesis Tool (GitHub Repository & Releases)](https://github.com/chipsalliance/synlig)
6. [GHDL Project — `ghdl-yosys-plugin`: VHDL Synthesis Plugin for Yosys (GitHub Repository)](https://github.com/ghdl/ghdl-yosys-plugin)
7. [GHDL Documentation — Synthesis with GHDL (`ghdl synth`, Backends, and `--out=verilog`)](https://ghdl.github.io/ghdl/using/Synthesis.html)
8. [YosysHQ — `oss-cad-suite-build`: Pre-built Open-Source Digital Design Toolchain Manifests & Releases (GitHub Repository)](https://github.com/YosysHQ/oss-cad-suite-build)
9. [Homebrew — `homebrew-cask` `ghdl.rb` Cask Definition (v6.0.0 and `2026-09-01` `:fails_gatekeeper_check` Disablement)](https://github.com/Homebrew/homebrew-cask/blob/HEAD/Casks/g/ghdl.rb)
10. [Homebrew — Deprecating and Removing `--no-quarantine` / Gatekeeper Cask Policy](https://brew.sh/2025/02/03/homebrew-4.4.0/)
11. [Homebrew — `homebrew-core` `yosys.rb` Formula Definition](https://github.com/Homebrew/homebrew-core/blob/HEAD/Formula/y/yosys.rb)
