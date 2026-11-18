# EBR control and checks of disputed behavior

This analysis continues [EBR addressing](EBR_ADDRESSING.md). For CE, OCE, CS, reset, and the nine-bit masks, the documentation rules, the synthesis contract, and the results of the connection checks are kept separate. The exact priority of the control signals on a physical ECP5 has been in the board queue since 22 October 2026. [EBR execution](EBR_EXECUTION.md) was added later: unconfirmed combinations have an explicit conservative policy or are rejected; hardware questions are not closed by this.

## What the sources confirm

| Question | Known rule | Limit of knowledge |
|---|---|---|
| CE | In the Yosys contract, `clken` enables reads and writes | The combination with native reset/OCE/CS must be checked on the board |
| OCE | The guide ties the choice of output clock enable to the additional output register | Skipping of the pending result when CE=0 or CS does not match, and the action of OCE in NOREG, are not established |
| CS | Three-bit comparison; the inversions of the source CS are already included by the writer in CSDECODE | The effect of CS on the pending OUTREG result and on reset is not established |
| Synchronous reset | Yosys declares a zero result with priority over clock/read enable | This is a synthesis assumption; the hardware transition table is not yet measured |
| Reset together with a write | Yosys uses the native synchronous reset when reset and write are proven to be incompatible | Signals that coincide at the inputs of the EBR itself are not given an invented priority |
| Asynchronous reset | Yosys declares an asynchronous reset of the read result | The declaration `rdarst zero` alone is not enough to conclude that writes are inhibited |
| Release | The guide distinguishes SYNC/ASYNC release of the asynchronous reset | The number of edges, the role of CE/OCE/CS, and the internal stages are not specified |
| Memory array | Reset does not clear the contents of the array | The initial state of the output, the local reset, GSR, and the configuration startup are checked separately |

