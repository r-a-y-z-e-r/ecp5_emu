# Summary of materials for the full LFE5U-85F

The current runtime milestone is [observation of the inputs of an active ALU slice](DSP_ALU_INPUTS.md), [operand forwarding ROA/ROB](DSP_FORWARDING.md) and [MULT18 with a shared registered output](DSP_OUTPUT.md) on top of [EBR](EBR_EXECUTION.md) on top of [global controls and wake phases](SYSTEM_EXECUTION.md), [LUTRAM/RAMW](LUTRAM_EXECUTION.md) and the automatic plan from CRAM. Software examples do not close the research questions: D01–D10 keep the mode storage, registers and operations of the DSP; B01–B08 keep the hardware modes/collisions of the EBR, Y01/Y02/S01 the physical startup/PUR/START and DONE, L02/L03 the unknown DI of FF C and the hardware collisions. The [MODE/WEAMUX overlay and CLK/LSR constants](EBR_MODE_OVERLAY.md) are qualified in software; BASE_TYPE/PULLMODE and the hardware check need continuation. D05 remains open: the added DSP profile registers the signs/data under explicit assumptions about reset, CE and the divider. D01 is partly advanced by the logical product representations of the shared bank; the original overlay of CIBOUT_BYP with CLK0 is qualified for all 156 MULTs. The output CE/reset/SIGNEDP and the ALU/flags are not closed by this. There are no board results.

After this collection, the main Rust code was started: the [first DeviceModel milestone](CORE_STATUS.md), then [configuration and separate state](CONFIGURATION_STATE.md) and the [LUT/FF event kernel](EVENT_KERNEL.md). The research results do not establish verified behavior for the whole chip.

Snapshot of October 22, 2026. This pass focused on collecting materials; the main Rust code was not changed. The existing simulator remains a prototype. The [implementation plan](IMPLEMENTATION_PLAN.md) describes the next order of work and does not claim implemented functions.

The initial pass over the available materials now covers all families from the passport: fabric, memory/DSP, IO/banks, clock networks, system functions and packages. This is not a claim that all physical internals and transitions have been recovered. The public documents and Trellis still have gaps; they are listed below and in the family studies.

## What is collected where

| Family | Content of the materials | Main documents | Open IDs |
|---|---|---|---|
| Device/routing | 14 231 tiles/148 types, 54 721 site references, 3 274 setting descriptions, all mux/fixed descriptions | [Passport](DEVICE_PASSPORT.md), [bindings](RESOURCE_BINDINGS.md), [structure](STRUCTURAL_MODEL.md) | Unbound/candidate references and mode interpretation |
| PLC/LUTRAM | 10 455 PLCs; interfaces, LUT INIT/addresses, RAMW and shared modes | [PLC_LUTRAM](PLC_LUTRAM.md) | L01–L09 |
| FF/carry | Controls, INJECT/output mux, open model checks and all carry outputs | [FF_CCU2](FF_CCU2.md) | S01–S06 |
| EBR | 208 arrays, six widths, INIT/WID, CS/masks, reset/CE/OCE, synthesized environment | [Addressing](EBR_ADDRESSING.md), [control](EBR_CONTROL.md) | B01–B08 |
| DSP | 39 groups/78 slices, 156 MULT18/78 ALU54, small modes/PRADD, shared bits, cascades and ports | [Internals](DSP_INTERNALS.md), [wires](DSP_ROUTING.md) | D01–D10 |
| IOLOGIC/SIOLOGIC | 224+141 resources, 9 660 IOLOGIC fields, DDR/7:1/delay, 603 route-control bits | [Resources](IOLOGIC_RESOURCES.md), [paths](IO_ROUTING.md) | I01–I10 |
| DQS/DDRDLL | 14 DQS, four DDRDLLs, DDRDEL/controls and 224 FIFO consumers | [DQS_CONTROL](DQS_CONTROL.md) | Q01–Q08 |
| Primary/ECLK/PLL/OSC | All quadrants/segments, directed TAP/SPINE, DCC/DCS/PLL fields, 9 probes of writable settings | [CLOCK_NETWORK](CLOCK_NETWORK.md) | C01–C10 |
| PIO/banks | 365 PIO/8 banks, 4 451 fields, pairs/pulls/open-drain, shared bits and bank controllers | [PIO_BANKS](PIO_BANKS.md) | P01–P08 |
| Startup/system functions | GOE/GSR/GWDIS/DONE/START, OSC/SED/DTR, commands/CRC, loss of service data by the loader | [SYSTEM_STARTUP](SYSTEM_STARTUP.md) | Y01–Y09 |
| Packages/JTAG | 4 packages, all 1 976 balls, 947 checked GPIO, BSDL IR/IDCODE/750 cells per package | [PACKAGE_JTAG](PACKAGE_JTAG.md) | Y06–Y08, P07 |

