# Global controls and explicit wake-up phases

After this milestone, [EBR port execution](EBR_EXECUTION.md) was added, with the same GSR/GWDIS and startup profiles. The checks and the boundaries of the system milestone itself are kept below.

Continuation of [LUTRAM/RAMW](LUTRAM_EXECUTION.md), October 22, 2026. The main kernel now connects asynchronous and HY01 synchronized user GSR, the startup GSR, GOE and GWDIS to the executable FFs, PIO and distributed memory. The system state is kept in `DeviceState::system()` and in the JSON summary.

Added the profile `--startup wake-phases`: an already loaded image passes through four explicit logical phases T0…T3. It does not execute POR, SRAM clearing/loading, the SPI/JTAG transport or the physical START clock. The exact correspondence of the phases to the START/MCLK edges is not established and is not derived from the manufacturer's figure. PUR remains separately marked `unmodelled`.

## Binding to the configuration

[compile/system.rs](../src/compile/system.rs) reads the composite GSR.GSRMODE/SYNCMODE from the existing catalog. ACTIVE_LOW inverts the GSR input, ACTIVE_HIGH uses direct polarity. Inside the kernel, an asserted reset is always 1. NONE does not connect a user source. ASYNC and NONE in SYNCMODE have the same empty pattern with no synchronizer enabled; both source labels are kept. HY01 adds SYNC for ACTIVE_LOW with a routed CLK: user GSR is raw active OR s2, where two plain rising-CLK stages start at 11 without resets or asynchronous preset. The reset acts asynchronously on enabled resources. SYNC with unmeasured polarity, an ambiguous mode or missing CLK routing remains refused. See [the rule, startup counters and 20 ns timing hazards](SYSTEM_STARTUP.md#synchronous-user-gsr-hy01).

The GSR input, and CLK for SYNC, become independent roots of the plan. Its real routing and its GPIO are built even when the resource is not in the usual data chain of the probe. A disabled source does not turn the startup GSR into the user GSR: their values and origin remain separate.

The wake-phases profile checks the CRAM of two EFB tiles:

- SYSCONFIG.WAKE_UP, DONEPHASE and both DONE_EX must define consistent standard pairs: 4 / T0 / ON or 21 / T3 / OFF.
- GOEPHASE must be T1, GSRPHASE and GWEPHASE must be T2. Other combinations are rejected for now.
- The shared bits of WAKE_UP and DONEPHASE are interpreted together. The native check of sequence 4 writes both labels explicitly, so that a later default of the text parser does not overwrite the chosen bits.

All the raw fields read go into `report.fields`, the source GSR fields into `report.resources`, the accepted configuration and the boundaries into `report.system`. The `.sysconfig` values from the source text do not replace CRAM.

## FF, GPIO and RAM

[kernel/system.rs](../src/kernel/system.rs) forms separate global strobes. For FFs with SLICE.GSR=ENABLED the plan connects the effective reset `startup_gsr OR user_gsr`; the reset value is taken from the actual REGSET. The reset is asynchronous, with priority over the local reset and CE. With GSR=DISABLED this connection is absent; the FF keeps its clock/CE/local reset and starts at X until the corresponding event. Such an FF gets no invented PUR value. PRLD still needs a separate model and is rejected.

In `ready-unknown`, startup_gsr=0, GOE=1, GWDIS=0; the FF state is not initialized by REGSET alone. A connected user GSR still acts. In `wake-phases`, the startup GSR really holds the corresponding FFs until T2; the known state results from this executable event.

GOE controls the PIO's own strong output driver. With GOE=0 it outputs Z regardless of the user tristate/data. The external driver, the input receiver and the user pulls keep working. This is a model of the window after the image is loaded; special pull-downs and ownership of the configuration pins before loading are not implemented.

GWDIS suppresses LUTRAM writes on top of the ordinary WREMUX. It does not clear the array. The user GSR resets only the corresponding FFs and does not itself change GOE/GWDIS, so RAM writes may continue during the user GSR. `Builder::lut_ram_with_inhibit` also supports an unknown inhibit: a possible write is merged with hold. The shared LUT/INIT cells and the source image are not duplicated.

An unknown GSR merges the reset and ordinary-behavior variants. If the values agree, the result stays known; otherwise it is X. Z on the reset input is treated as an unknown. The policy for absent local controls does not change: assumed pull-downs are not added.

## USRMCLK and the dedicated CCLK pad

`pin:U3` probes the ULX3S CABGA381 CCLK ball. It drives **non-inverted USRMCLKI**
while USRMCLKTS=0 and GOE=1, and releases to Z otherwise. The planner reaches this
output-only resource from the package reference and follows its PADDO/PADDT inputs
(`R95C0:JPADDO_CCLK` / `R95C0:JPADDT_CCLK`). It uses the same external driver and
tristate muxes as a PIO output, so event and fast execution need no separate kernel
primitive. External drivers and board wires work through the ordinary pad interface.
With CCLK.MODE=NONE, the reference resolves to an undriven Z pad, listed as
`R95C0:PAD_CCLK` in `report.undriven`.

The model adds **no CCLK pull**: there is no dedicated PULLMODE setting bound here,
and the board's pull under USRMCLKTS=1 has not been measured. Thus Z assumes no
external drive or board pull. HR07 reads the ULX3S flash
through USRMCLK and the emulator reproduces it, establishing the non-inverted data
path. The pull under tristate, CCLK before the first user edge after configuration,
and pad timing remain unmeasured. This is an already configured user-mode model;
configuration-clock transport and ownership handoff are not executed.

Balls come from the model's
[official pinout/BSDL table](inventory/LFE5U-85F/package_system_research/physical_pins.csv),
described in [PACKAGE_JTAG.md](PACKAGE_JTAG.md):

| Package | CCLK reference |
|---|---|
| CABGA381 | `pin:U3` |
| CABGA554 | `pin:AE3` |
| CABGA756 | `pin:AM3` |
| CSFBGA285 | `pin:U16` |

CCLK ownership is decoded from **EFB3_PICB1 CCLK.MODE** (bit 720,1135).
EFB1_PICB1's apparent CCLK.MODE bit 508,1135 is shared with OSC.DIV: nextpnr writes
`_NONE_` for that enum when writing OSCG and writes USRMCLK only in EFB3_PICB1
(`ecp5/bitstream.cc`). It is not a second enable replica. The raw composed
CCLK.MODE remains in the report, potentially `unmatched` when these bits differ;
`report.fields` records the authoritative EFB3 field. Execution uses observed CRAM
without changing the image. The native tests exercise
OSCG dividers 16 and 128 with USRMCLK, and divider 16 without it, covering both
values of the shared bit and both directions of the apparent replica conflict.

`compile::cclk_pad(&device, package)?` publicly returns `CclkPad { ball, direction }`.
The direction is `Some(model::Direction::Output)` when USRMCLK owns the driver and
`None` otherwise; dynamic TS/GOE do not turn an output into an unused ball.
Board helper integration requires two edits in `src/board.rs`:

1. At the start of `direction()`, before the PIO lookup, add:

   ```rust
   let cclk = crate::compile::cclk_pad(device, PACKAGE)?;
   if ball == cclk.ball {
       return Ok(cclk.direction.map(|_| Direction::Output));
   }
   ```

2. At the start of `used_balls()`, obtain the same query and replace its PIO-only
   package filter with this one, keeping its existing call to `direction()`:

   ```rust
   let cclk = crate::compile::cclk_pad(device, PACKAGE)?;
   // In the existing package_pins() iterator:
   .filter(|p| {
       p.package == PACKAGE
           && (p.pio.is_some() || (p.ball == cclk.ball && cclk.direction.is_some()))
   })
   ```

Run the native configurations and compare every reference-clock and OSCG edge:

```sh
cargo test --release --test usrmclk -- --include-ignored --nocapture
```

## Persistent configuration ports

`SYSCONFIG.MASTER_SPI_PORT`, `SLAVE_SPI_PORT` and `SLAVE_PARALLEL_PORT` are
checked in every startup profile. DISABLE keeps normal user IO. ENABLE reserves
the package's configuration pins: reaching one as a user input, output or tristate
pad fails with the field, ball and function in the error. This also covers pad
references reached through routing fan-in, external drives and board wires.
An enabled port whose pins are unused is accepted and recorded as having no effect
on the plan in `report.system.configuration_ports`. Configuration commands and
transport remain unmodelled; a retained configuration pad cannot be probed as an
ordinary user pad.

The pin sets use the model's `PackagePin.dual_function` aliases, matched as whole
slash-separated names, and its dedicated CCLK pin. No package ball is hardcoded.
The primary source is the online [ECP5 sysCONFIG User Guide,
FPGA-TN-02039-2.6](https://www.latticesemi.com/view_document?document_id=50462)
(checked 2026-11-17), sections 4.7 and 6.1–6.3, tables 4.8/6.2/6.3 and section 7.
The repository previously linked revision 2.5; the PDF itself is not vendored.

| Field | Reserved functions | CABGA381 balls |
|---|---|---|
| MASTER_SPI_PORT | CSSPIN/MCSN; MOSI/D0, MISO/D1; quad IO2/D2, IO3/D3; CCLK; conservative MOSI2/MISO2 aliases | R2, W2, V2, Y2, W1, U3; V1, U1 |
| SLAVE_SPI_PORT | SN; MOSI/SI, MISO/SO; CCLK | T2, W2, V2, U3 |
| SLAVE_PARALLEL_PORT | D0..D7; CSN, CS1N, WRITEN, BUSY; conservative DOUT/CSON; CCLK | W2, V2, Y2, W1, V1, U1, T1, R1, T2, U2, T3, R2, R3, U3 |

Master SPI reserves quad pins even without a known transfer width. MOSI2/MISO2
and parallel DOUT/CSON are conservative additions: their precise post-wake
qualification is not modelled, so they fail instead of silently behaving as GPIO.
HOLDN is explicitly configuration-only in the guide and is not reserved by the
slave SPI persistence setting. These are software ownership checks; no board
experiment was performed, and the flash was neither commanded nor modified.

nextpnr writes the options as tile enums in EFB1 (master SPI), EFB0 (parallel),
and EFB0/EFB2 (slave SPI). The planner reads their actual CRAM patterns, including
the qualified empty DISABLE pattern, and rejects contradictory replicas.
A manually added `.sysconfig NAME ENABLE` is retained as metadata but is not
applied by Trellis `to_chip`; the planner also checks this explicit declaration.
Either source enabling a port reserves its pins, so a text DISABLE cannot hide
enabled CRAM. Unknown directive values fail. The raw fields and the text directive
remain separate evidence in the report.

USRMCLK with any enabled port is refused because all three claim CCLK. This is
checked even when observed scope never reaches CCLK. A reached GPIO conflict is
reported first, giving HR07's MOSI/MISO/CS a useful ball-specific error. The guide's
USRMCLK section rules out background SPI programming; no ownership priority is
invented between that primitive and a retained configuration controller.

`tests/configuration_ports.rs` checks all three ports and PIO directions, CRAM and
text settings, unused pins, unchanged DISABLE behaviour, metadata aliases and
alternate ways to reach a pad. Its ignored native test adds MASTER_SPI_PORT=ENABLE
to HR07's flash dump configuration and refuses MOSI, MISO, CS and an unobserved
USRMCLK conflict. All checks run in software:

```sh
cargo test --release --test configuration_ports -- --include-ignored
```

## Phase order and DONE

The profile starts before T0: GOE=0, startup_gsr=1, GWDIS=1. Each defined 0→1 transition on the virtual `sys:step` passes the next phase:

| Phase | Sequence 4 | Sequence 21 |
|---|---|---|
| T0 | Release DONE | No change to the strobes |
| T1 | Enable the GOE outputs | Enable the GOE outputs |
| T2 | Deassert startup GSR and GWDIS | Deassert startup GSR and GWDIS |
| T3 | Completion with DONE released | Set DONE and complete |

For sequence 4, `sys:done` is a virtual open-drain bus: the controller driver 0/Z, an external driver and a pull to 1 chosen by the environment. This is a model interface, not a binding to the physical DONE ball and not a statement about an internal pull on the die. An external 0 holds the completion, Z releases the bus; a conflict/unknown is kept as X.

The profile lets T0…T2 pass while the external DONE is low and holds the completion at T3. This is an explicitly chosen policy until HY01: the documentation describes a delay of the completion, but the exact stall of the early phases is not established. After a reliable entry into user mode, a new external pull-down of DONE does not restart wake-up. An undetermined possibility of earlier completion stays X until a defined release.

An initial high `sys:step` does not create a phase. X/Z on this input leads to a diagnostic: the model does not guess the number of phases passed. After T3, additional defined pulses do not change the phase. There is no automatic reset/reconfigure of this controller yet.

At a single timestamp the kernel first settles the fabric and the system strobes, then the FFs and the RAM ports capture a common snapshot. Therefore a coincident T2 and write clock already see GWDIS deasserted. This is a digital policy of the profile; the hardware setup/hold, the internal delays and the physical coincidence of START with the fabric clock remain open.

## Interface and running

The references `sys:goe`, `sys:gwdis`, `sys:startup_gsr`, `sys:user_gsr`, `sys:gsr`, `sys:ready`, `sys:done` are available. They show the logical strobes; 1 for gsr/gwdis means an active reset/inhibit. `sys:step` is available only in wake-phases. Driving is allowed for step and, only for sequence 4, the external done. The other system references are available for observation. PUR and the physical START are not accepted as supported control inputs.

The virtual `$SYS.*` exist in the runtime separately from the immutable architecture; they do not add discovered physical wires or BELs. The `sys:` interface is used for them, and `report.system` explains the profile. Their values and transitions go into the ordinary bounded trace when the corresponding probes are selected.

```sh
# Produce the images and check both sequences:
cargo test --release --locked --test system_native -- --ignored

# Repeat the CLI example of sequence 21:
cargo run --release --locked -- emulate target/system-controls/top.bit \
  --startup wake-phases --scope observed --clock sys:step --cycles 4 \
  --stimulus target/system-controls/cli-top-events.json \
  --probe sys:goe --probe sys:gwdis --probe sys:gsr --probe sys:ready \
  --output target/system-controls/user-run.json
```

Here `--clock sys:step` sets the logical phase boundaries at 1, 3, 5, 7 ns. These are times chosen for the test bench, not a measured duration of wake-up. The same event file is used for the ordinary fabric clocks and reset. `ready-unknown` is kept as the profile of an already started user mode.

## The `configured` profile (added 2026-10-29)

`--startup configured` (`Startup::Configured`) starts in user mode, like `ready-unknown`:
GOE released, GWDIS and startup GSR inactive, no wake phases. In addition, **every executed PLC
flip-flop starts at its REGSET value, whether SLICE GSR is enabled or disabled.**

Evidence: hardware series HS01 on the ULX3S (LFE5U-85, JTAG SRAM loads), recorded in
[hs01_init_slice](inventory/LFE5U-85F/hs01_init_slice/README.md). Flip-flops with no clock edge
after configuration equalled REGSET in 4 loads out of 4, in both SLICE GSR groups. This closes the
init-value part of S01 for this loading path. The open parts stay open: PUR, power-up from flash,
SYNC GSR, PRLD, and the inputs that settle after wake-up (HS01 run A).

Scope of the profile:
- Only flip-flops the plan executes are initialised; unexecuted storage is not touched.
- EBR, DSP and LUTRAM state is unchanged by this profile.
- `report.system.flip_flop_initial` states the rule that applies.
- `ready-unknown` and `wake-phases` keep their previous meaning. GSR-disabled flip-flops stay `x`
  there until a reset or capture defines them.

A related planner rule applies to every profile. When a PLC local-reset selector `MUXLSRn` has no
routing arc, and both `LSR0.LSRMUX` and `LSR1.LSRMUX` are `LSR`, the flip-flop's local reset is
constant 0. Before this rule, the selector counted as an undriven wire, so an `x` synchronous reset
corrupted every capture.
- nextpnr relies on this: it drops a constant-0 LSR net with `LSRMUX=LSR` and leaves the selector
  unrouted.
- All HS01 flip-flops had exactly this encoding and toggled normally on the board.
- Each use is listed in `report.constants` with that rule.

Checks:
- `tests/system_controls.rs`: `configured_profile_starts_flip_flops_at_regset_whatever_slice_gsr_says`.
- `tests/configured_startup_native.rs` (`--ignored`): the HS01 run B design reproduces the board's
  LED pattern and its toggling for `.config` and `.bit`, and a plain ULX3S blinky without reset
  counts 300 edges from 0 in observed and all-plc scope.

## Checks

[Seven portable tests](../tests/system_controls.rs) check both sequences, GSR enable/disable, polarity, reset priority, RAM preservation, independent writes during the user GSR, GWDIS including X, pad input/conflict with GOE=0/1, an undefined DONE and latching of the completion, the absence of an initial phase, rejections for SYNC/non-standard phases/inconsistent fragments, and writes to unavailable system references.

The [native test](../tests/system_native.rs) places three FFs with RESET/SET and different GSR, a 16×4 LUTRAM, 23 GPIOs and a real GSR input. Six runs of **1025 samples** each were checked:

| Image | Profile and scope | Executable FFs / RAM columns / paths |
|---|---|---:|
| 21, `.config` and `.bit` | wake-phases, observed | 3 / 4 / 394 |
| 4, `.config` and `.bit` | wake-phases, observed | 3 / 4 / 394 |
| 21, `.bit` | wake-phases, all-plc | 83 640 / 4 / 711 266 |
| ACTIVE_HIGH, `.bit` | ready-unknown, observed | 3 / 4 / 394 |

The observations are identical for the matching `.config`/`.bit` and for observed/all-plc for sequence 21. In each wake run, one suppressed write before T2 and 204 allowed writes after release are confirmed, including a write during the user GSR; the separate expected FFs and the memory array do not read the kernel's internal computations. In the full PLC scope, 83 638 FFs have GSR=ENABLED and two have DISABLED; this includes the unused half of a slice.

The CLI separately runs 9/10 samples for sequences 21/4: it shows Z before GOE, a suppressed and an allowed write, the wait for DONE, and completion. The [native results](inventory/LFE5U-85F/system_execution_native.json), [CLI 21](inventory/LFE5U-85F/system_execution_cli21.json), [CLI 4](inventory/LFE5U-85F/system_execution_cli4.json), [HDL](inventory/LFE5U-85F/system_execution_fixture.v), [package constraints](inventory/LFE5U-85F/system_execution_fixture.lpf) and [validation summary](inventory/LFE5U-85F/system_execution_validation.json) are saved separately.

In total **81 checks** passed: 73 ordinary tests and eight explicitly run integration tests. The following passed again: the two carry counters and the CLI, both LUTRAMs with FFs, the native basic kernel, the snapshot of CRAM/INIT/service data, the full architecture with 197 408 clock relations, the EBR/DSP decode and the old counter with 65 538 observations. Clippy passed with no warnings, and formatting passed. The external blink with ECP5_TEST_BIT was not run separately.

The physical board was not programmed. S01/Y01/Y02 and HY01 remain open: PUR, the synchronized GSR, START, the physical wait for DONE and the initial states of the other families are not proven by this. Next come the EBR ports, included in the same system controls; transport/POR, PRLD, PIO overlay and the other families are kept in the [plan](IMPLEMENTATION_PLAN.md).

## DTR die-temperature profile (HY04)

The planner dispatches `DTR.MODE` from CRAM to `kernel/dtr.rs`. Active DTR
drives all eight native DTROUT wires with `compile::Board::dtr_code` before
STARTPULSE; `None` uses the measured ULX3S room-temperature byte 0x9E. This
profile lives beside `oscillator_hz` in `src/compile/mod.rs`; `src/board.rs`
is unchanged. `DTR.MODE=NONE` drives inactive X.

Assertion clears all bits to 00 and cancels any old completion. A definite
high-to-low transition schedules the configured byte 32.740 us later (HY04
pooled isolated median, nominal synchronizer correction). High and conversion
hold 00; bit 7 stays clear until completion. Retrigger cancels/restarts rather
than queues. X/Z controls cancel pending completion and hold 00 until a known
high then low re-establishes a conversion. Initial low holds the profile byte
without an invented edge; initial high holds 00 until its fall starts conversion.
These uncertain-control/startup cases are deterministic emulator policies.
The timer generation prevents cancelled work from publishing stale valid data.

[HY04](inventory/LFE5U-85F/hy04_dtr/README.md#reading) has regular 24/25-tick
increments only after a width/phase-dependent startup delay: the first 01 is
27..101 observer ticks after fall (pooled median 88). Thus the steps are not
uniform from fall within one or two ticks, selecting the requested zero fallback
rather than floor((t-fall)/step). All 173 one-tick intermediate codes occur
immediately before the next persistent count and mix only old/new bits,
consistent with bus skew. The fallback intentionally omits low-bit progress;
it matches board done polling, with a fixed median completion policy rather
than width/phase-dependent timing. There is no Celsius scale or evidence at
other temperatures; details below 40 ns remain open. The profile participates
in plan-cache identity; DTR kernels still bypass the plain-kernel plan/snapshot
cache. This is separate from fast-mode run snapshots.

Since `1c79418`, fast mode executes this same profile. The generated C program
pauses with return code 2 when its captured STARTPULSE changes; the host clears
DTROUT to 00, cancels any old deadline, and on a definite high-to-low transition
schedules completion at exactly **fall + 32 740 000 ps**. It then resumes the
program. Completion is published in the same input batch as a coincident clock
edge, before captures, so a register on that edge sees the profile byte.
DTR designs use the host-aware schedule instead of the clock-only fast path;
designs without DTR keep the same generated program. Fast run snapshots retain
STARTPULSE history and the pending completion. The event kernel's X/Z-control
policy above remains available; fast mode still requires binary controls.
