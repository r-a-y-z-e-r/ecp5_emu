# HDL languages in the project flow

Status: slices 1 and 2 implemented for v0.1.0. Ordered Verilog/SystemVerilog,
an explicit yosys-slang pin, and VHDL through the GHDL Yosys plugin are supported.
VHDL source probing is unavailable: the plugin emits no original source locations.
[Two-host suite evidence](evidence/hdl-suite-check-2026-11-18.md) records the commands,
limitations and runtime packaging measurements. Historical research below is
superseded by those measurements and the implemented decisions in §§5–8.

## 1. Decision and evidence

Auto and builtin always use the default Yosys built-in reader, including
`read_verilog -sv` for SystemVerilog. There is no slang capability probe or
automatic selection. Only `sv_frontend = "slang"` or VHDL roots select the suite
for this project. Never retry a failed compile with another frontend.

The evidence has different strengths:

- [Mac measurements](evidence/hdl-measured-2026-11-17.md), lines 1–29: Yosys
  0.70+post, individual construct synthesis, source attributes, broken Mac GHDL,
  absent plugins, and the rserv inventory.
- [Supplied web research](evidence/hdl-web-research-2026-11-17.md): upstream
  descriptions, licences and packaging claims, with primary-source links. This
  note cites that recorded research; it does not independently audit the releases.
- [Blind panel](evidence/hdl-panel-2026-11-17.md): both recommend slang, the GHDL
  plugin and OSS CAD Suite. The selected fallback policy is explicit selection
  and recording, no concealed slang compile failures, and pinning support.

For Yosys 0.70 the measurements win over two web claims: the tested interface
with modport passed `synth_ecp5`; all three tested `import` forms failed. Neither
result establishes all interfaces or all other SV constructs. The web claim
“yosys-slang integrated upstream as sv-elab in Yosys 0.67+” is **unverified**;
the panel reports no `read_slang` in this installed Yosys. An explicit pin selects the measured suite plugin; version numbers do not select frontends.

## 2. Frontends and present availability

These frontends run as external processes; no Rust dependencies were added.
Keep each installed distribution's notices; the suite has no single licence.
The earlier [web research](evidence/hdl-web-research-2026-11-17.md) describes
origins/licences; packaging must audit the actual shipped notices.

| Frontend | Default tools | OSS CAD Suite 2026-10-22 on Mac and rserv |
| --- | --- | --- |
| `read_verilog -sv` | Built-in measured subset; default for auto/builtin. | Packaged `ECP5_YOSYS` may point here too. |
| yosys-slang / `read_slang` | Never probed automatically. | `-m slang`, explicit project pin; synthesis, default routing/packing, original SV `src` and emulation measured. |
| GHDL Yosys plugin | Standalone `ghdl` on PATH is irrelevant. | `-m ghdl`, VHDL-2008/work; synthesis, default routing/packing and emulation measured. **No VHDL source locations**. |
| Synlig or GHDL text export | Not selected. | Outside v0.1.0. |

The GHDL plugin was chosen as the direct synthesis interface. The earlier claim
that it produces original VHDL `src` attributes was disproved on both hosts.
No generated-Verilog source substitution or guessed VHDL location is offered.

The Mac's GHDL 6.0.0 cask fails with an invalid `libgcc_s.1.1.dylib` signature
(`hdl-measured-2026-11-17.md:25–27`). The research records Homebrew disabling the
cask on 2026-09-01 for `fails_gatekeeper_check`. A `ghdl` executable on PATH is
therefore not proof of VHDL support. Do not automatically strip quarantine,
re-sign binaries or change machine security settings.

## 3. The measured built-in SystemVerilog subset

All rows below reproduce the scope of the Mac measurements, not a general SV
conformance promise. Each small top passed or failed
`read_verilog -sv FILE; synth_ecp5 -top top -json FILE.json` on Yosys 0.70+post.

