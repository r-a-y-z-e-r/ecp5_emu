# Automatic assembly of execution from the configuration

The continuations of this milestone are [LUTRAM/RAMW execution](LUTRAM_EXECUTION.md), then [GSR/GOE/GWDIS and explicit wake phases](SYSTEM_EXECUTION.md). Shared LUT/RAM cells, active RAMW as all-plc roots and the global controls were added. The 65 checks described below, the rejection for an active user GSR and the absence of DPRAM belong to the first milestone of automatic assembly.

Milestone of October 22, 2026. The [event kernel](EVENT_KERNEL.md) now gets its plan from CRAM, without a manual description of the LUTs, FFs and each connection. Added are [compile](../src/compile/mod.rs), the reverse index [index](../src/compile/index.rs) and the `emulate` command. Source HDL/netlist is not used for execution: Yosys/nextpnr are needed in the check to produce an independent `.config`/`.bit` example.

This is the first executable part of the full model. All tiles, resources and LUT/FF/EBR storages are kept. The behavior of EBR/DSP/IOLOGIC, of the dynamic clock blocks and of the full configuration procedure is not connected yet. A software end-to-end check of the fabric, carries, local reset and GPIO has been achieved in an explicitly selected user mode.

## What is built automatically

| Part | Implemented binding |
|---|---|
| PLC scalar fields | Enum templates from `bits.db`, including inversions and nested variants; defaults do not replace observations |
| LUT4 | Live INIT table, native A/B/C/D/F, selection of the constant 1 by the input muxes |
| Extension muxes | F5 and FX through M0/M1 and the native/coarse ports of the slice |
| CCU2 | Both halves through their own LUTs, carry inputs/outputs, INJECT1, fabric chains/adapters |
| FF | DI/M through SD, shared CE, selected CLK/LSR, inversions, REGSET, sync/async local reset |
| Routing | Directed fixed/mux by CRAM, relative endpoints, the 85K density and the device boundaries |
| Primary clock | Verified raw/segment relations, real CMUX/SPINE/TAP paths and a static DCC bypass |
| PIO | Receiver, separate pad, output, tristate, digital pull/open-drain; an unambiguous supported configuration |

`TileType` keeps all enum constraints and the index of incoming descriptions. `ConfiguredDevice::enum_candidates` keeps aliases and incomparable variants. Only a strict superset of the same constraints excludes a weaker template. In CCU2 mode the output Fn is forcibly tied to the corresponding sum: the shared INJECT/F bits are not interpreted as an independent F5/FX selection. This is a separate rule LS10–LS12/S04, kept in the report of each path.

For an FF, `REGSET` sets the value of the local reset, not a proven initial Q. PRLD and MODE=DPRAM currently lead to an error on the executable path. Unknown/ambiguous modes are not replaced by LOGIC. Without a selected/driven CLK or LSR the wire stays undriven; assumed pull-downs for these controls are not added.

The reverse index stores templates, type placements and small exceptions of clock segments. For a required wire it finds candidates from all suitable tiles, including from a neighboring tile. All 28 million source descriptions are not expanded into a separate permanent copy. The applicability of mux and fixed descriptions is checked against the shared physical destination point.

An empty mux template with the same source as a fixed one is considered a repeated description of one transfer. Known overlays of the direct JDI/JINCK and of the delayed branch are resolved as a selection; the delay/DLL itself requires its own model when an unsupported output is reached. Other, different fixed/mux sources lead to an error. The ECLKSYNC synchronizer is not replaced by an unconditional ECLKI→ECLKO.

## Two execution scopes

`Scope::AllPlcRegisters` / `--scope all-plc` is the default CLI value. The roots are all **83 640 FFs** and the specified probes. Their data, controls and dependencies are built automatically. Other families are connected by dependencies/pins; the name of this mode does not mean support for the behavior of the whole chip.

`Scope::Observed` / `--scope observed` builds the dependencies of the selected probes and the participating pins. The observed results were checked against the all-FF mode. FFs that are not in the dependencies are not executed; their storage stays in the model. This is an optimization with an explicitly stated boundary, unsuitable for reading the dynamic state of an arbitrary uncovered FF without a new plan.