The semantics of `clken`, `rdsrst`, and `rdarst` were read in the [memory_libmap description of the same Yosys revision](https://raw.githubusercontent.com/YosysHQ/yosys/7f8fdfd8d7bc08c749a2a969388d3425d4f369d5/passes/memory/memlib.md) used by the installed version. The settings of the specific ECP5 are in the local `lattice/brams_16kd.txt`. The [handle_rd_rst implementation](https://raw.githubusercontent.com/YosysHQ/yosys/7f8fdfd8d7bc08c749a2a969388d3425d4f369d5/passes/memory/memory_libmap.cc) checks the mutual exclusion of write and reset; if there is none, Yosys emulates the output reset with external logic. The name `block_wr` alone does not establish who wins on the hardware.

The OCE binding and the choice of reset release are given in the [Memory User Guide FPGA-TN-02204-1.6](https://www.latticesemi.com/view_document?document_id=50466), pp. 20 and 28; the tables were checked against the page images. [Lattice FAQ 3626](https://www.latticesemi.com/en/Support/AnswerDatabase/3/6/2/3626) additionally describes the reset of the output registers with the array preserved. This does not remove the difference in the level of detail of the descriptions of the input registers and the output latches in the guide and the datasheet.

## Seven designs: what stays in the EBR and what moves to the fabric

The designs use one shared 1 024×18 array, two clocks, and independent enables of the two nine-bit groups of each port. Exactly one DP16KD memory was synthesized per design. NOREG was obtained on both ports everywhere.

| Probe | Behavior of the source design | Synthesis result besides one EBR |
|---|---|---|
| base | CE enables access; the output is held when WE=1 | 8 LUT4 |
| sync_ungated | Output reset and write inhibit take priority over CE | 12 LUT4, 4 PFUMX; the native RST receive the source reset |
| sync_gated | CE takes priority over reset; with reset active, writes are inhibited | 14 LUT4, 4 PFUMX; the native RST is conditioned on CE |
| sync_write_survives | The output is reset, an enabled write continues | 46 LUT4, 2 FF; the native RST are zero |
| async | Asynchronous read reset, independent write process | 8 LUT4; the native RST receive the source reset |
| fabric_pipeline | An 18-bit register with its own OCE for each port is added after the read | 8 LUT4, 36 FF in the fabric; both EBR are still NOREG |
| falling_edge | Access on the falling edge of the clock | 8 LUT4; CLKAMUX/CLKBMUX equal INV |

The results refer to Yosys 0.66 (`7f8fdfd8d7bc08c749a2a969388d3425d4f369d5`). In the probes without a used reset, the optimizer chose RESETMODE=ASYNC with zero RST; this is the choice for this design, not a requirement on user HDL.

In all seven designs, CSDECODE is 000, the CS inputs are tied to zero, GSR is disabled, and the native OCE are one. The dynamic OCE of the fabric_pipeline probe controls external FFs. Therefore the results of these probes do not close CS, GSR, or the native OCE; separate experiments HB03/HB04/HB12 are prepared for them.

The probes show why the behavior of a single EBR cannot be assigned from the source `if(reset)`, nor can any pipeline after the memory be declared its internal OUTREG. When emulating a bitstream, the actual parameters and the surrounding fabric must be executed. The two external FFs of the sync_write_survives probe likewise give no grounds to add two hidden FFs inside every hardware EBR.

896 combinations of the control inputs were checked: seven designs × two logical ports × 64 combinations of CE/WE/reset/BE[1:0]/OCE. The levels of CEA/B, OCEA/B, WEA/B, RSTA/B and of the low mask pins were stored. The results of evaluating the LUTs/muxes from the JSON matched Icarus Verilog on the synthesized connections. The masks at the memory inputs and the mutual exclusion of the native reset/write in the synchronous probes were also checked.

Only the logic **before the EBR inputs** is checked. The DP16KD itself stays a blackbox; its read, write, reset, and pipeline were not executed. The check does not establish the hardware priority of the control signals. For each design, when restoring the link between the logical and the native port, a possible A/B swap by the synthesizer was taken into account.

## Partial write and zero mask

The Yosys contract separates the common WR_EN from the group enables WR_BE. A group is written only when CLK_EN, WR_EN, and its WR_BE are active at the same time. For `rdwr new`, it assumes the whole new word of the array: the written groups get the new data, the others are preserved. This is a more specific synthesis assumption than the general description of WRITETHROUGH in the guide; the physical output with masks remains question B04.

A special case is WE=1 and BE=00. In the source probes `if(WE) … else read`, the output must be held even when no groups are actually written. Synthesis may generate a different CE/WE for this than the source HDL signals. Therefore the behavior of native WE=1/BE=00 cannot be established from the output hold of such a design. HB07 uses explicit control of the primitive and a check of the resulting configuration.

The experiment needs three distinct values: the memory contents M, the held output Q, and the input data D. Then, bit by bit, Q, M, D, and the merged word after a partial write can be told apart. The write into the array is checked by a later read through the other port at a separate time, so that a collision does not hide the result.

## Measurement queue

The original queue contains 12 series with initial conditions, control signal variants,
observable results, and a criterion for telling hypotheses apart. Later board results
and their limited scope are linked below and in [EBR_ADDRESSING.md](EBR_ADDRESSING.md).

| Series | What is checked | Questions |
|---|---|---|
| HB01 | Order of the PDP36 halves: INIT, live write, and the two conversion paths | B01 |
| HB02 | CE for the read and write core | B02 |
| HB03 | OCE, NOREG/OUTREG, and release of the pending result when CE=0 | B02 |
| HB04 | All CSDECODE values, a non-matching CS, and the pending result | B02 |
| HB05 | Synchronous reset versus CE/OCE/CS and writes | B02, B03 |
| HB06 | Asynchronous reset assertion without clocks and SYNC/ASYNC release | B02, B03 |
| HB07 | Four DP18 masks, three WRITEMODEs, zero mask | B04 |
| HB08 | 16 PDP36 masks and six read views | B01, B04 |
| HB09 | Same-edge DP collisions — **done** ([HB09](inventory/LFE5U-85F/hb09_ebr_collision/README.md)): equal width/mode, full words, CE/OCE=1; RBW reads old, NORMAL/WRITETHROUGH reads new; B wins double writes at 1/2/4/9; disjoint narrow words in one row are independent. Width-18 double writes mix bits. Mixed widths/modes, masks and different clocks/phases remain open | B05, B06 (measured subset) |
| HB10 | 25 DP width pairs, shared and non-overlapping cells | B05, B06 |
| HB11 | READBEFOREWRITE for the undocumented widths 1/2/4 — **done** ([HB11](inventory/LFE5U-85F/hb11_ebr_narrow_rbw/README.md): behaves as 9/18) | B07 |
| HB12 | INIT, first access, local reset, and GSR | B08, B03 |

The full conditions are in [hardware_cases.json](inventory/LFE5U-85F/ebr_control_research/hardware_cases.json). The general measurement rules and the queues of the other families are in [HARDWARE_VALIDATION.md](HARDWARE_VALIDATION.md). For each variant being built, the placed resource and the actual CRAM will have to be confirmed: the intent of the HDL does not replace checking what is programmed.

## Data and reproduction

[ebr_control_research](inventory/LFE5U-85F/ebr_control_research/evidence_report.json) holds six files: 12 contract rules, seven synthesis designs, 812 native connections, 896 control vectors, 12 series of hardware experiments, and a report with checksums. The number of series is not the number of bitstreams: one series includes many settings and repetitions. Variants that are structurally inapplicable to specific widths must be marked separately when the fixture is created.

```sh
python3 tools/collect_ebr_control.py
```

The source and synthesized HDL, the JSON of the designs, the input connection tests, and their traces are in `target/research/ebr/control-probes`. The checksums of these files, of the local libraries, of the Yosys sources, of the official sources, and of the executables used are stored in the report. The data of the earlier addressing research is unchanged. These synthesis probes do not measure EBR behavior; the later board results are linked in the queue above.

Regeneration with the new probe catalog gave byte-identical six research files. Also checked: the number and uniqueness of the ports, the full set of control combinations of each logical port, the masks at the native inputs, the independent Icarus traces, and the coverage of all B01–B08 in the hardware queue.
