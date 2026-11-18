# FF registers, CCU2 and PLC output selection

Continuation of the main code — [asynchronous/synchronized GSR and system strobes](SYSTEM_EXECUTION.md): SLICE.GSR/REGSET and the separate startup/user GSR are taken into account. HS02 adds PRLD in the event kernel and fast mode. PUR remains unmodelled; [HY01](inventory/LFE5U-85F/hy01_sgsr/README.md) executes ACTIVE_LOW synchronized user GSR with a routed CLK. The rest of the text keeps the research snapshot and the limits of the open models; hardware S01–S06 remain open.

This pass collected the FF control encodings, the tables of the open CCU2 model and the overlaps with the output muxes. Through libtrellis it checked 305 bit-level groups of the database, 256 variants of the software TRELLIS_FF and 164 variants of CCU2C. The interfaces of 83 640 physical FFs come from the earlier cross-check of all PLCs. This continues the information gathering: the computational kernel and the working decoder were not changed, and there are no hardware measurements yet.

## Executed preload model (HS02)

The event kernel decodes REGn.LSRMODE=PRLD and the slice M0/M1 input as a
separate `FlipFlop.preload: Option<Input>`; ordinary SD still selects DI/M
independently. None means LSR/REGSET behavior; Some(M) means PRLD even for a
constant source. CEMUX behavior is unchanged.

With LSR_OVER_CE, a clock edge loads M when LSR is active, regardless of CE;
otherwise CE loads DI. REGSET does not choose the preload value. With ASYNC,
Q follows M while LSR is high and holds the last M after release. If M changes
in the same simulation instant that LSR falls, Q becomes X; timestamps are
tracked across capture waves, so routing order cannot select old/new M. Normal
clocked operation resumes after release, and CE=0 holds X until another update.
Under configured startup, M is read after the initial input fabric settles;
known M initializes Q, otherwise Q is X. This startup rule rests on one HS02s
observation, not a general POR measurement.

Fast mode runs PRLD as of `ed624e7`: synchronous LSR loads M over CE; ASYNC
follows M while LSR is active, and configured startup uses initial M. An unknown
initial M is a `StartsUnknown`. If M changes in the same instant as asynchronous
LSR release, fast mode stops with an explicit error (code 5), since it cannot
represent the event kernel's X. The check spans the waves of that instant.
HS02's CEMUX=1 probes had conflicting CE routing and are
excluded; no new constant-enable behavior is inferred. See
[HS02](inventory/LFE5U-85F/hs02_slice_prld/README.md) for evidence and exclusions.

The simulation experiments below are the earlier research snapshot. The HS02
board result supersedes its assertion-only asynchronous preload hypothesis.

## Inputs and scope of the FF settings

Each PLC has eight FFs, two per slice A/B/C/D. Each native FF has six pins: CLK, LSR, CE, DI, M and Q. A whole SLICE and the separate TRELLIS_FFs describe the same registers; their capacities do not add up.

| Setting/path | Scope | Meaning |
|---|---|---|
| REG0/1.SD | One FF | `1`: DI; `0`: M |
| REG0/1.REGSET | One FF | RESET / SET |
| REG0/1.LSRMODE | One FF | LSR / PRLD |
| GSR | The slice's FF pair | ENABLED / DISABLED |
| CEMUX | The slice's FF pair | CE / INV / 0 / 1 |
| MUXCLK0…3 | One selector per slice | CLK0 or CLK1 |
| MUXLSR0…3 | One selector per slice | LSR0 or LSR1 |
| CLK0/1.CLKMUX | Shared raw PLC line | CLK / INV |
| LSR0/1.LSRMUX and SRMODE | Shared raw PLC line | Polarity and LSR_OVER_CE / ASYNC |

This is the structure of the database fields and interfaces. In addition, nextpnr requires a common clock, LSR, their polarities and ASYNC for the FFs of the whole PLC. Within an FF pair it checks the compatibility of CE and GSR, and the use of M by the extension mux. These placement rules are kept separate from the physical constraints (L05, S03).

`pack_ffs` sets SD=1 when it manages to place the FF together with the COMB that feeds data to DI. In all other cases it sets SD=0 and renames the DI input to M. Therefore M cannot be considered only a preload input or only a selection of the extended LUT.