| Construct | Measured result |
| --- | --- |
| `logic`, `always_ff`, `always_comb` | Pass. |
| `always_latch` | Pass. |
| `typedef enum logic [1:0]` and case | Pass. |
| Packed struct and field access | Pass. |
| Packed union containing a packed struct | Pass. |
| Instantiated interface/modport passed to `bus_if.src` submodule port | Pass. |
| `unique case` | Pass. |
| `.*` port connections | Pass. |
| Immediate assertion inside `always_ff` | Accepted; assertion synthesis meaning was not checked. |
| Package localparam referenced as `pkg::W` | Pass. |
| `import pkg::*;` inside module | Fail: unexpected `TOK_IMPORT`. |
| File-scope `import pkg::*;` | Fail: unexpected `;`, expecting `(` or `[`. |
| Module-header import | Fail: unexpected `TOK_IMPORT`. |
| Function `return x + 1;`, in module or package | Fail: unexpected `TOK_ID`. |
| Typedef return type, assigning the function name | Pass. |

“Pass” means read plus synthesis, not simulated behaviour. The same measurements
found `src` strings such as `enum.sv:1.1-6.10` in synthesis JSON. Routing and
bidirectional window probing remain release checks. Unmeasured language features
are unsupported by the release promise until tested; do not infer support from
web claims about `bit`, casts, unpacked aggregates or verification constructs.

## 4. Project format and source order

Add one ordered opt-in `sources.hdl` list. Keep `sources.verilog` and its sorted,
root-level `*.v` default unchanged when `hdl` is absent. Do not auto-discover
`.sv`/`.vhd` beside an old project and silently change what it builds. To use
new languages, put all compilation roots in `hdl`, even a single `top.sv`.
An explicitly supplied `verilog` list and `hdl` together are an error; do not
merge two orders. Legacy `verilog` remains the old Verilog-mode input path;
language-by-extension applies to the new list.

```toml
[project]
name = "counter"
top = "top"
[sources]
hdl = ["rtl/types.sv", "rtl/counter.sv", "top.sv"]
constraints = "top.lpf"
files = ["include/settings.svh"]
include_dirs = ["include"]
[synthesis]
sv_frontend = "auto" # auto | builtin | slang; default auto
```

Implemented new-list classification: `.v` is Verilog, `.sv` is SystemVerilog,
`.vhd`/`.vhdl` are VHDL. Match suffixes without case sensitivity, retain the real
path's case. Reject other root suffixes and duplicate paths. Require at least one
HDL root across the selected mode, not at least one Verilog file. All source,
include and auxiliary paths retain the relative/no-`..` rule.
The manifest denies unknown keys. Both source-list modes share relative-path and
file-existence validation; mixed VHDL and Verilog/SV roots fail before tools run.

`sources.files` keeps its current default of root-level hex/mem/vh/svh/init,
minus memory images (`src/flow/project.rs:219–224`). Headers are copied/hashed
inputs, not separately compiled roots. New `include_dirs` defaults to `["."]`;
an explicit list replaces it. Resolve include directories under the project,
copy listed nested headers preserving layout, and pass include search paths to
the selected frontend. Listing a directory does not import/hash its contents:
included files must be in `sources.files`. Diagnose missing/unlisted user
includes rather than accept an untracked build dependency. `include_dirs` is
valid only with `hdl`; legacy mode keeps its current invocation. No new defines,
free-form frontend options or dependency scanner in these slices.

Preserve `hdl` order; packages precede users, with VHDL package bodies and
entities/architectures in analysis order. No filename-based dependency sorting.
Built-in Verilog/SV reads form contiguous same-language runs in that order;
slang receives the complete Verilog/SV list in one invocation. Headers must be
included explicitly; file-to-file macro leakage is not a portable contract.

VHDL slice 2 supports library `work`, standard `--std=08`, one elaborated root,
and ordinary entity/port identifiers. `project.top` is a module for Verilog/SV,
an entity for VHDL; default remains `top`. Validate VHDL basic-identifier rules,
canonicalize its top to ASCII lower case for GHDL/Yosys and report that name.
Do not lower-case filesystem paths or Verilog identifiers. Use the actual emitted
port names in LPF; templates use lower-case names matching the existing board
ports. Multiple architectures require an unambiguous selected elaboration, not
a guess from source filenames; initially require one architecture per entity.

