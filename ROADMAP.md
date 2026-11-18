# Roadmap

Agreed on 2026-10-29. This file sets the **order of work**. Where it disagrees with the ordering in
[PLAN.md](PLAN.md) or [docs/IMPLEMENTATION_PLAN.md](docs/IMPLEMENTATION_PLAN.md), this file wins.
Those documents keep their design content and are reconciled with this one during Stage 0.

## Goal

An open-source **educational tool for FPGA students**, roughly "Quartus with a built-in emulator"
for the Lattice ECP5 and the ULX3S board:

1. an accurate digital model of the LFE5U-85F, executed from the real bitstream;
2. a GUI for looking inside the chip: tiles, slices, LUTs, flip-flops, routing, memories;
3. interactive input/output elements: buttons, LEDs, displays, a console;
4. a virtual ULX3S board;
5. soft processors running on the emulated chip, with a console and a firmware workflow in the
   spirit of Nios II and the Nios II EDS.

The unique value is the combination of three properties. The **same `.bit`** runs in the emulator
and on a real ULX3S. The student sees what synthesis and place-and-route actually built in the
fabric. And they see it under their **own signal names**.

## Principles

- **Student designs first.** A feature is prioritised by how often designs built with
  Yosys/nextpnr for a ULX3S use it. LUT/FF/carry, LUTRAM, EBR, MULT18, PLL/OSC, PIO, IOLOGIC DDR
  and UART-style IO come before ALU54B, DQS, SED or DTR.
- **No invented behaviour.** Unknown behaviour stays explicit: unsupported with a clear message,
  `X`, or a named assumption profile. It is never a convenient constant. A resource family is
  *good enough for now* when all four of these hold:
  - the modes that student designs use execute;
  - every other mode is rejected with a message;
  - every assumption is named and listed;
  - once the board loop exists, at least one design using the family has matched the real board.
- **Two executors, one plan.** The exact event kernel serves stepping, traces and teaching. A fast
  compiled mode serves soft CPUs and long runs. Both execute the same plan, and they are
  cross-checked on identical stimuli.
- **Names are an overlay.** Execution comes from the bitstream. HDL names come from the nextpnr
  `--write` JSON, stored as a separate layer and never used to decide behaviour.
- **No home-grown IDE.** Synthesis, place-and-route and programming stay with Yosys, nextpnr and
  openFPGALoader. We build a thin shell around them, and the code editor is an external one.
- **English only** in code, comments, docs, commits and PRs.

## Stage 0 — Foundation

