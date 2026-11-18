# How it is verified

The [coverage overview](overview.md#what-runs-today) lists supported and refused modes;
the [startup and digital limits](getting-started.md#known-limitations) and
[VHDL source-probing limit](projects.md#languages) qualify the results below.

Verification is layered. Input integrity, a software contract, agreement between executors, a toolchain conversion and a physical observation answer different questions. The [hardware-validation policy](../../docs/HARDWARE_VALIDATION.md#how-to-handle-a-disagreement-before-the-measurement) requires those evidence levels to remain distinct. Commands below run from the repository root. The evidence consists of checked-in tests and recorded experiments; no additional FPGA or emulator tests were run for the website.

## Input integrity

[DeviceModel::load](../../src/model/mod.rs) verifies the researched files listed in [model_manifest.json](../../docs/inventory/LFE5U-85F/model_manifest.json), then verifies the database source hashes pinned by [device_inventory.json](../../docs/inventory/LFE5U-85F/device_inventory.json). [model/catalog.rs](../../src/model/catalog.rs) rejects paths outside the allowed relative path form and SHA-256 mismatches. [prepared.rs](../../src/prepared.rs) verifies [configuration_manifest.json](../../docs/inventory/LFE5U-85F/configuration_manifest.json), its architecture fingerprint and its catalogue files. Binding also checks image dimensions, device and resource ownership. A matching filename or database revision label alone is insufficient.

```sh
cargo run --release --locked -- device
cargo run --release --locked -- load DESIGN.bit
```

The first command checks architecture inputs; the second additionally needs the native reader and checks configuration binding. See [requirements](getting-started.md#requirements). The synthetic corruption, ownership and fingerprint checks are in [device_model.rs](../../tests/device_model.rs) and [configuration_state.rs](../../tests/configuration_state.rs). They establish consistency with the pinned evidence, not completeness of the database or accuracy of every decoded behaviour.

## Default Rust suite and CI

The [CI workflow](../../.github/workflows/ci.yml) fetches the pinned database and runs formatting, clippy and:

```sh
cargo test --release --locked
```

The inventory below was counted with `rg`, counting each literal Rust test attribute once, including indented attributes in modules. The second search counts ignored attributes; the third counts unit-test attributes under `src/`. The counts are source counts, not a claim that every function ran successfully or a count of randomized vectors:

```sh
rg -n '^\s*#\[test\]' tests -g '*.rs'
rg -n '^\s*#\[ignore' tests -g '*.rs'
rg -n '^\s*#\[test\]' src -g '*.rs'
```

This gives **478 integration-test attributes: 208 without `#[ignore]`, 270 ignored** across all **93 `tests/*.rs` files** below. These are literal source attributes; a macro can register more than one test. Helpers in [tests/common](../../tests/common) and [tests/board](../../tests/board) have no such test attributes. The same count over [src/](../../src) gives **263 unit-test attributes**, separate from this integration-test total. The Python checks in [test_resource_catalogue.py](../../tests/test_resource_catalogue.py) are also separate and are not invoked by that CI command.

| Area and what is checked | Files, with `#[test]` count | Total / ignored |
|---|---|---:|
| Architecture and configuration: identities, corrupt inputs, field composition, metadata, state isolation | [device_model](../../tests/device_model.rs) 6; [configuration_state](../../tests/configuration_state.rs) 10; [resource_decode](../../tests/resource_decode.rs) 1; [configuration_ports](../../tests/configuration_ports.rs) 8 | 25 / 4 |
| Automatic planning and startup: qualified routes, carry, PIO, GSR, wake phases and initialized FFs | [automatic_compile](../../tests/automatic_compile.rs) 16; [system_controls](../../tests/system_controls.rs) 8; [configured_startup_native](../../tests/configured_startup_native.rs) 2; [system_native](../../tests/system_native.rs) 1; [sgsr_native](../../tests/sgsr_native.rs) 4 | 31 / 8 |
| Kernel and legacy execution: exact time, resolution, ordering, traces, faults, counter and benchmark | [event_kernel](../../tests/event_kernel.rs) 36; [whole_chip](../../tests/whole_chip.rs) 2; [kernel_speed](../../tests/kernel_speed.rs) 1 | 39 / 4 |
| Distributed memory: shared cells, writes, uncertain controls, planner rejection and native mapping | [lutram](../../tests/lutram.rs) 10; [lutram_native](../../tests/lutram_native.rs) 1 | 11 / 2 |
| EBR: width views, controls, collisions, initialization and wide-read replay | [ebr](../../tests/ebr.rs) 19; [ebr_native](../../tests/ebr_native.rs) 3; [ebr_wide_read_native](../../tests/ebr_wide_read_native.rs) 1; [ebr_collision_native](../../tests/ebr_collision_native.rs) 3; [ebr_narrow_rbw_native](../../tests/ebr_narrow_rbw_native.rs) 3; [ebr_r02](../../tests/ebr_r02.rs) 8; [ebr_start_native](../../tests/ebr_start_native.rs) 2 | 39 / 20 |
| DSP: arithmetic, shared stages, operand forwarding, ALU rules, native and recorded-board replay | [dsp](../../tests/dsp.rs) 19; [alu54](../../tests/alu54.rs) 8; [dsp_native](../../tests/dsp_native.rs) 1; [dsp_register_native](../../tests/dsp_register_native.rs) 2; [dsp_output_native](../../tests/dsp_output_native.rs) 1; [dsp_alu_operands_native](../../tests/dsp_alu_operands_native.rs) 1; [alu54_native](../../tests/alu54_native.rs) 14 | 46 / 23 |
| DSP capture and slice preload: HD09/HS02 board replays and configured M | [hd09_hs02_native](../../tests/hd09_hs02_native.rs) 7 | 7 / 7 |
| IO: registers, latches, delays, DDR reset, gearboxes, pads and their recorded-board lines | [io_register](../../tests/io_register.rs) 2; [io_latch](../../tests/io_latch.rs) 2; [io_delay](../../tests/io_delay.rs) 2; [iologic_native](../../tests/iologic_native.rs) 3; [io_register_native](../../tests/io_register_native.rs) 1; [io_latch_native](../../tests/io_latch_native.rs) 2; [io_delay_native](../../tests/io_delay_native.rs) 2; [ddr_reset_native](../../tests/ddr_reset_native.rs) 2; [gearbox_native](../../tests/gearbox_native.rs) 2; [gearbox71_native](../../tests/gearbox71_native.rs) 2; [differential_native](../../tests/differential_native.rs) 1; [output_delay](../../tests/output_delay.rs) 7; [r03_differential](../../tests/r03_differential.rs) 4 | 32 / 28 |
| Clocks: divider rules, gates, bridge, oscillator, PLL and generated-clock ordering | [edge_clock](../../tests/edge_clock.rs) 6; [cross_domain_fast](../../tests/cross_domain_fast.rs) 3; [oscillator_dividers](../../tests/oscillator_dividers.rs) 1; [clock_gate_native](../../tests/clock_gate_native.rs) 2; [edge_clock_native](../../tests/edge_clock_native.rs) 2; [eclk_bridge_native](../../tests/eclk_bridge_native.rs) 2; [oscillator_native](../../tests/oscillator_native.rs) 2; [pll_native](../../tests/pll_native.rs) 1; [dcs_native](../../tests/dcs_native.rs) 3; [eclksync_none](../../tests/eclksync_none.rs) 5; [eclksync_start](../../tests/eclksync_start.rs) 2; [pll_feedback_native](../../tests/pll_feedback_native.rs) 1; [pll_reset_native](../../tests/pll_reset_native.rs) 3 | 33 / 23 |
| Executor equivalence: samples, traces, reset order, floating pads and reference designs | [fast_mode](../../tests/fast_mode.rs) 22; [fast_bidir](../../tests/fast_bidir.rs) 2; [fast_reset_order](../../tests/fast_reset_order.rs) 8; [fast_trace](../../tests/fast_trace.rs) 1; [fast_tristate_external](../../tests/fast_tristate_external.rs) 2; [reference_designs](../../tests/reference_designs.rs) 8 | 43 / 40 |
| Names and header pins: HDL index order, cross-probing, pad levels, frequency and duty | [names_native](../../tests/names_native.rs) 1; [names_bits](../../tests/names_bits.rs) 4; [names_ranges_native](../../tests/names_ranges_native.rs) 1; [header](../../tests/header.rs) 6 | 12 / 5 |
| Live runner and CLI: board inputs, peripherals, publication, history and replay | [live_board](../../tests/live_board.rs) 13; [live_peripherals](../../tests/live_peripherals.rs) 11; [live_switches](../../tests/live_switches.rs) 6; [live_header](../../tests/live_header.rs) 3; [cli_board_inputs](../../tests/cli_board_inputs.rs) 6; [cli_peripherals](../../tests/cli_peripherals.rs) 10; [published_values](../../tests/published_values.rs) 2 | 51 / 48 |
| Fabric, overlays and geometry: connectivity, cache round trips, parallel equality and window load order | [fabric](../../tests/fabric.rs) 2; [fabric_overlay](../../tests/fabric_overlay.rs) 3; [overlay_cache](../../tests/overlay_cache.rs) 8; [geometry_cache](../../tests/geometry_cache.rs) 6; [open_sequence](../../tests/open_sequence.rs) 1 | 20 / 7 |
| Projects and HDL flow: creation, copies, builds, source diagnostics, caches and real-suite frontends | [flow](../../tests/flow.rs) 28; [new_project](../../tests/new_project.rs) 8; [demo_copy](../../tests/demo_copy.rs) 6; [examples](../../tests/examples.rs) 2 | 44 / 19 |
| Peripherals and USRMCLK: UART results, frames, audio traces and board-record reads | [adc](../../tests/adc.rs) 3; [audio](../../tests/audio.rs) 2; [hdmi](../../tests/hdmi.rs) 3; [oled](../../tests/oled.rs) 1; [edid](../../tests/edid.rs) 4; [rtc](../../tests/rtc.rs) 7; [sd_native](../../tests/sd_native.rs) 4; [sdcard](../../tests/sdcard.rs) 1; [sdram](../../tests/sdram.rs) 4; [spiflash](../../tests/spiflash.rs) 11; [usrmclk](../../tests/usrmclk.rs) 3 | 43 / 31 |
| DTR: conversion timing, final code, profile override and board-count fallback | [dtr_native](../../tests/dtr_native.rs) 2 | 2 / 1 |

An active test can still need the pinned database: [fabric.rs](../../tests/fabric.rs) loads the full model. Conversely, some fast-executor comparisons are active without a native bridge because they construct a synthetic kernel, as in [cross_domain_fast.rs](../../tests/cross_domain_fast.rs). The default suite is therefore not synonymous with either “synthetic only” or “all fast-mode tests.” It checks implemented contracts and regressions; it cannot establish physical timing or untested resource modes.

### Designs that CI builds itself

Three default tests run built designs: `uart_hello_over_the_fabric` and
`blinky_cycle_over_the_fabric` in [fabric_overlay.rs](../../tests/fabric_overlay.rs), and
`chosen_options_give_the_same_trace_as_the_defaults` in [fast_mode.rs](../../tests/fast_mode.rs). CI
builds those two designs with its own Yosys and nextpnr
([build_test_fixtures.sh](../../packaging/build_test_fixtures.sh)). Place and route differ between
machines even with the same tool versions (uart_hello and life place differently on Linux than on
the Mac), so these tests check properties that hold for any placement: the pads and the clock's
pad are in use, every net starts at a site's pin, each net shows one value at a time, and two fast
programs give the same trace. None of them compares a build with a stored placement or a stored
configuration. A run on Linux with Ubuntu's older Yosys 0.52 and nextpnr 0.9, from a fresh copy
without the Mac's fixtures, passed them all (2026-11-16). The opt-in tests below either replay
stored configurations against recorded board lines, which are not rebuilt, or check behaviour that
a build shows whatever its placement (the UART text, a counter's value).

## Native integration and stored-fixture replay

The [test guide](tests.md) gives the per-area opt-in sequence, prerequisites, common skips and measured times.

Read the individual `#[ignore = "..."]` prerequisites first. [automatic_compile.rs](../../tests/automatic_compile.rs), [ebr_native.rs](../../tests/ebr_native.rs) and [dsp_native.rs](../../tests/dsp_native.rs) build fixtures with Yosys, nextpnr and ecppack, then load and compare text configuration and bitstream execution. [resource_decode.rs](../../tests/resource_decode.rs) compares CRAM, resource settings and INIT. [device_model.rs](../../tests/device_model.rs) cross-checks the full architecture and saved native clock relationships. These need a compatible database, native reader and the tools their source names.

Other ignored tests replay saved configurations and UART observations, such as [gearbox_native.rs](../../tests/gearbox_native.rs), rather than rebuilding a design or programming a board. [fast_mode.rs](../../tests/fast_mode.rs) builds its references on demand the first time, except that its compiler-override check still needs a prepared `target/reference/life`; build that case with the [example workflow](../../examples/ulx3s/README.md#build). Some DSP checks depend on fixture outputs from another test; follow their prerequisites when selecting individual tests. The skipped [whole_chip test](../../tests/whole_chip.rs) requires an external blink bitstream through `ECP5_TEST_BIT`. Thus `--ignored` is an opt-in selection, not a guarantee of self-contained fixture setup.

Tool agreement checks extraction and conversion in those tool versions. It does not prove that the tools' simulation assumptions are silicon semantics. A native replay of a previously recorded board line is also distinct from taking a fresh physical observation.

## Executor equivalence

[fast_mode.rs](../../tests/fast_mode.rs) runs identical stimuli through both executors and compares probe samples for UART, debounce, PLL, DDR, oscillator, EBR and CPU designs. Resource-level comparisons in [lutram.rs](../../tests/lutram.rs), [ebr.rs](../../tests/ebr.rs), [dsp.rs](../../tests/dsp.rs), [alu54.rs](../../tests/alu54.rs), [io_register.rs](../../tests/io_register.rs) and [cross_domain_fast.rs](../../tests/cross_domain_fast.rs) stress capture order, memory state and generated clocks. A comparison only establishes agreement for its inputs and observations; it does not supply an independent physical oracle.

For byte-level output, `fast_mode_writes_the_kernel_vcd_on_the_register_benches` in [dsp_register_native.rs](../../tests/dsp_register_native.rs) reads both VCD files as bytes and asserts equality. [fast_bidir.rs](../../tests/fast_bidir.rs) checks the CLI's VCD including floating-pad `Z`. The [fast-mode checks record](../../docs/FAST_MODE.md#checks) also lists saved byte-identical VCD comparisons on reference designs. These are waveform comparisons, not equality of internal causal delta traces: fast mode has no such trace.

The specifically named `a_run_changing_program_gives_the_same_trace` in [fast_mode.rs](../../tests/fast_mode.rs) compares an ordinary fast run with one cycling through generated tuning candidates. It checks samples, EBR/LUTRAM contents, bulk reads and snapshot restoration. It complements kernel equivalence by testing state transfer between optimized programs. To select these ignored checks after preparing their prerequisites:

```sh
cargo test --release --locked --test fast_mode -- --ignored --test-threads=1
cargo test --release --locked --test dsp_register_native -- --ignored --test-threads=1
```

## Physical ULX3S evidence

The [hardware record](../../docs/HARDWARE_VALIDATION.md) identifies a ULX3S **85K v3.0.8**, JTAG IDCODE **`0x41113043`**, package **CABGA381**; the package text marking was not read. The recorded experiments load SRAM over JTAG with `openFPGALoader -b ulx3s`; they do not write flash. Host software reads UART results, with versions, hashes, gateware, constraints and raw captures retained in each inventory record. For concrete loading/readout commands use the [HS01](../../docs/inventory/LFE5U-85F/hs01_init_slice/README.md), [HC05](../../docs/inventory/LFE5U-85F/hc05_pll_lock/README.md) and [HR01](../../docs/inventory/LFE5U-85F/hr01_picorv32_console/README.md) procedures. Tests replaying these records do not themselves issue those board loads.

The policy's evidence vocabulary is:

| Level | Scope |
|---|---|
| Documented | A particular manual version states a rule. |
| Tools checked | The selected tools produce or decode a particular encoding and circuit. |
| Observed on the board | The recorded setup produces the captured result. |
| Repeated on the board | That result repeats for the stated loads, instances and conditions. |

These definitions and the requirement to distinguish hypotheses come from [the evidence policy](../../docs/HARDWARE_VALIDATION.md#how-to-handle-a-disagreement-before-the-measurement). For example, [HB01](../../docs/inventory/LFE5U-85F/hb01_pdp36/README.md) distinguishes wide-read half order and controls, while [HI05](../../docs/inventory/LFE5U-85F/hi05_ioreg/README.md) tests competing reset/enable interpretations. Repeated undefined collisions would still not establish a universal port priority.

Dates, statuses and limits come from the experiments in [Results](../../docs/HARDWARE_VALIDATION.md#results). “Repeated” always retains the source record's scope.

| Date | Series | What was checked | Status and record |
|---|---|---|---|
| 2026-10-29 | HS01 | Fabric FF initialization after configuration | [Repeated; wake-up input settling open](../../docs/inventory/LFE5U-85F/hs01_init_slice/README.md) |
| 2026-10-29 | HS01, PIO observation | LED drive and button pulls | [Repeated under the recorded IO settings](../../docs/inventory/LFE5U-85F/hs01_init_slice/README.md) |
| 2026-10-29 | HC05 | PLL ratio, early output and LOCK | [Repeated; UART replay matches](../../docs/inventory/LFE5U-85F/hc05_pll_lock/README.md) |
| 2026-11-16 | HC07 (builds hc06, hc06b) | PLL RST/STDBY stop, restart and LOCK | [Three loads per build; ramp unmodelled, fast mode refuses live controls](../../docs/inventory/LFE5U-85F/hc07_pll_reset/README.md) |
| 2026-11-16 | HB11 | Narrow EBR READBEFOREWRITE | [Repeated; both executors replay the record](../../docs/inventory/LFE5U-85F/hb11_ebr_narrow_rbw/README.md) |
| 2026-11-17 | HB09 | Equal-width EBR collisions on one edge | [Repeated; width-18 double writes remain uncertain](../../docs/inventory/LFE5U-85F/hb09_ebr_collision/README.md) |
| 2026-11-17 | HB09P | Raw PDP read/write collisions | [Three loads agree; both executors return the new word](../../docs/inventory/LFE5U-85F/hb09p_pdp_collision/README.md) |
| 2026-11-17 | HB09Ps | PDP collisions from configuration | [Repeated window; event replay matches; fast mode runs the profile](../../docs/inventory/LFE5U-85F/hb09p_pdp_collision/README.md) |
| 2026-11-17 | HD09 variants | DSP capture edges with dividers disabled | [Repeated; kernel replays match, fast mode refuses](../../docs/inventory/LFE5U-85F/hd09_dsp_clkdiv/README.md) |
| 2026-11-17 | HS02/HS02s | Synchronous and asynchronous slice preload | [Repeated; release/M-change race remains uncertain](../../docs/inventory/LFE5U-85F/hs02_slice_prld/README.md) |
| 2026-11-17 | HY04 (DTR) | DTR conversion codes and busy restart | [Three loads; no Celsius scale or intermediate-code replay](../../docs/inventory/LFE5U-85F/hy04_dtr/README.md) |
| 2026-11-17 | HC02 (DCS) | Protected clock switching and static selection | [Three loads per build; one static observer record unresolved](../../docs/inventory/LFE5U-85F/hc02_dcs/README.md) |
| 2026-10-29 | HI02 | Side-bank DDR and IO registers | [Repeated digital samples; pad discharge differs](../../docs/inventory/LFE5U-85F/hi02_iologic/README.md) |
| 2026-10-29 | HC06 | OSC frequency and divider encodings | [Repeated; frequency remains chip-specific](../../docs/inventory/LFE5U-85F/hc06_osc/README.md) |
| 2026-11-08 | HR01 | PicoRV32 console and replaced firmware | [Repeated text and reported cycle count](../../docs/inventory/LFE5U-85F/hr01_picorv32_console/README.md) |
| 2026-11-11 | HC01 | Dynamic DCCA enable | [Repeated; both executors replay it](../../docs/inventory/LFE5U-85F/hc01_dcca/README.md) |
| 2026-11-11 | HI03 | X2 gearbox and edge clocks | [Repeated; start phase qualified by HI04](../../docs/inventory/LFE5U-85F/hi03_gearbox/README.md) |
| 2026-11-11 | HI04/HI04b | STOP, reset, alignment and divider startup | [Repeated; build-dependent startup edge remains](../../docs/inventory/LFE5U-85F/hi04_eclk/README.md) |
| 2026-11-11 | HI05 | IO register CE, local reset and GSR | [Repeated windows; initial readout caveat](../../docs/inventory/LFE5U-85F/hi05_ioreg/README.md) |
| 2026-11-11 | HI06 | Bridge mode and synchronization | [Repeated; replay retains startup-phase caveat](../../docs/inventory/LFE5U-85F/hi06_eclkbridge/README.md) |
| 2026-11-11 | HI07/HI07b | 7:1 serializer and capture order | [Repeated; recorded lines match](../../docs/inventory/LFE5U-85F/hi07_gearbox71/README.md) |
| 2026-11-11 | HI08/HI08b/HI08c | DELAYF counter and protocol | [Repeated; out-of-protocol differences retained](../../docs/inventory/LFE5U-85F/hi08_delay/README.md) |
| 2026-11-11 | HI09 | DDR local reset and GSR | [Repeated; early UART line is partly missed](../../docs/inventory/LFE5U-85F/hi09_ddr_reset/README.md) |
| 2026-11-12 | HD02 | ALU operation table | [Repeated; whole fast pass and kernel prefix replayed](../../docs/inventory/LFE5U-85F/hd02_alu54/README.md) |
| 2026-11-12 | HD03 | ALU operands and cascade | [Predicted subset repeats; feedback loops remain](../../docs/inventory/LFE5U-85F/hd03_alu54_inputs/README.md) |
| 2026-11-12 | HD04 | Cascade sign, flags, enable and bypass | [Predicted subset repeats; fast mode refuses loops](../../docs/inventory/LFE5U-85F/hd04_alu54_cascade/README.md) |
| 2026-11-12 | HD05 | Registered ALU chain | [Repeated; full fast pass replayed](../../docs/inventory/LFE5U-85F/hd05_alu54_chain/README.md) |
| 2026-11-12 | HD06/HD06b | ALU output enable and reset | [Repeated; enabled GSR was not measured here](../../docs/inventory/LFE5U-85F/hd06_alu54_reset/README.md) |
| 2026-11-15 | HD07/HD07s | DSP GSR and reset ownership | [Repeated; bounded executor replays recorded](../../docs/inventory/LFE5U-85F/hd07_dsp_gsr/README.md) |
| 2026-11-15 | HD08 variants | ALU C/opcode stages and rounding source | [Repeated; IN_1 unresolved and refused](../../docs/inventory/LFE5U-85F/hd08_alu54_cop/README.md) |
| 2026-11-16 | HB02 | EBR output latch before the first read | [Repeated readouts; both executors start latches at zero](../../docs/inventory/LFE5U-85F/hb02_ebr_start/README.md) |
| 2026-11-11 | HI10/HI10ff | IO latch transparency and reset | [Repeated; latch/FF controls compared](../../docs/inventory/LFE5U-85F/hi10_io_latch/README.md) |
| 2026-11-11 | HD01 | Multiplier output CE/reset selection | [Repeated; tool parameter mismatch exposed](../../docs/inventory/LFE5U-85F/hd01_mult_output/README.md) |
| 2026-11-11 | HB01 | Wide PDP read order and controls | [Repeated; both executors replay the record](../../docs/inventory/LFE5U-85F/hb01_pdp36/README.md) |
| 2026-11-11 | HR02 | Inferred wide memory and multiplier designs | [Repeated UART arithmetic results](../../docs/inventory/LFE5U-85F/hr02_ram32_mac/README.md) |
| 2026-10-29 | HI02c | Top-bank IOLOGIC versus side bank | [Repeated digital equivalence; analog tail differs](../../docs/inventory/LFE5U-85F/hi02_iologic/README.md) |

[HP02](../../docs/inventory/LFE5U-85F/hp02_lvcmos33d/README.md) is prepared but not run on the board. Its presence in the inventory is not a result.

## The window's own record

[gui/src/main.rs](../../gui/src/main.rs) reads `ECP5_GUI_TRACE` and `ECP5_GUI_SCRIPT`; [trace.rs](../../gui/src/trace.rs) defines the timed commands and tab-separated record. A script can run, pause, step, change rate, select value colours, zoom, press buttons and quit. The record measures frame and map drawing time, changed wires in sight, missed or delayed visible changes, value colours and GPU board LED readback. `# layout` lines record panel geometry. The [GPU map](../../gui/src/gpu_map.rs) and [board renderer](../../gui/src/board_view.rs) supply their own draw evidence.

For an already prepared script and reference build:

```sh
ECP5_GUI_TRACE=target/gui-trace.tsv ECP5_GUI_SCRIPT=target/gui-script.txt \
  cargo run --release --locked -p ecp5_gui -- target/reference/uart_hello/top.config \
  --names target/reference/uart_hello/top_routed.json
```

Use the script syntax in [trace.rs](../../gui/src/trace.rs), relative to the moment the design becomes ready. These hooks support repeatable window measurements; no universal frame-rate result is asserted here. Saved [board-renderer QA](../../docs/qa/ulx3s_3d/README.md#in-the-gui) checks appearance; [fabric tests](../../tests/fabric.rs) check drawn connectivity, while [live_board.rs](../../tests/live_board.rs) exercises backend publication and history. Neither a screenshot nor a backend sample proves that every displayed frame was correct, which is why the draw record is separate.

## Explicitly open

The [settings audit](../../docs/SETTINGS_COVERAGE.md#silent-risks) records unread configuration-port settings with possible digital effects. The [resource coverage](../../docs/RESOURCE_COVERAGE.md) does not declare any family fully closed. Physical POR/programming transport, general analog IO behaviour, physical routing delays, unmeasured collisions, unsupported DQS/system modes and arbitrary external contention remain outside the demonstrated scope. Fast-mode restrictions and selected unknown handling are in [FAST_MODE](../../docs/FAST_MODE.md#what-it-covers) and [kernel/fast.rs](../../src/kernel/fast.rs); opcode IN_1 is explicitly rejected in [compile/dsp.rs](../../src/compile/dsp.rs).

Historical documents sometimes say there are no board results or no current ALU execution. Compare those snapshots with the dated records above and the linked code. Reported performance is fixture- and machine-specific; cross-chip repeatability, flash power-up behaviour and exhaustive equivalence over arbitrary bitstreams are not established by the cited tests or board runs.
