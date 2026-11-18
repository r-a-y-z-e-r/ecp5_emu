# Architecture overview

## Problem and goals

The project asks how to make a placed and routed FPGA design understandable while executing the configuration that can also be loaded onto the board. The [Roadmap's goal](../../ROADMAP.md#goal) is an educational tool for the ECP5 and ULX3S: inspect the fabric, interact with peripherals, and run soft processors. Its [principles](../../ROADMAP.md#principles) prioritize student designs, keep unknown behaviour explicit, require cross-checks between executors, and separate HDL names from execution. These are design goals, not a claim of complete device coverage.

The implementation separates permanent architecture, loaded configuration, mutable state and execution. This matters because an unused resource still belongs to the chip, while a decoded setting does not establish executable behaviour. [DeviceModel](../../src/model/mod.rs), [ConfigurationImage](../../src/image.rs), [ConfiguredDevice](../../src/prepared.rs) and [DeviceState](../../src/state.rs) express those boundaries in code. The [structural specification](../../docs/STRUCTURAL_MODEL.md#three-linked-layers) explains the intended distinction; older milestone descriptions should be read alongside the current implementation.

The pipeline below follows the [CLI load path](../../src/main.rs), [planner](../../src/compile/mod.rs) and [fast-program generator](../../src/kernel/fast.rs). The names file joins observation and reference resolution, not configuration decoding.

```text
HDL --Yosys--> netlist --nextpnr--> configuration --ecppack--> .bit
                                  |                    |
                                  +-- routed JSON      v
                                         |      native image reader
Trellis DB + pinned inventory             |             |
            |                             |      ConfigurationImage
        DeviceModel ----------------------+-------------+
            |                             |             |
            +------------------------ ConfiguredDevice  |
                                          |             |
                                     execution planner  |
                                          |             |
                                    event kernel <--- DeviceState
                                          |
                                     fast C program
                                          |
routed JSON --> Names --> probes, stimulus, CLI and live GUI
```

## Inputs and toolchain boundary

The emulator consumes `.bit` or text `.config`; synthesis and placement remain external. [examples/ulx3s/build.sh](../../examples/ulx3s/build.sh) runs Yosys, nextpnr and ecppack and retains the routed JSON for names. At load time, [image.rs](../../src/image.rs) reads the original bytes and sends them through standard input to [tools/trellis_image_reader.cpp](../../tools/trellis_image_reader.cpp). The native bridge invokes libtrellis's bitstream deserializer or text parser and emits CRAM, EBR initialization and service metadata. It does not open a hardware programming transport.

For bitstreams, this direct path avoids a text round-trip that loses service values. The image keeps its source bytes, source SHA-256, decoded-content SHA-256, comments and native CTRL/USERCODE/SED values. The flag `parser_defaults_may_apply` explicitly says that native parser state is not proof of a command's presence in the original stream. Text configurations retain directives and comments but do not acquire native register defaults. These distinctions are enforced by [ConfigurationImage::from_snapshot](../../src/image.rs) and explained in [configuration loading](../../docs/CONFIGURATION_STATE.md#what-is-loaded-now).

The database parser in [database.rs](../../src/database.rs) preserves fixed connections, mux alternatives, selection-bit polarity, enum patterns, word groups and source lines. Architecture loading checks the hashes of researched inputs through [model_manifest.json](../../docs/inventory/LFE5U-85F/model_manifest.json); [device_inventory.json](../../docs/inventory/LFE5U-85F/device_inventory.json) pins the database files themselves. [model/catalog.rs](../../src/model/catalog.rs) verifies relative paths and SHA-256 values, and [DeviceModel::load](../../src/model/mod.rs) checks the input set and revisions. Thus the manifest is an integrity contract for the selected model inputs, not evidence that Trellis describes every physical function.

## Device identity, topology and geometry

The [device passport](../../docs/DEVICE_PASSPORT.md#device-and-source-data) records **14,231 tiles of 148 types**. Its [resource table](../../docs/DEVICE_PASSPORT.md#main-functional-resources) gives **83,640 LUT4s and fabric flip-flops**, **208 EBRs**, **156 multipliers** and **78 ALU54s**. Those are architectural capacities, not the number of active elements in a run. The [core inventory](../../docs/CORE_STATUS.md#what-works) separately records **54,721 source site references** and **53,690 keyed resources**; repeated site descriptions do not create additional physical blocks.

[DeviceModel](../../src/model/mod.rs) stores tiles, resources, parent relationships, source references and package pins with distinct identities. Vendor site coordinates remain separate from fabric coordinates. Internal PLC nodes use templates under their resource owners. A package ball can map to a PIO without becoming an ordinary user pin in every package. These distinctions prevent tile fragments, alternative representations and package bonding from being counted as independent logic capacity. The [binding study](../../docs/RESOURCE_BINDINGS.md) documents why simple deduplication of source names gives misleading resource counts.

Wires in [model/topology.rs](../../src/model/topology.rs) have a location and native name. Connections retain direction, selection constraints and provenance; endpoints can explicitly belong to another density or fall outside the device. Clock-segment relationships are resolved separately from raw identities. The [core inventory](../../docs/CORE_STATUS.md#what-works) reports **28,013,216 placed route descriptions**, including inapplicable variants. This is not a unique-wire count or an active-connection count. A complete measured count of physical conductors is not established by that record; neither wire names nor drawn pieces should be substituted for it.

There is also an older implementation: [chip.rs](../../src/chip.rs) builds a `Chip` using [routing.rs](../../src/routing.rs), whose enabled connections join nets through a union structure. [main.rs](../../src/main.rs) still directs the legacy `run` command there. The modern `emulate` path uses `ConfiguredDevice` and the directed planner. Describing the union structure as the current event kernel's architecture would therefore conflate different implementations.

View geometry is another layer. [layout.rs](../../src/layout.rs) derives design segments from plan routes; [fabric.rs](../../src/fabric.rs) lays out the die's database wires, arcs and sites; [fabric_overlay.rs](../../src/fabric_overlay.rs) maps the running design over that fabric. Their lanes and site shapes make connectivity readable. As the [structural specification](../../docs/STRUCTURAL_MODEL.md) states, architectural coordinates do not establish the exact metallization geometry. Inference: the map is useful for following implemented connectivity, but it should not be used as a physical delay or floorplan measurement.

## Configuration decoding and state ownership

[config.rs](../../src/config.rs) parses text directives, tile groups, words, arcs and enums, preserving rewritten enum values where the input repeats them. [decode.rs](../../src/decode.rs) and its [EBR](../../src/decode/ebr.rs), [DSP](../../src/decode/dsp.rs) and [PIO](../../src/decode/pio.rs) modules interpret resource fields against shared CRAM. A source fragment's default is not automatically the effective setting of an entire resource. Distributed fields and shared bits need a joint interpretation, especially where an enum overlaps another resource or a routing selection.

[prepared.rs](../../src/prepared.rs) merges the catalogues listed in the configuration manifest, validates their ownership and dimensions, and binds the result to the image and architecture. Its configuration fingerprint includes architecture, source, decoded content and catalogue identity. Raw mux selection removes strict-subset fallbacks but preserves incomparable candidates. Word decoding returns unknown bits for missing or contradictory replicas. These behaviours implement the uncertainty described in [Fields and routes](../../docs/CONFIGURATION_STATE.md#fields-and-routes), rather than choosing whichever source appeared first.

[state.rs](../../src/state.rs) owns mutable LUT cells, registers, memories and resource pipeline state. A LUT used as distributed RAM shares its cell storage with its LUT view; port widths of an EBR are views of common memory. State is bound to a configuration fingerprint, so an unrelated image cannot silently reuse it. The following ownership diagram summarizes [prepared.rs](../../src/prepared.rs), [state.rs](../../src/state.rs) and [kernel/mod.rs](../../src/kernel/mod.rs).

```text
DeviceModel                 immutable architecture and readable identities
    |
ConfiguredDevice            architecture + image + decoded fields + fingerprint
    |
    +---- DeviceState       cells, register values, memory, resource stages
    |          ^
    |          | writes through execution phases
    +---- Kernel            drivers, resolved wires, queue, timers, trace
               |
               +---- FastProgram / FastSim
                     generated slot layout + copied current execution state
```

## From configuration to an executable plan

The [planner](../../src/compile/mod.rs) begins with package selection, startup policy, input drives and observation roots. Its reverse indexes identify potential producers and incoming routes, then walk dependencies. Mode checks decide whether a source is a LUT, carry, register, memory, clock or other resource. They qualify connections that a raw CRAM match cannot safely activate. The report retains fields, selected and excluded routes, undriven wires, clock aliases, resource uses and limitations. The original [automatic-execution milestone](../../docs/AUTOMATIC_EXECUTION.md) describes the motivation; its initial rejection list is historical.

With `observed`, the walk is rooted in requested observations and required controls. This can omit unrelated sequential behaviour while keeping the architecture and storage present. With `all-plc`, the current code also roots fabric registers, active EBR, active multipliers and ALUs, and active RAMW writes. It does not promise that every device family executes. An unsupported mode encountered by planning fails with context; an undriven signal stays explicit rather than receiving an inferred pulldown. These scope rules are in [compile/mod.rs](../../src/compile/mod.rs).

Resource planners supply executable contracts: [lutram.rs](../../src/compile/lutram.rs) maps RAMW and shared cells; [ebr.rs](../../src/compile/ebr.rs) assembles composite memory controls; [dsp.rs](../../src/compile/dsp.rs) qualifies multiplier and ALU stages; [iologic.rs](../../src/compile/iologic.rs) and [eclk.rs](../../src/compile/eclk.rs) map supported IO and edge-clock paths. Rejecting an ambiguous route, PRLD mode or unmeasured register profile prevents a plausible-looking waveform from being mistaken for established behaviour. This is a deliberate boundary of the model, as illustrated by [planner rejection tests](../../tests/automatic_compile.rs).

## Event kernel: time, uncertainty and causes

The exact executor is [kernel/mod.rs](../../src/kernel/mod.rs). [kernel/time.rs](../../src/kernel/time.rs) represents nonnegative rational picoseconds with checked arithmetic, so repeated fractional periods need no per-edge rounding. The queue batches equal timestamps. It rejects conflicting updates to one driver, discards obsolete timer generations and applies the valid input updates together. A closed timestamp cannot later receive another stimulus. Exact virtual-time arithmetic is not a claim about measured propagation delays.

Combinational elements evaluate against a common wave snapshot and publish together. After stabilization, sequential elements compute captures before those captures are published. LUTRAM writes, EBR captures, DSP stages and IOLOGIC participate in this separation. A register-generated clock can trigger a later delta wave at the same virtual time. Coincident independent clocks therefore do not gain an insertion-order priority. The implementation's `settle`, `publish` and capture functions in [kernel/mod.rs](../../src/kernel/mod.rs), and the [event-kernel tests](../../tests/event_kernel.rs), define this digital ordering.

[logic.rs](../../src/logic.rs) defines `0`, `1`, `X` and `Z`. An unknown LUT address merges compatible entries and can still yield a known common result. Driver resolution releases `Z`, accepts agreeing levels and produces `X` for conflict; [kernel/mod.rs](../../src/kernel/mod.rs) consults pull drivers only when the strong drivers release. Possible edges, unknown enables and uncertain resets merge alternatives. Reset priority and coincident data/clock sampling follow the explicit [digital rules](../../docs/EVENT_KERNEL.md#explicit-digital-rules), not an analog race simulation.

Resource kernels add their own contracts: [LUTRAM](../../src/kernel/lutram.rs) computes uncertain writes conservatively, [EBR](../../src/kernel/ebr.rs) tracks common memory and collisions, and [DSP](../../src/kernel/dsp.rs) and [ALU](../../src/kernel/alu.rs) implement qualified arithmetic and register stages. [IOLOGIC](../../src/kernel/iologic.rs) contains the recorded register, latch and gearbox paths. [PLL](../../src/kernel/pll.rs) learns a periodic reference and schedules outputs; [OSC](../../src/kernel/osc.rs) schedules a free-running source. A digital input delay path currently passes data without modelling its physical delay; its control counter is separate, as documented in [HI08](../../docs/inventory/LFE5U-85F/hi08_delay/README.md).

Watched transitions carry time, delta, phase, previous value, new value and direct causes. The bounded trace records evictions; a delta limit diagnoses non-settling feedback, and an execution fault prevents silent continuation. These mechanisms are in [kernel/mod.rs](../../src/kernel/mod.rs). The [CLI adapter](../../src/command_emulate.rs) exports probe samples as VCD and can write a JSON report in event mode. VCD is an observation stream, not the full causal delta trace.

The [performance record](../../docs/EVENT_KERNEL.md#performance) reports an Apple M3 Max counter run changing from **53.3 s to 14.2 s**, then **7.7 s** after another pass, with peak memory changing from about **1.4 GB to 465 MB** in that latter comparison. Its board-fixture table reports roughly **18.3k–91k reference cycles/s**. These are recorded measurements under stated fixtures and loading conventions. Buffer collapsing and clock groups reduce work; the record explicitly notes that the second pass changes delta numbering and within-time trace order. Output agreement must be stated at the observation level actually compared.

## Compiled fast mode

[kernel/fast.rs](../../src/kernel/fast.rs) lowers the kernel's executable structure into slots, dependency-ordered gates and clock-group captures. [fast.rs](../../src/fast.rs) compiles the generated C with clang, loads the shared library and advances periodic edge trains. Source and compiler flags identify cached programs. Large generated sources can be split into compilation units. [kernel/fast_dsp.rs](../../src/kernel/fast_dsp.rs) adds multiplier/ALU expressions and capture state to the same machinery; DSP is supported within qualified profiles.

The generated program folds constants and aliases, removes irrelevant dependencies and folds gate cones into lookup tables. Gates needed only by enabled register captures can be evaluated on demand. Probes constrain these optimizations because observed values must be current. Large settles are partitioned into shared work and independent worker parts; the caller publishes shared values before the parts run. A persistent pool lets another worker or the caller claim an unstarted part. The code and its limits are in [kernel/fast.rs](../../src/kernel/fast.rs); the [fast-mode design notes](../../docs/FAST_MODE.md#how-it-works) explain the measured tradeoffs.

Tuning in [fast.rs](../../src/fast.rs) measures alternate thread, folding and partition choices while advancing the same run, compiling candidates in the background and carrying state between layouts. Environment-pinned options stay fixed; `FAST_TUNE=0` disables tuning. [a_run_changing_program_gives_the_same_trace](../../tests/fast_mode.rs) checks that changing fast programs preserves samples, memory and restore behaviour. It is a fast-program comparison; event-kernel equivalence is established by the separate sample comparisons in that file and resource tests.

The ordinary fast program requires known values where computation reads them. An event-kernel warm start can make register and memory stages known before takeover. Internal EBR unknowns remain tracked and matter when read; an unobserved unknown is not blanket permission to substitute zero. Floating pads can be observed as `Z`, while logic reading them is refused. External contention, unsupported cycles and certain clock arrangements also restrict the compiled path. See [kernel/fast.rs](../../src/kernel/fast.rs), [fast_dsp.rs](../../src/kernel/fast_dsp.rs), [fast_bidir.rs](../../tests/fast_bidir.rs) and the [coverage/refusal list](../../docs/FAST_MODE.md#what-it-covers).

Speed depends strongly on the configured logic. The original [speed table](../../docs/FAST_MODE.md#speed) gives **41–46 million reference cycles/s** for `uart_hello` and **1.90–1.94 million** for PicoRV32. Later [full-chip Life measurements](../../docs/FAST_MODE.md#the-full-chip-life-life_full) report **100–112 kHz** for `life_full` with **10 threads**, and **2.31–2.39 MHz** for the CPU after that optimization pass. These tables describe different experiments; they are not universal throughput guarantees or fresh measurements of this website build.

## Names, live execution and the GUI

[names.rs](../../src/names.rs) reads nextpnr's routed JSON. A net resolves through its routing source; top-level ports resolve through placed IO cells to package pins. Bus names expand in bit order. Missing or ambiguous sources retain diagnostics. The overlay affects references and labels, while image decoding and the plan's functions still come from CRAM. The [HDL-names guide](../../docs/HDL_NAMES.md) describes this division for probes, stimuli and waveforms.

[live.rs](../../src/live.rs) loads a design in a worker thread, plans it with configured startup and observed scope, warms the kernel and runs fast mode. Commands handle run/pause, stepping, buttons, UART keys, pacing and history. Published shared state supplies values and changes for the window. History uses fast snapshots plus an input log to restore and replay recorded execution. [board.rs](../../src/board.rs) maps the board clock, buttons, LEDs and UART to package balls; [uart.rs](../../src/uart.rs) converts timed line samples and queued bytes into a console interface.

The [egui application](../../gui/src/app.rs) combines the [fabric view](../../gui/src/fabric_view.rs), [inspector](../../gui/src/inspector.rs), console and board pane. [gpu_map.rs](../../gui/src/gpu_map.rs) uploads geometry, patches changed wire records, and uses shaders and a compute pass to draw changing wires. The inspector exposes pin values, names, settings and supported internal views such as LUT truth tables and FF controls. Display animation represents changes at a readable pace; it supplies no physical routing delay to the executor. This separation follows [live.rs](../../src/live.rs) and the [structural observation design](../../docs/STRUCTURAL_MODEL.md).

[board_model.rs](../../gui/src/board_model.rs) loads the glTF exported from the [KiCad/Blender workflow](../../tools/ulx3s_3d/README.md); [board_view.rs](../../gui/src/board_view.rs) renders it, lights the driven LEDs and sends button interactions back to the app. The [QA record](../../docs/qa/ulx3s_3d/README.md) lists simplified geometry and uncertain markings. `ECP5_GUI_SCRIPT` drives timed UI actions and `ECP5_GUI_TRACE` records frames, wire visibility/value checks and board GPU readbacks. [trace.rs](../../gui/src/trace.rs) defines those hooks; they measure presentation, not hardware timing.

## Soft CPU and the hardware loop

The [PicoRV32 example](../../examples/ulx3s/README.md#a-soft-cpu) is a configured FPGA design, not a CPU instruction interpreter embedded in the emulator. Its [top.v](../../examples/ulx3s/picorv32_console/top.v) instantiates the core, memory and UART; the execution path follows the resulting LUTs, registers and hard blocks. [swap.sh](../../examples/ulx3s/swap.sh) uses ecpbram placeholders to replace firmware without another place-and-route run. The [HR01 record](../../docs/inventory/LFE5U-85F/hr01_picorv32_console/README.md) compares console behaviour and a firmware-reported cycle count with the physical board under its recorded loading conditions.

The [hardware-validation policy](../../docs/HARDWARE_VALIDATION.md#how-to-handle-a-disagreement-before-the-measurement) distinguishes a manual statement, tool conversion, a board observation and a repeated observation. Tests should choose stimuli that distinguish competing explanations, preserve the actual bitstream and settings, and retain raw host UART captures. This feedback can change execution semantics: [HB01](../../docs/inventory/LFE5U-85F/hb01_pdp36/README.md) resolves a wide-read ordering dispute; the [ALU study](../../docs/DSP_ALU.md) grounds arithmetic, register controls and cascade behaviour in the recorded DSP series. Neither result closes every mode of its family.

## Unknowns and current limits

Startup is an explicit assumption boundary. [compile/mod.rs](../../src/compile/mod.rs) defines `configured`, `ready-unknown` and `wake-phases`; [compile/system.rs](../../src/compile/system.rs) binds global controls. Configured startup follows the [HS01 observation](../../docs/inventory/LFE5U-85F/hs01_init_slice/README.md) for executed fabric FF initialization. Wake phases are a logical controller profile, not POR, SPI/JTAG loading or calibrated START timing. Input settling seen near wake-up remains outside that model, as the [startup limitations](getting-started.md#known-limitations) acknowledge.

The [resource matrix](../../docs/RESOURCE_COVERAGE.md) is a research and implementation ledger, with historical statements mixed into current rows. The more recent [settings audit](../../docs/SETTINGS_COVERAGE.md#silent-risks) identifies retained configuration-port settings that can affect digital behaviour but are not read. DQS/DDRDLL, DCS and several system resources remain unexecuted; analog thresholds, pad discharge, physical delay calibration and general timing closure are outside the digital contracts. The [planner](../../src/compile/mod.rs), [IO planner](../../src/compile/iologic.rs) and [DSP planner](../../src/compile/dsp.rs) remain the authority for current rejection boundaries. In particular, ALU opcode IN_1 is refused rather than assigned an invented register model. See [verification](verification.md) for what the available checks establish and what remains open.

## Repository layout

| Path | Contents |
|---|---|
| `src/model` | Immutable architecture of the whole device |
| `src/decode`, `src/config.rs` | Decoding of the configuration into resource settings |
| `src/compile`, `src/prepared.rs` | Execution plan from the configuration |
| `src/kernel` | Deterministic event kernel (0/1/X/Z, delta waves, traces) and fast mode's C generator |
| `src/fast.rs`, `src/live.rs` | Fast-mode runtime; the live run the window drives |
| `src/flow`, `src/command_flow.rs` | Project build flow: `build`, `run`, `load` |
| `src/peripherals` | HDMI receiver and OLED models |
| `src/state.rs`, `src/image.rs` | Device state and configuration image |
| `gui/` | The window: die map, inspector, 3D board, console |
| `examples/ulx3s`, `examples/gallery` | Reference designs and demos for the ULX3S, several run on the board |
| `tests/` | Fast tests and native integration tests |
| `tools/` | Research collectors and native probes (Python/C++), the 3D board pipeline |
| `docs/` | Design notes, research reports and the pinned evidence inventory |
| `site/` | The documentation site (mdBook) built from `docs/` |
| `packaging/` | Release bundles: macOS `.app`, Linux tarball |
| `licenses/` | Ledger of third-party material |