Each selected transfer has a separate executor and a source in the database; the wires are not merged into one net. The only exception to this, by identity, is the verified raw/segment representations of one clock conductor. The architecture keeps both names; `clock_aliases` explains the correspondence. The runtime and the trace use the segment name; a query through `Compiled::value(Reference::Wire(...))` resolves the raw name. No cache is created in `DeviceState` for each possible clock alias. The relation to a segment itself does not include SPINE/TAP: without the selected real transfers the value does not appear.

## Explicit user-mode profile

A run requires `--startup ready-unknown`. The profile sets an already reached user mode: outputs are enabled, PUR/GSR are inactive, the write inhibit is released. FFs start at X and get known values from the executable circuit/reset. The POR/loading/wake-up controller, the GOE/GSR/GWDIS/DONE phases and their timed transitions are not implemented by this.

An active or unparsed user `GSR.GSRMODE` is rejected before the plan is built, even if the GSR does not lie on the ordinary data chain of an observed output. Dynamic global effects cannot be ignored merely because of the selected probes.

The profile also considers access to `*_CIBTEST` inactive. These fixed descriptions stay in the architecture and are listed separately in `excluded_routes` marked **not hardware-validated**. This is an explicit assumption of the normal user mode, not implemented behavior of factory nodes. Checking the profile on a physical unit is still ahead; 1 496 unbound references are not removed from the map.

## PIO and the BASE_TYPE boundary that was found

Supported are INPUT/OUTPUT/BIDIR with an unambiguous digital direction and LVCMOS12/15/18/25/33 or LVTTL33. Aliases are allowed only if all candidates give one supported digital function. The external level is already given as 0/1/X/Z: voltages, thresholds, bank power and electrical correctness are not derived from CRAM.

The receiver reads the enabled pad after the joint action of the external and own drivers. Z at the receiver input gives X. For OUTPUT the receiver of this base model is disabled; a request for its O without a supported mode is rejected. Pull is applied in INPUT/BIDIR; in OUTPUT it is disabled according to the collected manual. The DRIVE/SLEWRATE parameters are kept in the configuration but do not become delays or currents.

DATAMUX/TRIMUX must select the direct PADDO/PADDT. For these specific fields and DCC.MODE=NONE, a single empty fallback is allowed when the other variants have observable control bits. The source status `unobserved` is kept in the resource report; this is a rule of the corresponding mode, not a general permission to consider any empty enum known.

A reproducible case P01/P04 was found: `PIOB.BASE_TYPE=OUTPUT_LVCMOS33` requires bit F1B3 in PICL1, and the subsequent application of the default `PIOB.PULLMODE=DOWN` clears it. `BitDatabase.cpp::config_to_tile_cram` applies the missing defaults after the explicitly given enums. In the natively built image R47C0/PIOB, the common strict decoder gets BASE_TYPE=NONE and PULLMODE=DOWN, although the text declares OUTPUT. The [evidence](inventory/LFE5U-85F/automatic_pio_overlay.json) is recorded.

The original textual name is not used to work around the discrepancy: it does not exist for a `.bit`. Such an image is rejected for now; the physical direction/permitted interpretation after the overlay is not declared established. A separate overlay model and a distinguishing board check in the existing P01/P04 are needed. The first native end-to-end example explicitly writes PULLMODE=NONE and stays in the unambiguous area. Pull/open-drain and readback were checked separately on synthetic configurations; this is not a check of all physical IO codes.

## The verified end-to-end example

The independent HDL contains two eight-bit counters, `a+=1` on the posedge of the first clock and `b+=3` on the negedge of the second, with a shared asynchronous reset. Yosys/nextpnr create real routing for CABGA381 and 19 GPIOs with LVCMOS33/PULLMODE=NONE; `ecppack --compress` produces a `.bit`. The emulation then reads only the image and the pinned device data.

Three variants were checked:

