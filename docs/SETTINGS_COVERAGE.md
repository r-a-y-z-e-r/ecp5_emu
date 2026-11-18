# Settings coverage: every Trellis setting of the LFE5U-85F against the emulator

Added on 2026-11-11. Trellis describes the chip's configuration as 3 274 settings in 148 tile
types. Folding instance letters and numbers (`SLICEA`→`SLICE`, `MULT18_3`→`MULT18`) leaves 322
distinct fields in 87 groups. Each field has been checked against the planner (`src/compile`,
`src/decode`): does the emulator turn it into behaviour, refuse it with an error, or not read
it? The full table, with the code line each verdict rests on, is
[settings_coverage.csv](inventory/LFE5U-85F/settings_coverage.csv). It was built by reading the
code on 2026-11-11, spot-checked, and updated for the changes made since.

## Summary

| Status | Fields | Meaning |
|---|---:|---|
| executed | 209 | read and executed, every value |
| partly | 28 | some values executed, the others refused with an error |
| refused | 21 | any value but the default is refused with an error |
| resource-not-executed | 23 | the whole block is not executed, so the field is never reached |
| ignored-no-effect | 41 | not read, and it has no digital effect (drive, slew, termination, PLL loop trims, test bits) |
| ignored-RISK | 0 | not read, and it could change digital behaviour: a silent mismatch |

Not executed at all (5 of 28 resource families): DQSBUFM, DDRDLL, DLLDELD, JTAGG,
SEDGA. ECLKBRIDGECS and BRGECLKSYNC were added on 2026-11-11 (HI06), and the
ALU54 on 2026-11-12 (HD02–HD08, [DSP_ALU.md](DSP_ALU.md)).

The 322 CSV rows were recounted on 2026-11-17 after configuration-port retention,
HD09, HS02, HC02, HY04, R03, R08, R09 and R15. DTR.MODE moves from resource-not-executed to
executed: the event kernel and fast mode use the board-profile code and measured
restart/fall timing, with 00 during high/conversion because count offsets from fall
are nonuniform. Fast mode host-schedules exact completion at fall + 32 740 000 ps
(1c79418). DCSMODE moves from resource-not-executed to executed: the
event kernel executes the measured handshake and documented static table; fast
mode refuses DCSC. The three SYSCONFIG port fields move from ignored-RISK to partly:
DISABLE preserves user IO; ENABLE is accepted when its pins are unused and refused
when a retained pad or USRMCLK ownership would conflict. Eight
ALU54_/MULT18_ CLKn_DIV fields move from partly to executed, and two SLICE
REGn.LSRMODE fields from refused to executed. Both values now execute in the
event kernel; mixed DSP divider profiles remain refused, and fast mode explicitly
refuses disabled DSP dividers. PRLD runs in fast mode as of ed624e7; an async
LSR-release/M-change race stops fast mode with an error. The earlier CCLK.MODE
update is retained.

R03 expands PIO.BASE_TYPE to all 17 differential outputs and 16 differential
inputs; 13 differential bidirs and open-drain differential outputs remain refused, so this field stays partly and
the R03 totals remained 204 executed, 32 partly, 22 refused, 23 resource-not-executed
and 41 ignored-no-effect (322 total). Input reads P; N and invalid-pair behaviour
are not modelled. No electrical/board qualification is claimed.

R08 moves IOLOGIC.DELAY.OUTDEL from refused to executed: HI11 validates registered and
fabric-FF output data with zero propagation delay, plus bank-specific DELAYF startup.
DELAY.WAIT_FOR_EDGE remains refused, with an error naming it. The recount is **205 executed,
32 partly, 21 refused, 23 resource-not-executed and 41 ignored-no-effect: 322 fields**.
HI11 measured left IOLOGIC and bottom SIOLOGIC; right/top startup follows the corresponding
kind as an assumption.