The `decode` command preserves the first set of 14 417 composite fields. The new `load` merges five research catalogs into 28 810 fields; the limits are described in [CONFIGURATION_STATE.md](CONFIGURATION_STATE.md). The original family volumes overlap; the field counts of different reports must not be added up as unique coverage.

## What the last pass added

12 192 primary clock coordinates, 197 408 segment relations, 192 SPINEs and 18 048 TAP buffers were checked. A research analysis of 337 fields of the clock/system part was added, including the PLL/DCC/DCS fields that are missing from the working catalog. The full PIO/bank fields and the official pins/chains of the BSDL of all packages were collected separately.

The four new collectors were rerun into independent directories; **all 43 output files matched byte for byte**. The native API confirmed the clock/PIO encodings; nine placed software circuits confirmed the behavior of the clock settings writer, and a dedicated C++ probe confirmed the loss of four service values in a text round-trip. The [final check](inventory/LFE5U-85F/material_validation.json) also checks the tables, the CRAM bounds, the question IDs and the document links. There were no hardware runs. The earlier collected studies have their own reports; the scope of the rechecking after the toolchain update is described separately in [TOOLCHAIN.md](TOOLCHAIN.md) and is not declared complete for all old experiments.

The most significant new inconsistencies:

- 126 OSC.DIV designations give 63 distinct raw codes; also, DIV=128 is written by nextpnr as 127.
- The DCSMODE parameter and the DCSMODE attribute are handled differently by nextpnr; the programmed mode is not always the HDL mode that was requested.
- 328 PIO BASE_TYPE have diverging fragment defaults, but without conflicting polarities inside a value.
- Trellis Chip preserves CTRL0/CTRL1/SED/USERCODE from `.bit`, but the conversion to `.config` loses them; the current bridge does not export them either.
- The PLL guide contains 16 ms and 16 us for LOCK; the SED guide and a separate errata disagree on distributed RAM. For DTR, the documented 70 µs diagram remains a reference; HY04 now measures three ULX3S loads with initial/final 0x9E, busy restart and approximately 31–33 µs after release. The event kernel and fast mode use a profile byte, 00 during high/conversion (nonuniform fall-relative count offsets) and pooled median 32.740 µs. Fast mode host-schedules exact completion at latest fall + 32 740 000 ps, including publication before a coincident edge captures (1c79418). There is still no Celsius scale, other-temperature evidence or sub-40 ns model.

## What really remains to be learned

Currently **78 family questions** are registered: D10 + I10 + Q8 + B8 + L9 + S6 + C10 + Y9 + P8. These are large topics, not 78 separate measurements and not a percentage of unfinished reading. The summary [machine registry](inventory/LFE5U-85F/research_manifest.json) links them to the documents. The questions share dependencies; closing one may clarify several families.