- Translate PLAN.md and docs/*.md to English.
- Keep one status document plus a README quickstart ("how do I run my `.bit`"). Fix the stage
  statuses in PLAN.md, which are out of date.
- Add VCD output to `emulate`, so the legacy `run` prototype can be retired without losing
  features.
- Remove allocations from the event-kernel hot path. Baseline measured on 2026-10-29: the 8-bit
  counter fixture runs at about 22k cycles/s, with about 80% of profile samples in malloc/free.
- Work through branches and PRs, with CI running the fast tests, clippy and fmt. CI pins the
  Trellis database to commit `9754ff47`.

**Exit:** CI is green on `main`, all docs are in English, a newcomer can run a `.bit` from the
README, and the kernel speed-up is measured against the baseline.

**Status 2026-10-29:** the docs were translated to English, and the kernel speed-up is measured
([EVENT_KERNEL.md](docs/EVENT_KERNEL.md#performance)). One item is still open: the single status
document (PLAN.md stage statuses are out of date).

## Stage 1 — A student's ULX3S design runs

- A board-like startup profile: flip-flops are initialised as after configuration, without a
  hand-written `sys:step` stimulus.
- OSC and PLL (EHXPLLL) with named behaviour profiles, for example lock time. The open questions
  C06, C07 and Y03 stay open until they are measured.
- DCC and the primary clock paths that PLL outputs drive.
- IOLOGIC: registered IO and DDR X1 (IDDRX1F/ODDRX1F), enough for HDMI-style outputs.
- The remaining PIO modes that typical designs use: pull-ups, bidirectional and open-drain.
- A name overlay from the nextpnr `--write` JSON, so probes can be given by HDL net name.

**Exit:** a set of reference designs builds with Yosys/nextpnr and runs in `emulate`, with probes
given by HDL name. The set covers blinky via PLL, UART TX, a debounced button and a DDR output
pattern.

**Status 2026-10-29: exit reached.** The four designs are in [examples/ulx3s](examples/ulx3s/README.md),
and `tests/reference_designs.rs` checks them by HDL name. Each item has a board measurement except
`LVCMOS33D`, which needs wires on the header:
- startup: HS01;
- PLL: HC05;
- IOLOGIC on the left and top banks: HI02;
- OSC: HC06;
- the UART design itself, run on the board.

## Stage 2 — Virtual ULX3S, headless

- A board model with LEDs, buttons, a UART connected to a terminal console, OLED over SPI and an
  SD card in SPI mode. SDRAM comes later in this stage.
- A board description that maps package balls to peripherals. The ULX3S revision must first be
  confirmed against the physical board.
- The hardware loop begins: the same reference designs are flashed with openFPGALoader, and the
  LED and UART traces are compared with the emulator's.

**Exit:** the reference designs behave the same on the virtual and the real ULX3S in the recorded
scenarios.

## Stage 3 — Fast mode and soft CPUs

- A compiled executor generated from the execution plan: Rust/C++ code, or Verilog for Verilator.
  The choice is made by measurement.
- A cross-check against the event kernel on identical stimuli.
- A soft CPU (PicoRV32, or VexRiscv/LiteX) with a UART console. Firmware is swapped with
  `ecpbram`, without re-running place-and-route.
- Target: at least about 1M emulated cycles/s for a small SoC. This figure still has to be
  validated.

**Exit:** a firmware prints to the console interactively, and it can be rebuilt and swapped in.

**Started early (2026-10-29):** fast mode (`emulate --fast`) compiles the plan to C, cycle for cycle
equal to the event kernel. It covers LUT, carry, FF, PIO, IOLOGIC, EBR, PLL and OSC. All Stage 1
reference designs run in it: `uart_hello` at 1.2–1.5× and `ddr_pattern` at 4–5× the board's speed,
`blinky_pll` at 0.6× (its counter runs at 100 MHz), `uart_ram` (two EBR) at 0.53×. The cost grows
with the logic per clock, so soft CPUs need the next step: skipping unchanged logic
([FAST_MODE.md](docs/FAST_MODE.md)).

**Soft CPU (2026-10-30):** `examples/ulx3s/picorv32_console` is the stock PicoRV32 (RV32I) with
8 KB of EBR, its register file in LUTRAM, a UART and a C firmware built with clang. Fast mode now
covers LUTRAM and ran it at 1.3 MHz (0.05×), cycle for cycle equal to the event kernel and with
the console text of an RTL simulation. Above the 1M-cycle target, far from the board's 25 MHz.
Skipping blocks of unchanged logic was measured and is slower than evaluating everything. Gates
that only registers with an enable read are now evaluated on demand: 1.9 MHz on the CPU, and HC06
twice as fast.

**Speed reference (2026-11-08).** Word operations were measured as a lever. 40 to 67% of the
CPU's gates are bit-sliced, which is worth 1.6 to 2.4 times.

Verilator 5.054 runs the same SoC as follows:
- from its RTL, 12–16 MHz;
- from the Yosys ECP5 netlist, 1.2 MHz. Fast mode does 1.9 MHz from the bitstream.

So 25 MHz for PicoRV32 is above even an RTL simulator. The figures are in
[FAST_MODE.md](docs/FAST_MODE.md).

**Console and firmware swap (2026-11-08):** `emulate --fast --console-out … --console-in …` puts a
UART terminal on the design. The firmware's text goes to stdout, and the keys go to its receiver.

`examples/ulx3s/swap.sh` puts a rebuilt firmware into the placed-and-routed design in about a
second, with ecpbram over random placeholders. The swapped bitstream is identical to a full
build, and fast mode reuses its compiled program.

**Exit reached (2026-11-08).** On the board (HR01), a swapped-in firmware printed the same text as
the emulator, `151485 cycles` included, in 3 loads out of 3; the console echoes as in the emulator
([HR01](docs/inventory/LFE5U-85F/hr01_picorv32_console/README.md)).

## Stage 4 — GUI

- The virtual board panel comes first: LEDs, buttons, the console, displays.
- Then the chip viewer: tiles, slices, LUT/FF, routing and EBR, with live state, signal history and
  names from the overlay.
- Animation and stepping never change emulated time.

## Stage 5 — Project shell

- A project file and a one-command flow: build (Yosys → nextpnr → ecppack), emulate, flash.
- The editor stays external. A VS Code integration may come later.

## Parallel track — hardware validation

The existing queue in [docs/HARDWARE_VALIDATION.md](docs/HARDWARE_VALIDATION.md) holds 37 series
for 78 open questions and is reordered by the student-designs-first filter. The first need is a
reusable observation bench: gateware that drives stimuli into a design under test and streams
samples back over UART. It lets each series be a configuration of the bench instead of a
one-off build. Seven board observations are recorded so far (HS01, HC05, HC06, HI02, HI02c, HR01); see
the Results table in HARDWARE_VALIDATION.md.

## Deferred

These stay explicitly unsupported until the goal needs them:

- ALU54B operations;
- DQS and DDRDLL;
- SED and DTR;
- factory test resources;
- SERDES (the LFE5U has none);
- real-time HDMI, which is not feasible at bitstream level (slow frame rendering only);
- the ULX3S ESP32.

## Workflow

- Work happens directly on `main`, without a branch or PR per change. The accumulated diff
  is reviewed, and PRs are opened as needed. CI checks every push and PR.
- Definition of done for a change:
  - tests for the change and the affected native tests;
  - clippy and fmt;
  - updated docs;
  - evidence in `docs/inventory` where relevant.
- Kernel performance is tracked against the counter baseline.