R09 moves ECLKSYNC0_BK.MODE, ECLKSYNC1_BK.MODE and BRGECLKSYNC.MODE from partly
to executed: both NONE and ECLKSYNCB now have execution. [HI12](inventory/LFE5U-85F/hi12_eclksync_none/README.md#reading)
confirms NONE as a buffer with no /2 deficit against pin-clock fabric counters;
/3.5 has the expected waveform and rounded counts with a free initial phase.
NONE remains qualified to an unrouted STOP path. HI12 directly measures bank 7
lane 1; other banks/lanes and BRGECLKSYNC NONE inherit the model. The current
recount is **208 executed, 29 partly, 21 refused, 23 resource-not-executed and
41 ignored-no-effect: 322 fields**. Enabled ECLKSYNCB still executes the HI04
four-falling-edge STOP model, but its post-load startup at a running pin clock
lags the HI12 board by four CLKI periods. That startup-history accuracy item
is open; it is not a remaining R09 refusal.

R15 moves GSR.SYNCMODE from partly to executed: ASYNC/NONE keep the no-sync
pattern, and [HY01](inventory/LFE5U-85F/hy01_sgsr/README.md) adds SYNC with
ACTIVE_LOW polarity and routed CLK. Two plain rising-CLK stages start at 11;
raw active OR s2 is distributed asynchronously with zero propagation time.
The 120 board residual bins are all within 20 ns of reset transitions and are
timing hazards. All field values now have execution; unmeasured cross-field
polarity/route combinations remain explicitly refused. The current recount is
**209 executed, 28 partly, 21 refused, 23 resource-not-executed and
41 ignored-no-effect: 322 fields**.

## Order of work

Every resource goes in the same way:
- the planner;
- the event kernel;
- fast mode;
- tests against the kernel;
- a board run where the documents leave the behaviour open.

1. **IOLOGIC 7:1.** Done on 2026-11-11: ODDR71B/IDDR71B, measured on the board
   ([HI07](inventory/LFE5U-85F/hi07_gearbox71/README.md)), run in both executors.
2. **The rest of IOLOGIC:**
   - latches (`INREGMODE`/`OUTREGMODE`): measured on hand-edited configurations (HI10) and
     executed in both executors since 2026-11-11;
   - CE, GSR and synchronous reset on the DDR paths;
   - delays: the data path runs with zero delay since HI08. DELAYF's counter and CFLAG run in
     both executors since 2026-11-11, checked against the HI08 records. HI11 closes output
     delay (R08) and sets startup to DEL_VALUE & 0x3E for IOLOGIC, 0 for SIOLOGIC; only
     WAIT_FOR_EDGE remains refused;
   - the memory DDR modes, which come with the DQS family.
3. **MULT18 and ALU54:**
   - registered SIGNEDP;
   - the output stage with live CE/RST: done (HD01);
   - MULT9 and the pre-adder;
   - the ALU54: done on 2026-11-12 for the measured profile (HD02–HD08,
     [DSP_ALU.md](DSP_ALU.md)). Of its registers only IN_1 remains.
4. **PLL:**
   - output trims;
   - dynamic phase;
   - STDBY and RST: executed in the event kernel since 2026-11-16, as
     [HC07](inventory/LFE5U-85F/hc07_pll_reset/README.md) measured; fast mode refuses
     live enabled controls. The restart ramp remains;
   - a PLL fed by another PLL.
5. **SLICE and LUTRAM:** the two refused fields and the mixed LUTRAM modes.
6. **SYSCONFIG:** configuration-port retention is checked in every startup profile.
   [Retained pads and USRMCLK conflicts are refused](SYSTEM_EXECUTION.md#persistent-configuration-ports).
7. **DCS.** Executed in the event kernel from HC02; both board configs route.
   Fast execution, startup/setup/hold and further transient measurements remain.
8. **JTAGG, SEDGA, and DQSBUFM/DDRDLL/DLLDELD.** USRMCLK is executed;
   its pull, configuration handoff and pad timing still need measurements.

## Silent risks

No rows remain classified ignored-RISK. MASTER_SPI_PORT, SLAVE_SPI_PORT and
SLAVE_PARALLEL_PORT are read from CRAM and preserved text directives. An enabled
port with unused pins is recorded as having no effect on the plan; using a retained
pad produces an error naming the field, package ball and function. This checks
ownership; configuration transactions are not executed.

## What is not fully executed, by resource

Every field not `executed` or `ignored-no-effect`, grouped by resource. Values are Trellis's
names.

### ALU54

36 fields: 22 executed, 1 without digital effect. Executed since 2026-11-12 as measured on the board ([DSP_ALU.md](DSP_ALU.md)).

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CLK0_DIV | partly | ENABLED | DISABLED on the clock of a register in use ('ALU54B nonstan… | Read only for the clocks the output and flag registers use. |
| CLK1_DIV | partly | ENABLED | DISABLED on the clock of a register in use ('ALU54B nonstan… | Read only for the clocks the output and flag registers use. |
| CLK2_DIV | partly | ENABLED | DISABLED on the clock of a register in use ('ALU54B nonstan… | Read only for the clocks the output and flag registers use. |
| CLK3_DIV | partly | ENABLED | DISABLED on the clock of a register in use ('ALU54B nonstan… | Read only for the clocks the output and flag registers use. |
| FORCE_ZERO_BARREL_SHIFT | refused | DISABLED | ENABLED ('ALU54B FORCE_ZERO_BARREL_SHIFT=ENABLED is not mea… |  |
| GSR | executed | ENABLED, DISABLED |  | The slice GSR bits, shared with both MULT18: enabled, GSR clears R at once and holds it (HD07); disabled, it does nothi… |
| LEGACY | refused | DISABLED | ENABLED ('ALU54B LEGACY=ENABLED is not measured') |  |
| MASK01 | ignored-no-effect |  |  | Not read. On the board EQZM stayed 1 and EQOM 0 with three masks (all ones, all zero, mixed): the flags never follow R … |
| MASKPAT | executed | any |  | Decides the constants EQPAT and EQPATB: the flags are those of a zero word against the patterns (HD04). A bit of 1 mask… |
| MASKPAT_SOURCE | refused | STATIC | DYNAMIC ('ALU54B MASKPAT_SOURCE=DYNAMIC is not measured') |  |
| MCPAT | executed | any |  | EQPAT is 1 when every unmasked bit is 0, EQPATB when every unmasked bit is 1 (HD04). |
| MCPAT_SOURCE | refused | STATIC | DYNAMIC ('ALU54B MCPAT_SOURCE=DYNAMIC is not measured') |  |
| MODE | executed | ALU54B (HD02-HD05); NONE (inactive: R reads zero … |  | Other ALU modes do not exist in Trellis for this primitive. Settings outside the measured profile leave only the MULT18… |
| REG_FLAG_CLK | executed | NONE (flags unregistered); CLK0..CLK3 (flag regis… |  | Measured with CLK0 (HD04). Registered flags beside CIBOUT_BYP of the second MULT18 are refused when a flag is used. |
| REG_INPUTC0_CLK | executed | NONE; CLK0..CLK3 (every other edge) |  | Holds C[26:0] and A[53:36] on every other edge of its clock (a divider GSR resets), CE0, RST0, RESETMODE; on only where… |
| REG_INPUTC1_CLK | executed | NONE; CLK0..CLK3 (every other edge) |  | Holds C[53:27] and B[53:36] on every other edge of its clock (a divider GSR resets), CE0, RST0, RESETMODE; on only wher… |
| REG_OPCODEIN_0_CE | executed | CE0..CE3 |  | The enable of IN_0 (HD08). |
| REG_OPCODEIN_0_CLK | executed | NONE, CLK0..CLK3 |  | OP[6:0], first stage, every edge (HD08). |
| REG_OPCODEIN_0_RST | executed | RST0..RST3 |  | The reset of IN_0 (HD08). |
| REG_OPCODEIN_1_CE | resource-not-executed |  | all | Read only for IN_1, which is refused. |
| REG_OPCODEIN_1_CLK | refused | NONE | CLK0..CLK3 ('ALU54B REG_OPCODEIN_1_CLK=... is not understoo… | With IN_1 on, 5-6% of the clocks followed no register model on the board, mostly through OP[3:2] (HD08 o4, o7). |
| REG_OPCODEIN_1_RST | resource-not-executed |  | all | Read only for IN_1, which is refused. |
| REG_OPCODEOP0_0_CE | executed | CE0..CE3 |  | The enable of OP0_0 and OP1_0 (HD08). |
| REG_OPCODEOP0_0_CLK | executed | NONE, CLK0..CLK3 |  | OP7, first stage, every edge (HD08). |
| REG_OPCODEOP0_0_RST | executed | RST0..RST3 |  | The reset of OP0_0 and OP1_0; synchronous or asynchronous by RESETMODE of the ALU (HD08). |
| REG_OPCODEOP0_1_CE | executed | CE0..CE3 |  | The enable of OP0_1 and OP1_1 (HD08). |
| REG_OPCODEOP0_1_CLK | executed | NONE, CLK0..CLK3 |  | OP7, second stage, every edge (HD08). |
| REG_OPCODEOP0_1_RST | executed | RST0..RST3 |  | The reset of OP0_1 and OP1_1; synchronous or asynchronous by RESETMODE of the ALU (HD08). |
| REG_OPCODEOP1_0_CLK | executed | NONE, CLK0..CLK3 |  | OP[10:8], first stage, on OP0_0's CE and RST (HD08). |
| REG_OPCODEOP1_1_CLK | executed | NONE, CLK0..CLK3 |  | OP[10:8], second stage, on OP0_1's CE and RST (HD08). |
| REG_OUTPUT0_CLK | partly | NONE (R[26:0] and R[53:45] stay 0 unless bypassed… | a clock other than the other output register's ('ALU54B out… | Clocks R[26:0] and R[53:45] (HD04). A group whose MULT18 has CIBOUT_BYP on is unregistered instead. |
| REG_OUTPUT0_RST | executed | RST0..RST3 (both fields on one line) | the two fields on different lines ('ALU54B REG_OUTPUT0_RST=… | The same bits as the first MULT18 REG_OUTPUT_RST, which nextpnr always writes: the reset line of R[17:0] and R[44:27]; … |
| REG_OUTPUT1_CLK | partly | NONE (R[44:27] stay 0 unless bypassed); CLK0..CLK3 | a clock other than the other output register's ('ALU54B out… | Clocks R[44:27] (HD04). A group whose MULT18 has CIBOUT_BYP on is unregistered instead. |
| REG_OUTPUT1_RST | executed | RST0..RST3 (both fields on one line) | the two fields on different lines ('ALU54B REG_OUTPUT0_RST=… | The same bits as the first MULT18 REG_OUTPUT_RST, which nextpnr always writes: the reset line of R[17:0] and R[44:27]; … |
| RESETMODE | executed | SYNC, ASYNC |  | ASYNC makes the reset of R[17:0] and R[44:27] asynchronous, as the first MULT18 RESETMODE does; R[26:18] and R[53:45] f… |
| RNDPAT | executed | any |  | The Z source for OP[6:4] = 6 (HD02). |

### IOLOGIC

30 fields: 10 executed, 3 without digital effect, 17 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CEIMUX | partly | 29 (all executed modes); CEMUX (IREG_OREG input re… | CEMUX in IDDRX1_ODDRX1/IDDRXN ('IOLOGIC clock enable CEIMUX… |  |
| CEOMUX | partly | 29 (all executed modes); CEMUX (IREG_OREG OREG/TSR… | CEMUX in IDDRX1_ODDRX1/ODDRXN ('IOLOGIC clock enable CEOMUX… |  |
| CLKIMUX | partly | CLK, INV | 0 when an input path (RXDATA/INFF) is used ('unsupported IO… |  |
| CLKOMUX | partly | CLK, INV | 0 when an output path (IOLDO/IOLTO) is used ('unsupported I… |  |
| DELAY.OUTDEL | refused | DISABLED | ENABLED ('IOLOGIC delay is not implemented'; its IOLDOD out… |  |
| DELAY.WAIT_FOR_EDGE | refused | DISABLED | ENABLED ('IOLOGIC delay is not implemented') |  |
| FF.INREGMODE | executed | FF, LATCH |  | Executed as measured in HI10: transparent while CLK is low and CE high, holding otherwise, LSR clears at once. Only with SRMODE=ASYNC, GSR=DISABLED and REGSET=RESET. yosys cannot produce it. |
| GSR | partly | DISABLED (all executed modes); ENABLED (IREG_OREG, IDDRX1_ODDRX1) | ENABLED with IDDRXN/ODDRXN ('IOLOGIC GSR=ENAB… | Trellis default ENABLED. X1 DDR measured in HI09. |
| IOLTOMUX | refused | TS, NONE (identical bits) | TDDR ('IOLOGIC IOLTOMUX [...] is not implemented') | Side IOLOGIC only. |
| LOADNMUX | executed | 202, LOADN |  | DELAYF data runs with zero delay; its counter and CFLAG run as measured (HI08). DIRECTION is taken at MOVE's rise. When DIRECTION changes while MOVE is high (outside the protocol), the result matches the board in 2 of 3 cases. |
| MIDDRX.MODE | refused | NONE | MIDDRX2 ('IOLOGIC MIDDRX.MODE is not implemented') |  |
| MODDRX.MODE | refused | NONE (MODDRX2 accepted only as the bit alias of O… | MODDRX2, MOSHX2 otherwise ('IOLOGIC MODDRX.MODE is not impl… |  |
| MODE | partly | IREG_OREG (SIOLOGIC IREG_OREG/NONE alias resolved… | MIDDRX_MODDRX (memory DDR, via sub-mode checks or 'IOLOGIC … | PIO DATAMUX/TRIMUX must agree with the mode (iologic.rs:212). |
| MTDDRX.MODE | refused | NONE (MTSHX2 accepted only as the bit alias of OD… | MTSHX2 otherwise ('IOLOGIC MTDDRX.MODE is not implemented') |  |
| OUTREG.OUTREGMODE | executed | FF, LATCH |  | Executed as measured in HI10: transparent while CLK is low and CE high, holding otherwise, LSR clears at once. Only with SRMODE=ASYNC, GSR=DISABLED and REGSET=RESET. yosys cannot produce it. |
| SRMODE | partly | ASYNC (all executed modes); LSR_OVER_CE (IREG_ORE… | LSR_OVER_CE with DDR/gearbox modes ('IOLOGIC SRMODE=LSR_OVE… |  |
| TSREG.OUTREGMODE | refused | FF | LATCH ('IOLOGIC latch is not implemented') |  |

### PLL

53 fields: 17 executed, 22 without digital effect, 14 below. RST/STDBY enables
are executed in the event kernel (HC07); fast mode refuses live enabled controls.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CLKOP_ENABLE | partly | ENABLED | DISABLED when that output is used ('PLL ... output ... is d… | ENCLK* dynamic enables are never used; with DISABLED the output would be gated by ENCLK* on hardware (unsure of exact s… |
| CLKOP_TRIM_DELAY | refused | 21 | 1, 2, 4 ('PLL ... is not implemented (only 0)') |  |
| CLKOP_TRIM_POL | partly | RISING (no trim) | FALLING ('... is not implemented (only RISING)') | Trellis default is FALLING, nextpnr writes RISING. |
| CLKOS2_ENABLE | partly | ENABLED | DISABLED when that output is used ('PLL ... output ... is d… | ENCLK* dynamic enables are never used; with DISABLED the output would be gated by ENCLK* on hardware (unsure of exact s… |
| CLKOS3_ENABLE | partly | ENABLED | DISABLED when that output is used ('PLL ... output ... is d… | ENCLK* dynamic enables are never used; with DISABLED the output would be gated by ENCLK* on hardware (unsure of exact s… |
| CLKOS_ENABLE | partly | ENABLED | DISABLED when that output is used ('PLL ... output ... is d… | ENCLK* dynamic enables are never used; with DISABLED the output would be gated by ENCLK* on hardware (unsure of exact s… |
| CLKOS_TRIM_DELAY | refused | 21 | 1, 2, 4 ('PLL ... is not implemented (only 0)') |  |
| CLKOS_TRIM_POL | partly | RISING (no trim) | FALLING ('... is not implemented (only RISING)') | Trellis default is FALLING, nextpnr writes RISING. |
| DPHASE_SOURCE | refused | DISABLED | ENABLED ('PLL ... is not implemented (only DISABLED)') | dynamic phase |
| FEEDBK_PATH | partly | INT_OP/INT_OS/INT_OS2/INT_OS3 through CLKINTFB; external output feedback traced through CLKFB | USERCLOCK / any CLKFB not traced to one of the PLL's outputs | HC07 fixes the CLKINTFB path used by LiteX INT_OS3. |
| INTFB_WAKE | refused | DISABLED | ENABLED ('PLL ... is not implemented (only DISABLED)') | internal-feedback wake |
| OUTDIVIDER_MUX | refused | DIVA (MUXA), DIVB (MUXB), DIVC (MUXC), DIVD (MUXD) | REFCLK and any crossed selection | Folded OUTDIVIDER_MUXA..D; each must select its own divider. |
| REFIN_RESET | refused | DISABLED | ENABLED ('PLL ... is not implemented (only DISABLED)') | reset on reference loss |
| SYNC_ENABLE | refused | DISABLED | ENABLED ('PLL ... is not implemented (only DISABLED)') | output sync |

### DQS

11 fields: 0 executed, 0 without digital effect, 11 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| DDRDEL | resource-not-executed |  | all | DQSBUFM not executed. |
| DQS_LI_DEL_ADJ | resource-not-executed |  | all | DQSBUFM not executed. |
| DQS_LI_DEL_VAL | resource-not-executed |  | all | DQSBUFM not executed. |
| DQS_LO_DEL_ADJ | resource-not-executed |  | all | DQSBUFM not executed. |
| DQS_LO_DEL_VAL | resource-not-executed |  | all | DQSBUFM not executed. |
| GSR | resource-not-executed |  | all | DQSBUFM not executed. |
| MODE | resource-not-executed |  | all | DQSBUFM not executed. |
| PAUSE_USED | resource-not-executed |  | all | DQSBUFM not executed. |
| RDLOADN_USED | resource-not-executed |  | all | DQSBUFM not executed. |
| READ_USED | resource-not-executed |  | all | DQSBUFM not executed. |
| WRLOADN_USED | resource-not-executed |  | all | DQSBUFM not executed. |

### MULT18

33 fields: 27 executed, 0 without digital effect, 6 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CLK0_DIV | partly | ENABLED (standard nextpnr profile, no highspeed d… | DISABLED ('MULT18 nonstandard clock divider is not implemen… | Read only for clocks a register stage selects. |
| CLK1_DIV | partly | ENABLED (standard nextpnr profile, no highspeed d… | DISABLED ('MULT18 nonstandard clock divider is not implemen… | Read only for clocks a register stage selects. |
| CLK2_DIV | partly | ENABLED (standard nextpnr profile, no highspeed d… | DISABLED ('MULT18 nonstandard clock divider is not implemen… | Read only for clocks a register stage selects. |
| CLK3_DIV | partly | ENABLED (standard nextpnr profile, no highspeed d… | DISABLED ('MULT18 nonstandard clock divider is not implemen… | Read only for clocks a register stage selects. |
| REG_INPUTC_CLK | partly | NONE; CLK0..CLK3 beside an executed ALU54B (its C… | CLK0..CLK3 beside no ALU ('MULT18 unsupported/unresolved IN… | The same bits as the ALU54B REG_INPUTC0/1_CLK of the slice. |
| SOURCEB_MODE | partly | B_SHIFT (parallel B, SOURCEA/B must be 0 at runti… | B_C_DYNAMIC, C_SHIFT, HIGHSPEED ('MULT18 C/dynamic/highspee… |  |

### SYSCONFIG

11 fields: 2 executed, 2 without digital effect, 7 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| DONEPHASE | partly | T0 (with 4/ON), T3 (with 21/OFF) | T1, T2 ('unsupported wake sequence/DONE_EX/DONEPHASE combin… | read only with --startup wake-phases; not read under configured/ready-unknown (GUI uses configured, src/live.rs:742) |
| GOEPHASE | refused | T1 | T2, T3 ('custom wake phase GOEPHASE is unsupported') | read only with --startup wake-phases; not read under configured/ready-unknown (GUI uses configured, src/live.rs:742) |
| GSRPHASE | refused | T2 | T1, T3 ('custom wake phase GSRPHASE is unsupported') | read only with --startup wake-phases; not read under configured/ready-unknown (GUI uses configured, src/live.rs:742) |
| GWEPHASE | refused | T2 | T1, T3 ('custom wake phase GWEPHASE is unsupported') | read only with --startup wake-phases; not read under configured/ready-unknown (GUI uses configured, src/live.rs:742) |
| MASTER_SPI_PORT | partly | DISABLE; ENABLE when retained pins are unused | ENABLE with retained user IO or configured USRMCLK | Package metadata supplies balls; serial/quad pins and conservative secondary SPI aliases; every startup profile. |
| SLAVE_PARALLEL_PORT | partly | DISABLE; ENABLE when retained pins are unused | ENABLE with retained user IO or configured USRMCLK | D0..D7, selects, write, busy, conservative daisy-chain outputs, CCLK. |
| SLAVE_SPI_PORT | partly | DISABLE; ENABLE when retained pins are unused | ENABLE with retained user IO or configured USRMCLK | MOSI/MISO/SN and CCLK; HOLDN is configuration-only. |

### DDRDLL

3 fields: 0 executed, 0 without digital effect, 3 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| FORCE_MAX_DELAY | resource-not-executed |  | all | DDR DLL not executed. |
| GSR | resource-not-executed |  | all | DDR DLL not executed. |
| MODE | resource-not-executed |  | all | DDR DLL not executed. |

### SED

3 fields: 0 executed, 0 without digital effect, 3 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CHECKALWAYS | resource-not-executed |  | all | SED (soft-error detect) not executed. |
| CLK_FREQ | resource-not-executed |  | all | SED (soft-error detect) not executed. |
| SEDEXCLK_USED | resource-not-executed |  | all | SED (soft-error detect) not executed. |

### DLLDEL

2 fields: 0 executed, 0 without digital effect, 2 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| DEL_ADJ | resource-not-executed |  | all | DLLDELD not executed. |
| DEL_VAL | resource-not-executed |  | all | DLLDELD not executed. |

### ECLKSYNC

2 fields: 2 executed, 0 without digital effect. R09 is fixed (HI12).

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| ECLKSYNC0_BK.MODE | executed | NONE buffer; ECLKSYNCB four falling-edge STOP model | NONE with connected STOP; unqualified bypass owner/mode | HI12 measured NONE on bank 7 lane 1; this lane inherits that qualification. HI13 closes enabled start: primary pin/constant-enabled DCC stages 0, fabric/other CE stages 1; PLL/OSCG history unmeasured. |
| ECLKSYNC1_BK.MODE | executed | NONE buffer (HI12); ECLKSYNCB four falling-edge STOP model (HI04) | NONE with connected STOP; unqualified bypass owner/mode | HI12 pin-clock NONE has no /2 deficit and /3.5 rounded counts fit zero deficit. HI12 sync_2 and HI13 P now start with stages 0; HI13 delayed DCCA retains stages 1 and four missing periods. |

### JTAG

2 fields: 0 executed, 0 without digital effect, 2 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| ER1 | resource-not-executed |  | all | Enables JTAGG user instructions ER1/ER2; JTAGG not executed (its outputs bail if reached). |
| ER2 | resource-not-executed |  | all | Enables JTAGG user instructions ER1/ER2; JTAGG not executed (its outputs bail if reached). |

### SLICE

24 fields: 24 executed, 0 without digital effect.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| REG0.LSRMODE | executed | LSR, PRLD | Async release/M-change race stops fast mode with an error | HS02 preload runs in both executors; event kernel uses X for the race. |
| REG1.LSRMODE | executed | LSR, PRLD | Async release/M-change race stops fast mode with an error | Same as REG0.LSRMODE; fast support ed624e7. |

### BRGECLKSYNC

1 field: 1 executed, 0 without digital effect. R09 is fixed for qualified NONE.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| MODE | executed | NONE buffer; ECLKSYNCB four falling-edge STOP model | NONE with connected STOP; unqualified bypass owner/mode | Same implementation and HI13 source-dependent initialization as ECLKSYNC; PLL/OSCG history unmeasured. HI06 measured enabled bridge synchronization; BRGECLKSYNC NONE itself was not directly measured by HI12. |

### CLKDIV (left)

2 fields: 1 executed, 0 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| GSR | partly | DISABLED | ENABLED ('CLKDIVF {key} GSR=ENABLED is not measured') | Trellis default is ENABLED; nextpnr writes DISABLED unless the cell sets GSR. |

### CLKDIV (right)

2 fields: 1 executed, 0 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| GSR | partly | DISABLED | ENABLED ('CLKDIVF {key} GSR=ENABLED is not measured') | Trellis default is ENABLED; nextpnr writes DISABLED unless the cell sets GSR. |

### DCS

1 field: 1 executed; event kernel only.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| DCSMODE | executed | All ten documented modes; NONE is the NEG bit alias | Fast mode refuses DCSC | HC02 two-new-edge POS/NEG handshake; immediate MODESEL bypass. CLK0_LOW/CLK1_HIGH SEL=00 and the other six static modes are documented-only; stopped new sources hold indefinitely without a timeout. |

### DSP_LEFT

1 fields: 0 executed, 0 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CIBOUT | partly | ON | OFF when a MULT18 of the slice is executed ('DSP slice CIB … |  |

### DSP_RIGHT

1 fields: 0 executed, 0 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| CIBOUT | partly | ON | OFF when a MULT18 of the slice is executed ('DSP slice CIB … |  |

### DTR

1 field: executed in the event kernel and fast mode (HY04; 1c79418). Fast mode
pauses on STARTPULSE changes; the host schedules the profile byte at the exact
latest fall + 32 740 000 ps, publishing it with any coincident clock edge before
captures. Fast snapshots preserve the DTR deadline and control history.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| MODE | executed | DTR; NONE (inactive X) | | Profile initial/final byte, default 0x9E; assertion clears/restarts, latest fall + 32.740 us completes. High/conversion drives 00; fall-relative steps are nonuniform. Both executors run this profile; fast completion uses the host scheduler. No Celsius scale. |

### EBR

30 fields: 29 executed, 0 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| ASYNC_RESET_RELEASE | partly | ASYNC; SYNC (with RESETMODE=SYNC, or while the lo… | SYNC release with RESETMODE=ASYNC and an active/unknown res… | Trellis default SYNC. |

### GSR

2 fields: 2 executed, 0 without digital effect. SYNC has the HY01 qualifications below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| SYNCMODE | executed | ASYNC/NONE (same no-sync pattern); SYNC with ACTIVE_LOW and routed CLK | Unmeasured SYNC polarity, missing CLK route, ambiguous mode | HY01/hy01b/c: raw active OR plain sync2 initially 11; no resets/preset; zero propagation delay and 20 ns reset-boundary hazards; equal event/fast traces and X20 config/bit. |

### PIO

15 fields: 6 executed, 8 without digital effect, 1 below.

| Field | Status | Executed | Refused or not reached | Note |
|---|---|---|---|---|
| BASE_TYPE | partly | Single-ended LVCMOS/LVTTL aliases; all 17 differential OUTPUT and 16 INPUT values | 13 differential BIDIR values; open-drain differential outputs; ambiguous ownership/direction; unsupported single-ended electrical modes | [R03](PIO_BANKS.md#differential-digital-io-r03-added-2026-11-17): output P=data/N=complement; input reads P, N and invalid-pair behaviour unmodelled. Catalogue support includes spellings the pinned tools cannot pack. |