| Kind of remainder | Examples | How to continue |
|---|---|---|
| Disputed digital behavior | PDP36, startup/GSR, DSP shared registers, FIFO/ALIGNWD, clock switching, phase transitions | Distinguishing experiments on the available board |
| Analysis of structure/modes during implementation | Conditions of the fixed/mux bypasses, missing segments of dedicated networks, shared fields/storages | Matching the already saved bit patterns, routes and controls; targeted new probes if needed |
| Not enough public information | SED CRC32 scan/seed, closed factory tests, part of the internal DSP/IO stages | An additional primary source or a separate hardware experiment; the unknown is kept |
| Instance/environment parameters | OSC frequency, LOCK/calibration, temperature/DTR, bank supply and marking | An explicit environment profile and a measurement, without turning the emulator into an analog model |
| Software integration | Full decoder, event kernel, metadata preservation, transport/controller, GUI | Implementation per the [plan](IMPLEMENTATION_PLAN.md); this is no longer documentation collection |

120 candidate bindings of source references (112 IO and 8 ECLKSYNC) and 1 496 unbound service records are preserved. Of the latter, 1 463 are CIBTEST, 12 CLKTEST, 8 BREFTEST, 1 PVTTEST; another 4 START and 4 each of BCINRD/BCLVDSO now have a documented purpose, but need binding. The numbers refer to source references, not to the number of independent physical blocks.

**37 descriptions of future experiment series** are prepared: HB12, HL6, HS5, HC5, HY5, HP4. For DSP/IO/DQS, the queue still needs to be split into concrete benches. No series is considered executed; the boards/connections/firmware of the benches are not fixed yet. The absence of Diamond does not block this path; unconfirmed models are not declared a reference.

## Why material validation currently fails

`tools/validate_materials.py` checks the **2026-10-22 material pass** against the repeated collection from that date in `target/research/clock_system/repeated-*`. Since then, the inventory's `clock_system_research/clock_system_contract.json` has gained the HY04/DTR Y05 findings and execution rows. Its collector source, `tools/data/clock_system_contract.json`, now matches that published copy byte for byte; the saved repeated copy still predates those additions.

`research_manifest.json` still pins the 2026-10-22 hashes of these documents edited since: `docs/CLOCK_NETWORK.md`, `docs/EBR_ADDRESSING.md`, `docs/FF_CCU2.md`, `docs/HARDWARE_VALIDATION.md`, `docs/IMPLEMENTATION_PLAN.md`, `docs/PIO_BANKS.md`, `docs/RESEARCH_STATUS.md`, `docs/RESOURCE_COVERAGE.md` and `docs/SYSTEM_STARTUP.md`. The source-contract synchronization also leaves its historical manifest and evidence-report hashes stale.

A preview of a fresh clock collection adds **88 PLL ports** to `clock_system_research/ports.csv` (22 per PLL), changing `evidence_report.json`'s port count from **384 to 472**. Making validation pass again therefore requires a full re-collection, including repeated outputs and updated manifests/reports. That is a separate task; this synchronization preserves the collected evidence and measured values.

## How to reproduce and read the results

The source publications and hashes are pinned in the family reports `docs/inventory/LFE5U-85F/*_research`. The last four collectors:

```sh
python3 tools/collect_clock_system.py
python3 tools/collect_pio_banks.py
python3 tools/collect_package_system.py
python3 tools/collect_system_metadata.py
python3 tools/summarize_research.py
```

`python3 tools/validate_materials.py` checks the saved repeated collection in `target/research/clock_system/repeated-*` and updates the final report. To repeat the comparison after target has been deleted, first run the same four collectors with `--output` into those directories; for clock, additionally set `--probe-directory target/research/clock_system/repeated-probes`.

The clock probes need the installed toolchain, and the metadata collector needs the research `target/trellis_metadata_probe`; its build is described in [TOOLCHAIN.md](TOOLCHAIN.md). The official PDF/CSV/BSDL are cached in `target/research`; the published reports keep the URL/revision/hashes. The cache in target is not a redistributable model library.

The initial collection of the families is complete to the stated extent; the full physical reverse engineering is not complete. The next implementation step and the measurement queue now rely on concrete information and the listed limits, not on a promise that Trellis already contains the behavior of the whole chip.