Mixed Verilog/SV projects are supported, including module instantiation across
those two languages. In these two slices, VHDL projects are VHDL-only: reject an
`hdl` list mixing VHDL and Verilog/SV before synthesis with “Cross-language VHDL
hierarchy is not supported yet”. Direct VHDL component/Verilog module binding,
foreign library roots and cross-language records/generics need a later design
and measured elaboration tests. This boundary avoids claiming that concatenating
frontend commands implements cross-language elaboration.

## 5. Selecting tools and exact synthesis scripts (implemented)

`Tools::from_env` keeps `ECP5_YOSYS`, `ECP5_NEXTPNR`, `ECP5_ECPPACK`,
`ECP5_ECPBRAM` and PATH defaults. **An ECP5_YOSYS inside a suite is allowed**:
[packaged launchers](bundled-toolchain.md#wiring) set that for ordinary projects.
The flow never inspects a Yosys executable; `bin/yosys` may be a wrapper script.
It never promotes suite bin on the caller's PATH.

Only a slang pin or any VHDL root resolves `ECP5_OSS_CAD_SUITE`. An explicit
invalid root fails, names the setting and has no fallback. Packaged releases set
it to their bundled toolchain root. Otherwise resolve the first executable
`yosys` on PATH, following symlinks, as when the user runs
`source <suite>/environment`. Accept the directory above its `bin/` only if it
contains `bin/yosys`, `libexec/yosys` and the project's needed plugin at
`share/yosys/plugins/slang.so` or `share/yosys/plugins/ghdl.so`. An ordinary
Homebrew Yosys does not qualify. The explicit environment override still requires
`bin/yosys`, with the needed plugin checked before any tool runs.

A missing suite emits one error naming the slang pin or VHDL requirement,
stating that `ECP5_OSS_CAD_SUITE` is not set, and showing the PATH Yosys path (or
`none`) and why it is not a suite with the needed plugin. It tells the user to
set `ECP5_OSS_CAD_SUITE` to the suite root or run `source <suite>/environment`,
before even a version probe or memory placeholder runs. The report records the
canonical root and source: `ECP5_OSS_CAD_SUITE` or `yosys on PATH (<root>)`.
An existing GHDL_PREFIX is respected; otherwise only the synthesis child
receives `GHDL_PREFIX=$SUITE/lib/ghdl`. No global environment is changed.

| Selection | Synthesis | Downstream |
| --- | --- | --- |
| Legacy Verilog, or HDL auto/builtin | Default Yosys, built-in reader; no slang probing. | Default tools. |
| Explicit `sv_frontend = "slang"` | `$SUITE/bin/yosys -m slang`; never fall back. | Default nextpnr, ecppack, ecpbram. |
| VHDL roots | `$SUITE/bin/yosys -m ghdl`; never use standalone GHDL text export. | Default nextpnr, ecppack, ecpbram. |

Exact script templates (argv execution, no shell), with top `top`:

```sh
# Legacy invocation is preserved.
yosys -l yosys.log -p 'read_verilog top.v; synth_ecp5 -top top -json top.json'
# Built-in mixed HDL: contiguous runs in declared order.
yosys -l yosys.log -p 'read_verilog -Iinclude leaf.v; read_verilog -sv -Iinclude types.sv top.sv; synth_ecp5 -top top -json top.json'
# Slang: every include directory and all Verilog/SV roots in ONE call.
"$SUITE/bin/yosys" -m slang -l yosys.log -p 'read_slang --top top -Iinclude leaf.v types.sv top.sv; synth_ecp5 -top top -json top.json'
# VHDL: analysis order, VHDL-2008, library work, one elaborated root.
"$SUITE/bin/yosys" -m ghdl -l yosys.log -p 'ghdl --std=08 types.vhd counter.vhd top.vhd -e top; synth_ecp5 -top top -json top.json'
```

A failed compile is never retried. The built-in reader's `unexpected TOK_IMPORT`
and `unexpected TOK_ID` diagnostics receive this hint, only for those known
unsupported forms:
`set [synthesis] sv_frontend = "slang" in ecp5.toml (needs OSS CAD Suite)`.
Other syntax errors keep their original diagnostic without this hint.

`report.json` Frontend records requested/resolved reader, reason, actual Yosys
path/version, suite root/source, plugin name and identity (relative `.so` path
plus SHA-256), VHDL standard and effective GHDL_PREFIX when applicable. The CLI
Frontend line includes a short digest and suite provenance. All these fields,
script/options, ordered source languages and input hashes enter the synthesis
cache key. Changing the reader, root or plugin bytes invalidates synthesis;
unchanged output hashes still permit downstream cache hits. The isolated HDL
workspace contains only declared roots/files and preserves relative paths.

## 6. Source probing, diagnostics and limits (implemented)

All languages synthesize to real ECP5 configuration; the emulator does not
interpret HDL. Device support remains LFE5U-85F/CABGA381. Mixed Verilog/SV works;
cross-language VHDL hierarchy, foreign libraries, extended VHDL identifiers and
full language conformance are outside v0.1.0. Use one architecture per entity.
VHDL basic top identifiers are validated and normalized to ASCII lowercase;
filesystem paths and Verilog identifiers retain their spelling.

| Frontend | Source probing |
| --- | --- |
| Built-in Verilog/SV | Original source attributes survive routing; slice-1 tests exercise both source-probe directions. |
| slang | Routed JSON retains original `.sv` locations, measured on both hosts. |
| GHDL | Build, routing, bitstream generation, emulation and Waves work; no original VHDL locations are emitted. |

For VHDL projects the build report and CLI include one note:
**Source probing is not available for VHDL yet: the GHDL plugin writes no source locations**.
That exact text also explains disabled Show in source for map nets and critical
path hops without a source location. No VHDL source case/vector lookup is promised
without attributes to map. See the [counterexample and measurements](evidence/hdl-suite-check-2026-11-18.md).

The diagnostic parser accepts `file.vhd:LINE:COL:error: ...`, the same form
without severity, and slang's `file.sv:LINE: ERROR: ...`. Original project paths,
absolute project/build paths and staged HDL paths resolve only against declared
inputs. Source excerpts, carets and indented continuation lines attach to the
preceding location diagnostic. Tool exit status remains authoritative.

## 7. Suite evidence and runtime packaging

OSS CAD Suite **2026-10-22** is measured on darwin-arm64 and linux-x64. The
[2026-11-18 evidence](evidence/hdl-suite-check-2026-11-18.md) records exact sources,
commands, versions, expected parser failures, default-nextpnr JSON compatibility,
packing, original SV attributes, absent VHDL attributes and 320-cycle LED counts.
The Mac requires the per-command GHDL_PREFIX. Both hosts use the suite's own
Yosys/plugin ABI together. This implementation does not download tools, modify
quarantine/signatures, or replace default reference-generation tools.

The appended runtime section lists suite files opened during VHDL synthesis,
including Yosys/ABC helpers and data beyond lib/ghdl. It is a measured fixture
closure for packaging, not a universal closure for every language feature.

## 8. v0.1.0 implementation and verification

Both slices ship in v0.1.0: the ordered HDL model and built-in SV reader from
slice 1, plus explicit slang and GHDL-plugin VHDL from slice 2. Examples:
[blinky_sv](../../examples/ulx3s/blinky_sv/README.md),
[sv_package](../../examples/ulx3s/sv_package/README.md), and
[blinky_vhdl](../../examples/ulx3s/blinky_vhdl/README.md).

Unit/stub coverage checks env/PATH resolution, invalid/missing roots before
any tool, script construction and ordering, child-only prefix preservation,
frontend/root/plugin cache invalidation, report fields, GHDL/slang diagnostics,
the built-in pin hint, and no retry. Ignored `real_suite_*` flow tests require
the suite and default downstream tools; set `ECP5_OSS_CAD_SUITE` to its root or
run `source <suite>/environment`. When explicitly run without either, they fail
with the setup error, never silently skip. They build/emulate both counters and
reject a slang error once at the original source location.

Existing legacy flow/build.sh comparisons keep the default tools and their
independent command construction. Never regenerate reference configurations to
make different toolchains byte-identical. Source locations are a slang gate;
VHDL ships with the stated source-probing limit. GUI tests run on the Mac.
Tool bundling, auto-download, CI provisioning and a New Project VHDL template
remain separate work; this slice adds no dependencies and no global tool changes.