| Input and scope | Executable FFs | LUT4 / carry halves | Directed transfers |
|---|---:|---:|---:|
| `.config`, observed | 16 | 3 / 20 | 359 |
| `.bit`, observed | 16 | 3 / 20 | 359 |
| `.bit`, all-plc | 83 640 | 83 616 / 24 | 711 110 |

All variants have two DCC bypasses, 12 used raw/segment relations and 19 PIOs. Over 2 000 checked timestamps, the values of both counters matched an independent integer expectation, including overflows and coinciding edges. The result is a=244, b=231. In observed, two undriven HFIE inputs of the carry adapters remained; the selected functions do not depend on them and gave known results without substituting zero.

In all-plc, the undriven lines of unused blocks and their X states are kept. The match of the outputs with observed does not prove the physical behavior of all unused areas. The frequencies of the test events are a virtual scale for checking order, not a statement about a permissible board frequency.

The [native result](inventory/LFE5U-85F/automatic_execution_native.json) keeps the plan sizes, the state, the statistics and the start of the trace of each variant. Saved are the [example HDL](inventory/LFE5U-85F/automatic_execution_fixture.v), the [package constraints](inventory/LFE5U-85F/automatic_execution_fixture.lpf) and the [CLI samples](inventory/LFE5U-85F/automatic_execution_cli.json). The [milestone summary](inventory/LFE5U-85F/automatic_execution_validation.json) records the checks and the code hashes. The full working observed plans and fresh reports are in `target/automatic-compile/` after the test; the huge all-plc plan is not exported to JSON in full by the test itself.

## Running

```sh
# Build and check the reproducible example:
cargo test --release --locked --test automatic_compile -- --ignored

# Run the first clock with the same asynchronous reset from the events file:
cargo run --release --locked -- emulate target/automatic-compile/top.bit \
  --startup ready-unknown --scope observed --clock pin:G2 --cycles 4 \
  --set pin:J4=1 --probe pin:H1 \
  --stimulus target/automatic-compile/cli-events.json \
  --output target/automatic-compile/user-run.json
```

The second clock in this short CLI command is held at 1; two independent clocks are checked by the native test. For an arbitrary sequence both clocks can be given through `--stimulus` without `--clock`.

The JSON events file uses integer **nanoseconds**, as in the previous CLI. `time:0` sets the initial inputs before the kernel is built; the order of the file's records does not affect their application. `--clock` generates rises at 1,3,5… ns and falls at 2,4,6… ns. `--set`/probes accept `pin:G2` and `R38C2:Q0`. The package is set by `--package`; CABGA381 in the example is a software profile, the physical marking of the available board has not been established.

`--output` contains the plan, the image, the state, the probe samples and a bounded delta trace in rational picoseconds. The eviction of old events is accounted for by a counter. `--vcd FILE` writes the probe samples as VCD with a 1 ns timescale: every value at time 0, then a value only when its sample changes. The old `run` remains a separate prototype; `load` still only prepares a snapshot.

## Checks and remaining work

**65 tests** passed: 59 ordinary (18 unit, 5 architecture, 8 configuration/state, 16 kernel and 12 automatic assembly) plus 6 explicitly run integration tests. The new native test also checks the CLI, unsorted initial events, and the rejection for an active user GSR and an overlapped BASE_TYPE. The earlier full architectural cross-check of 197 408 clock relations, reading of CRAM/INIT/service registers, the EBR/DSP decoder and the old counter with 65 538 observations also passed again. Clippy and formatting passed. The external blink with `ECP5_TEST_BIT` was not run separately; there are no board results.

The next stage is a safe interpretation of the PIO overlaps, LUTRAM/RAMW and PRLD with shared storages, then explicit system startup/GSR and the EBR ports. Dynamic DCC/DCS/PLL/ECLK, DSP, IOLOGIC/DQS, system functions and the other modes follow under the [plan](IMPLEMENTATION_PLAN.md). All 78 research questions and 37 series of experiments remain open until the corresponding evidence; the success of the end-to-end counter does not close them.