The local DI path also requires an output switch: the database has fixed connections `Fn → DIn → DIn_SLICE` for each number n. This path cannot start directly from the raw LUT result, bypassing the Fn selection, without taking the mode into account. The ordinary DI/M selection and the selection of the value source during preload are different functions that use the shared M; the permitted SD/PRLD combinations require S02/S03.

The materials hold 50 fields with 111 value variants, including MODE and INJECT1 for context. Separately, 194 variants of CLK/LSR/CE routes, slice selectors and F outputs are saved. All these groups matched the API of the installed libtrellis bit for bit.

## CE and reset: what is already established

The [Diamond 3.13 primitives guide](https://www.latticesemi.com/view_document?document_id=53950), pp. 36, 363–365, sets, for FD1P3IX/JX, the priority of the synchronous local clear/preset over enable. In the open TRELLIS_FF this is implemented by the same order of checking LSR and CE in the synchronous and asynchronous branches.

For this software model, after the polarities are selected:

1. ASYNC reacts to the active clock edge or to the assertion of LSR; LSR_OVER_CE reacts to the active clock edge.
2. If LSR is active, Q takes the reset/preset value regardless of CE.
3. Otherwise, with effective CE=1, Q takes DI; with CE=0 it holds.
4. With LSRMODE=LSR the control value equals REGSET; with PRLD, M is taken.

Edges, asynchronous events and hold were checked on all 256 combinations of `4 CEMUX × 2 CLKMUX × 2 LSRMUX × 2 SRMODE × 2 REGSET × 2 LSRMODE × 2 GSR`. For each variant, 56 transitions were run, with control and clock changes separated in time: 14 336 cross-checked transition observations in total. The initial 256 samples are recorded separately and were not used as evidence of the physical INIT.

An HDL probe with reset inside `if (ce)` is synthesized into an FF and an additional LUT. From the actual pins and the INIT of this LUT it was verified that it forms `LSR = reset & ce`. Therefore the external priority of enable over reset can be implemented by the fabric around the FF; it must not be attributed to another hidden register mode.

All 64 combinations of the local bits of the four CEMUX and the eight MUXCLK/MUXLSR are stored. These are bit-pattern match tables, not a list of permitted hardware modes. For example, CEMUX=CE has an empty bit pattern, while the constants also use the bits of the CE route. An empty match does not permit enabling ordinary CE, inversion and a constant at the same time. With partial codes and the simultaneous selection of two lines, the ambiguity stays explicit.

## TRELLIS_FF limitations and initial state

The installed open model has three significant limitations for emulation from a bitstream:

- GSR is declared as a parameter, but there is no global input and no handling of this event. All 128 pairs of traces that differ only in GSR matched; this is an expected property of the model, not a check of the physical GSR.
- SD is not declared as a parameter: the normal write always takes DI. After packing, SD=0 and data on M require a separate interface conversion. The mere presence of M in the model does not solve this task.
- For ASYNC+PRLD, the event of M changing while LSR is already active is absent from the sensitivity list. In a directed probe with the clock stopped: LSR asserted with M=0 → Q=0; M changes to 1 → Q stays 0; the next active clock cycle → Q=1. Whether the physical preload can follow M continuously is checked in HS02.

The Verilog contains `initial Q = srval`, and with PRLD srval=M. The resulting initial value depends on the inputs and on the order of the simulator's initial events; this is not a load rule of the physical FPGA. In the experiment M started at zero: the 128 PRLD variants gave 0, the ordinary variants gave 64 zeros and 64 ones. A separate check is needed for PUR/configuration completion, the user GSR, its disabling in the slice, and combinations with PRLD (S01).

The defaults also differ. The PLC2 database sets REGSET=SET and SD=1; TRELLIS_FF uses REGSET=RESET by default, and the nextpnr writer takes 0 when SD is absent. For example, for A.REG0, SET corresponds to `!F2B11` and RESET to `F2B11`; SD=1 is `!F4B11` and SD=0 is `F4B11`. The emulator must read the actual bits and preserve uncertainties instead of substituting the values of the HDL parameters.

In Yosys `lattice_gsr`, the AUTO parameter depends on the presence of GSR/SGSR and on the **cell** attribute `nogsr`. The probes confirmed DISABLED without a global primitive, ENABLED with GSR or SGSR, and DISABLED on a directly marked TRELLIS_FF. Marking the output wire with `nogsr` did not reach the cell in a separate probe. The pass can also remove the local asynchronous reset wiring that corresponds to the global signal. This is a circuit transformation during synthesis; it does not reproduce the propagation of a global event in a working chip.

## Preload and latch in Yosys 0.66 and 0.70

In the first snapshot of Yosys 0.66, the same HDL with asynchronous load of M was checked in three ways:

| Synthesis invocation | Observation |
|---|---|
| Plain synth_ecp5 | The legalizer rejects ALDFFE |
| With `-asyncprld` | ABC9 rejects a non-constant initial Q=M |
| With `-asyncprld -noabc9` | One TRELLIS_FF with ASYNC, PRLD and GSR=DISABLED was obtained |

These are observations of Yosys 0.66, a specific version. After the [update to Yosys 0.70](TOOLCHAIN.md), the third invocation stopped bypassing ABC9: `-noabc9` is accepted, but ABC9 still runs and rejects a non-constant INIT. All three rejections are kept in the new report. Placement and routing of these probes were not performed; a conclusion that the chip has no preload does not follow from them.

The HDL latch `if (ce) q=d` in the ordinary Yosys 0.66 flow gave one LUT4 with Q fed back to the input. In Yosys 0.70 such an implicit latch is rejected by default; with `-latches info`, and in a separate probe with an explicit SystemVerilog `always_latch`, a LUT with feedback was obtained again. Over all binary assignments of its pins, `Q_next = ce ? d : Q` was verified. This is explained by `latches_map.v`. The mention of level-sensitive registers in the datasheet, p. 16, is still not linked to a found configuration of the hardware FF: L09 is kept and refined in S06. LUT feedback does not prove such a binding.

The first snapshot had 14 synthesis variants: 12 successful and two rejections. The current Yosys 0.70 table has 16 variants: 12 successful and four rejections, including the changed preload and the implicit latch. Parameters, cell composition, connections, source HDL and result checksums are kept as tool evidence; the previous snapshot is in the update archive.

## The two halves of CCU2

The public CCU2C from `ccu2c_sim.vh` defines two sequential halves. For each:

```text
a = A + 2*B + 4*C + 8*D
p = INIT[a]
g = INIT[A + 2*B]
S = p XOR (INJECT1 == NO ? carry_in : 0)
carry_out = p ? carry_in : (INJECT1 == NO ? g : 0)
```

For the first half, carry_in=CIN; for the second, it is the carry_out of the first. The four low bits of INIT take part in both the LUT4 and the LUT2; there is no separate independent generate table.

With INJECT1=YES the sum equals p, but with p=1 the carry continues the input carry. Therefore interpreting YES as an unconditional zeroing of the carry is wrong. For NO the ordinary propagate/generate are obtained: with p=1 the carry passes through, with p=0 g is taken. A table of 16 combinations of p/g/carry/injection is stored as an abstract function; some independent values of p/g with fixed pins are constrained by the shared INIT.

41 INIT pairs were checked: one-hot positions of each half in turn, asymmetric and arithmetic bit patterns, and carry-adapter bit patterns. For each, four INJECT1 pairs and all 512 binary assignments of the eight LUT inputs and CIN. In total, 164 variants and **83 968 verified S0/S1/COUT triples**. This is a cross-check of the digital formula against the published model; not all 2³² INIT pairs were enumerated. Undefined values and analog effects were not explored.

The error of the hidden external FCO in TRELLIS_COMB, found in the [previous collection](PLC_LUTRAM.md), still stands. Therefore CCU2C is used here, where the carry output is actually assigned. It cannot be declared a hardware reference merely because this software error is absent.

## Output muxes and carry boundaries

In each of the eight F outputs, one bit is described at the same time as the selection of the extension output and as INJECT1:

| CRAM of this bit | Ordinary LOGIC path per the database | CCU2 field value |
|---|---|---|
| 0 | Fn_SLICE | YES |
| 1 | F5x_SLICE for even n, or FXx_SLICE for odd n | NO |

For example, A.K0 uses F8B10 and A.K1 uses F8B11. All 16 rows over the eight outputs are stored. With MODE=CCU2, the ordinary LOGIC switch must not additionally be executed on the same bit, thereby replacing the arithmetic sum with the wide-LUT output. The exact conditions of F/OFX activity under CCU2, RAMW/DPRAM and partial MODE remain L06/S04. The COMB model without the route mode does not settle this question.

The nextpnr writer writes INJECT1 for CCU2 and passes `_NONE_` for the other modes. In libtrellis this means no change to the group's bits, not a third physical mode. In the studied subset, 20 positions shared with other fields/routes were found: eight INJECT/F, eight CE/CE route, three WREMUX/LSR and the shared MODE A/B/C. The full PLC catalog earlier contained 100 such positions.

Inside a PLC, the eight stages go through K0→K1 and A→B→C→D. Output D is connected to FCO, then to `E1_HFIE0000`; the input of the neighboring PLC arrives through HFIE0000→FCI→FCI_SLICE. The native name conversion was done for all **10 455 outputs**. For 10 370 of them the destination matches the coordinate of the next PLC. Another 85 outputs are directed to column 125, where there is no PLC BEL. The mere existence of a canonical wire name does not establish its physical termination, a constant, or observability; these boundaries are kept in S04.

nextpnr inserts additional ordinary CCU2s for an input from the fabric and an output/branch into the fabric:

- Feed-in: INIT0=0x000a, INIT1=0xffff, INJECT=NO/YES; the signal enters through A0. With the unused C/D held high, the function COUT=A0 does not depend on the input CIN.
- Feed-out: INIT0=0, INIT1=0x000a, INJECT=NO/NO; S0 outputs CIN into the fabric. To continue the chain, S0 returns to A1 and restores COUT=CIN.

Six directed observations of these identities were extracted from the model traces. These are additional occupied resources and ordinary circuit connections, not hidden free carry ports. The bit patterns and the binding of unused inputs are taken from the packer/writer sources; a real P&R of these probes was not run here.

After the update, nextpnr also contains a separate branch for breaking a long chain: S0 outputs the carry into the ordinary fabric and feeds it to the next CIN, after which a new chain segment is created. This is an addition to the loopback described above, not a replacement of the two-half identities. The update check with placement/routing was run on a separate short counter; the long break was not checked on the board.

## What remains for the board

| ID | Unestablished part |
|---|---|
| S01 | Startup/PUR, user GSR, REGSET/PRLD, disabled GSR and the global synchronizer |
| S02 | Continuous or event-driven preload, compatibility of SD/PRLD and the shared M |
| S03 | Shared control lines, FFs in a pair, DI/M and combination with wide LUTs |
| S04 | Outputs and carries by MODE, partial encodings and physical chain boundaries |
| S05 | Policy for X/conflicts and coinciding events; Verilog branching is not a chip guarantee |
| S06 | Configuration binding of the level-sensitive FF from the datasheet |

Five series HS01–HS05 are prepared; they refine the existing L02/L04/L05/L06/L09 without replacing their status with hardware guesses. The plan and the distinguishable observations are in the contract and in the shared [board queue](HARDWARE_VALIDATION.md). For S05, the kernel's digital policy must be defined separately; a physical board does not produce an abstract Verilog X.

## Data and reproduction

The result is [logic_state_research](inventory/LFE5U-85F/logic_state_research): 18 CSV/JSON files. It stores the FF and encoding bit patterns, the raw combinations of control bits, the output contexts, all chain outputs, the variants and observations of the models, the preload diagnostics, the carry adapters, the synthesis results, the contract and the source checksums.

```text
python3 tools/collect_logic_state.py
```

The collector uses the installed libtrellis and the previously collected PLC audit, checking that the library and the BEL constructor are unchanged. Yosys, Icarus/vvp, the native `target/trellis_wire_probe`, local Trellis/nextpnr and the saved documents are also needed. Paths can be overridden with parameters. After the update, a repeat collection into a separate directory with fresh experiments gave 18 byte-identical files. The CSV row sizes, the completeness of variants/observations, the ranges of local bits, the question/experiment IDs and the document links were checked; the current report keeps 76 checksums of sources and results. The new probe sources are in `target/toolchain-update/logic-probes`.

The next collection is global clocks and system startup: physical distribution segments, GSR/PUR, OSC and the connection to PLL/DCC/DCS. The questions listed above remain open pending the corresponding sources or measurements.
