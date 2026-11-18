# IOLOGIC execution (DDR X1/X2 and IO registers)

Added on 2026-10-29. The emulator executes the side IOLOGIC and the top/bottom SIOLOGIC in the
two modes that nextpnr uses for ordinary designs:
- `IDDRX1_ODDRX1` (`ODDRX1F`, `IDDRX1F`);
- `IREG_OREG` (flip-flops packed into the IO with `(* syn_useioff *)`).

Since 2026-11-11 it also executes the X2 gearbox of the side IOLOGIC (`ODDRX2F`, `IDDRX2F`,
see [X2 gearbox](#x2-gearbox)).

The behaviour comes from the board measurement
[HI02](inventory/LFE5U-85F/hi02_iologic/README.md). It was run on the left bank (LED pins) and
on the top bank (the GPDI/HDMI pins), and every digital sample is the same on both. The model is
[kernel/iologic.rs](../src/kernel/iologic.rs), and the planner part is
[compile/iologic.rs](../src/compile/iologic.rs).

## Behaviour

| Path | Ports | Model | Basis |
|---|---|---|---|
| ODDRX1F | `TXDATA0`, `TXDATA1` → `IOLDO` | A pair captured at an `SCLK` rising edge leaves two rising edges later: `TXDATA0` in the high phase, `TXDATA1` in the following low phase. The output changes only at clock edges | HI02, HI02b |
| IDDRX1F | `PADDI` → `RXDATA0`, `RXDATA1` | `RXDATA0` = pad at a rising edge, `RXDATA1` = pad at the following falling edge, both published at the next rising edge | HI02, HI02b |
| OREG | `TXDATA0` → `IOLDO` | rising-edge flip-flop | HI02b |
| TSREG | `TSDATA0` → `IOLTO` | rising-edge flip-flop; drives the PIO tristate | HI02b (driver enable at the edge) |
| IREG | `PADDI` → `INFF` | rising-edge flip-flop sampling the pad | HI02b |
| Start-up | — | DDR stages start at 0; IO registers at `REGSET` | HI02b, `GSR=DISABLED` |
| Reset | `LSR` through `LSRMUX` and `LSRIMUX`/`LSROMUX` | asynchronous (`SRMODE=ASYNC`): holds the DDR stages at 0 and IO registers at `REGSET`; capture restarts at the first edge after release | HI02b |
| Top bank | the SIOLOGIC subset of the same ports | identical to the side IOLOGIC | HI02c |
| IO register CE | `CE` through `CEMUX` and `CEIMUX`/`CEOMUX=CEMUX` | a register captures at a rising edge only while CE is high | HI05 |
| IO register sync set/reset | `LSR` with `SRMODE=LSR_OVER_CE` | REGSET at the next rising edge, whatever CE is | HI05 |
| IO register GSR | `GSR=ENABLED` | the global reset sets REGSET at once while active; `GSR=DISABLED` registers ignore it; both start at REGSET | HI05 |
| IO latch | `FF.INREGMODE`/`OUTREG.OUTREGMODE=LATCH` (set by hand: yosys has no IO latch cell) | transparent while CLK is low and CE high, holding its output otherwise; LSR (`ASYNC`) clears it at once; starts at 0 | HI10 |
| X1 DDR GSR | `GSR=ENABLED` on ODDRX1F/IDDRX1F | the global reset clears every DDR stage and output at once while active, as `RST` does; `GSR=DISABLED` paths ignore it | HI09 |

The IOLOGIC has one `CLK`, one `CE` and one `LSR` input. `CLKIMUX`/`CEIMUX`/`LSRIMUX` apply them
to the input path (IDDR, IREG), and `CLKOMUX`/`CEOMUX`/`LSROMUX` to the output and tristate paths
(ODDR, OREG, TSREG). `CLK*MUX=INV` and `LSRMUX=INV` invert the routed signal; this follows the
setting name and the PLC muxes of the same name, and was not measured on the IOLOGIC.

A clock edge that may or may not have happened (an `x` clock) merges the held and the captured
values, as for PLC flip-flops. An `x` reset merges the held value with the reset value.

## PIO connection

The PIO takes its data and tristate inputs either from the fabric (`PADDO`, `PADDT`) or from the
IOLOGIC (`IOLDO`, `IOLTO`). Separate fields make the selection: `DATAMUX_ODDR`, `DATAMUX_OREG`
and `TRIMUX_TSREG` set to `IOLDO`/`IOLTO`. At most one field per direction may select the
IOLOGIC. `DATAMUX_MDDR` is taken as the X2 gearbox's output (see [X2 gearbox](#x2-gearbox));
with an IOLOGIC in a memory mode it is rejected.

Two kinds of wires have fixed arcs from two IOLOGIC outputs. The planner keeps the source that
is active in its mode and records the rule in `report.routes`:
- The CIB wires `JQ1..JQ3` next to a pair carry `RXDATA1..3` of B/D and also the 7:1 lanes
  `RXDATA4..6` of A/C. The lanes are active only with `IDDRXN.MODE=IDDR71`.
- The PIO input `IOLDO` is fed by `IOLDO` and by the output delay `IOLDOD`. The delay is active
  only with `DELAY.OUTDEL=ENABLED`.

Any other overlap of fixed sources still fails.

## Mode

The planner takes `MODE` from the IOLOGIC and checks that the PIO selects the same path:
- `DATAMUX_ODDR` for the ODDR;
- `DATAMUX_OREG` for the OREG;
- `TRIMUX_TSREG` for the TSREG.

On the top/bottom SIOLOGIC, `MODE=IREG_OREG` and `MODE=NONE` have the same bits, so the decoder
keeps both. When the plan uses a register output (IOLDO, IOLTO or INFF), the mode is taken as
`IREG_OREG`: `NONE` has no register outputs. The rule is recorded with the resource in
`report.resources`. The SIOLOGIC has no X2/7:1, memory or `IOLTOMUX` fields; their absence is
accepted there and rejected on the side IOLOGIC.

## X2 gearbox

`ODDRX2F` and `IDDRX2F` work from two clocks: SCLK (the IOLOGIC `CLK`, through
`CLKOMUX`/`CLKIMUX`) and ECLK, the bank's edge clock. In the designs nextpnr builds, ECLK comes
from an ECLKSYNCB and SCLK from a CLKDIVF /2 of the same edge clock
([CLOCK_NETWORK.md](CLOCK_NETWORK.md)). The board measurement is
[HI03](inventory/LFE5U-85F/hi03_gearbox/README.md), on the left bank.

| Path | Ports | Model | Basis |
|---|---|---|---|
| ODDRX2F | `TXDATA0..3` → `IOLDO` | The four bits captured at an SCLK rising edge leave from the ECLK rising edge where SCLK falls, 2.5 SCLK cycles later: `TXDATA0` in that ECLK high half, `TXDATA1` in the low half, then `TXDATA2` and `TXDATA3`. The pad changes only at ECLK edges | HI03 |
| IDDRX2F | `PADDI` → `RXDATA0..3` | `RXDATA0..3` published at an SCLK rising edge E are the pad at the ECLK rising edge where SCLK fell 1.5 SCLK cycles before E, the falling edge after it, the rising edge at the SCLK edge before E, and the falling edge after that | HI03 |

The planner builds each from the X1 parts ([compile/eclk.rs](../src/compile/eclk.rs)):
- **SCLK side.** Ordinary IO registers on internal nets: three ranks for ODDRX2F, one for
  IDDRX2F.
- **ECLK side.** One path of [kernel/iologic.rs](../src/kernel/iologic.rs):
  - `GearOut` loads the word at the ECLK rising edge where SCLK falls (SCLK read before the
    edge) and shows one bit each ECLK half;
  - `GearIn` collects the pad at four ECLK edges and hands the word to the SCLK rank at that
    same edge.
- **Reset.** `LSR` clears every stage, as HI02 showed for X1; HI03 did not exercise it.
- **PIO.** The ODDRX2F output reaches the PIO through `DATAMUX_MDDR`, whose bits are those of
  `DATAMUX_ODDR` and `DATAMUX_OREG` together; the planner reads that overlap as the gearbox.

### 7:1

`ODDR71B` and `IDDR71B` with CLKDIVF /3.5 ([HI07](inventory/LFE5U-85F/hi07_gearbox71/README.md),
left bank). The gearboxes need PIO A or C. RXDATA4..6 reach the fabric on the CIB wires the
pair's B/D shares (see *PIO connection*). ODDR71 reaches the PIO through `DATAMUX_ODDR`.

| Path | Ports | Model | Basis |
|---|---|---|---|
| ODDR71B | `TXDATA0..6` → `IOLDO` | The seven bits captured at an SCLK rising edge leave from the ECLK edge after the next SCLK rising edge, one per ECLK half: D0 first | HI07 |
| IDDR71B | `PADDI` → `RXDATA0..6` | At SCLK rising edge P, RXDATA0..6 take the pad at the seven ECLK edges ending at the edge after the previous SCLK rise, oldest first into Q4, Q5, Q6, Q0, Q1, Q2, Q3 | HI07, HI07b |

The planner builds each from SCLK registers and one ECLK path:
- **ODDR71B.** Two SCLK ranks and `GearOut7`, which loads the word at the ECLK edge after SCLK
  rises and shows the next bit at each later edge.
- **IDDR71B.** `GearIn7`, which keeps the last seven pad samples and hands them on at that same
  edge, and one SCLK rank.

`tests/gearbox_native.rs` runs `hi03.config`; both executors print exactly the board's line.
That needs the edge clock's HI04 fabric-source start too: ECLKSYNCB passes
ECLKI from its fifth rising edge. [HI12](inventory/LFE5U-85F/hi12_eclksync_none/README.md#reading)
closes R09 for MODE NONE with unrouted STOP: an immediate buffer, with no /2
count deficit and /3.5 counts consistent with the ratio and a free initial phase.
[HI13 run_1](inventory/LFE5U-85F/hi13_eclksync_start/README.md#reading) closes the
enabled pin-clock startup discrepancy: initialize all four STOP stages to 0
for a primary clock input pin through routes/buffers and constant-CE=1 DCC/DCCA;
keep 1 for fabric logic or other DCC enables. HI13 P loses no input periods;
its delayed DCCA lane loses four. PLL/OSCG configuration-time activity remains
unmeasured and uses stages at 1. The four-falling-edge runtime STOP pipeline
is unchanged; event and fast mode use the same kernel initial values. See the
[full start rule](CLOCK_NETWORK.md#eclksync-startup).

<a id="input-delay"></a>

## Input and output delay

DELAYG and DELAYF on inputs ([HI08](inventory/LFE5U-85F/hi08_delay/README.md)) and outputs
([HI11](inventory/LFE5U-85F/hi11_output_delay/README.md)) execute with zero delay. That is how the emulator executes every propagation delay: DEL_VALUE and the
run-time moves (up to 127 taps, a few ns) change timing, not data.
- **Accepted settings:** any `DEL_VALUE`, `DELAY.OUTDEL=DISABLED/ENABLED`, and `LOADNMUX`
  `LOADN` or `1` (LOADN tied high).
- **INDD.** The delayed pad to the fabric is planned as the pad.
- **IOLDOD.** The delayed output is IOLDO passed through a zero-time buffer, unchanged and
  uninverted. IOLDO is the IO output register, or TXDATA0 when the register is bypassed
  (`MODE=NONE`). The PIO follows IOLDOD when OUTDEL is enabled, including the bypass case
  where nextpnr leaves DATAMUX_OREG at its fabric default. HI11 received correct UART bytes
  through OFS1P3BX/DELAYF, OFS1P3BX/DELAYG(127), and fabric FF/DELAYG(127) on left-bank L4.
- **CFLAG.** DELAYF's counter runs as an IOLOGIC path clocked by MOVE (`IoPath::DelayCounter`),
  in both executors, as the board showed it:
  - after configuration, side-bank **IOLOGIC starts at `DEL_VALUE & 0x3E`**, retaining bits
    1..5; top/bottom **SIOLOGIC starts at 0**. HI11 measured output L4 and input A2 on the
    left banks, and input R1 on the bottom bank; HI08 also measured the bottom FIRE inputs.
    Right IOLOGIC and top SIOLOGIC use the same rule as their kind as an **unmeasured
    assumption**. The six side-bank values 1/10/63/64/100/127 started at 0/10/62/0/36/62;
  - LOADN low loads all seven bits of DEL_VALUE at once and holds it; MOVE does nothing
    meanwhile. The startup mask does not limit subsequent loading or stepping;
  - each MOVE pulse is one step, made at its falling edge, saturating at 0 and 127; DIRECTION 0
    counts up, 1 down;
  - CFLAG = `DIRECTION ? counter == 0 : counter == 127`, at once (plan muxes over the counter
    bits and DIRECTION).
  - **DIRECTION is taken at MOVE's rise.** Inside the documented protocol (DIRECTION steady
    while MOVE is high) this is the same as taking it at the fall. When DIRECTION changes while
    MOVE is high, the board followed the direction at the rise in two of its three cases (HI08b
    B and C). In the third (HI08c F) it did not.
  - **Out-of-protocol mismatches.** In those cases CFLAG also differs while MOVE is still high:
    the board drops it where the model holds it. The emulator differs from the board record in
    8 samples of HI08b and 30 of HI08c, all of them between such a change and the next load.
    Taking DIRECTION at the fall would differ in 108 and 30 samples.
  - LOADN low also clears the DIRECTION taken at the rise. A MOVE pulse that overlaps a load was
    not measured.
- **Refused.** `DELAY.WAIT_FOR_EDGE=ENABLED`, with an error naming that setting. The public
  nextpnr DELAYG/DELAYF packer does not set it; OUTDEL is accepted.

`tests/io_delay.rs` builds a DELAYF pair (DEL_VALUE 3 and 125) under pseudo-random LOADN, MOVE
and DIRECTION with Yosys/nextpnr. It runs 6 000 cycles, and CFLAG agrees in both executors and
with a reference model. The run reaches both ends many times and changes DIRECTION while MOVE is
high. `tests/io_delay_native.rs` runs the board's own HI08 configurations in fast mode:
- **HI08:** the UART line is the board's line, all 2 048 samples.
- **HI08b and HI08c:** the lines differ only in the out-of-protocol samples above.

`tests/output_delay.rs` builds X12, output DELAYF with input-driven controls and CFLAG, and
the fabric-FF bypass variant. Pad values and CFLAG have equal kernel/fast traces at inputs,
clock edges and between edges. It also replays the six native HI11s configurations, comparing
all 2048 captured control/CFLAG samples against the board on every used lane, plus equal
kernel/fast values throughout warm-up and at both clock edges. No fast implementation change
is needed: the planner uses existing buffers, IO registers and DELAYF flag logic; only the
counter drivers' initial bits differ by IOLOGIC kind.

## Rejected with an explicit error

These were not measured:
- the memory modes (`MIDDRX`, `MODDRX`, `MTDDRX`);
- `IOLTOMUX` other than `NONE`/`TS` (DDR tristate);
- `SRMODE=LSR_OVER_CE` and a clock enable on the DDR and gearbox paths, and `GSR=ENABLED` on
  the gearbox paths. The IO registers take all three since HI05, the way yosys maps
  `IFS1P3*`/`OFS1P3*`. The X1 DDR paths take `GSR=ENABLED` since HI09. ODDRX1F/IDDRX1F have no
  CE or SRMODE, so the open toolchain cannot produce those two;
- a latch with `SRMODE=LSR_OVER_CE`, `GSR=ENABLED` or `REGSET=SET`, and the tristate register
  in latch mode (HI10 measured the input and output latch with `ASYNC`, `GSR=DISABLED` and
  `REGSET=RESET`);
- the delay's `WAIT_FOR_EDGE` and `OUTDEL`.

## Checks

- `tests/iologic_native.rs` (`--ignored`) runs the three configurations that ran on the board and
  decodes their UART reports:
  - `hi02.config`: the line equals the board's. About 194k reference cycles, ~25 s.
  - `hi02b.config`: all digital signals equal the board's in all 384 samples. The TSREG pad
    differs only where the board's pad is still discharging after a release, which is analog
    behaviour. About 1.26M reference cycles, ~14 minutes. Its 2304-bit recorder makes every
    reference edge evaluate ~2400 flip-flops; that is a kernel performance limit, not IOLOGIC
    work.
  - `hi02c.config`: the same check on the top bank (a 3-sample analog tail), also ~14 minutes.
- `tests/event_kernel.rs`, `iologic_paths_follow_the_board_model`: the ODDR/IDDR/OREG
  sequences, start-up values and an asynchronous reset on a synthetic device (fast suite, CI).
- The model scripts next to the evidence ([hi02b_simulate.py](inventory/LFE5U-85F/hi02_iologic/hi02b_simulate.py))
  check the same register model against every recorded sample.

## Open

- The bottom and right banks were not run; the bottom SIOLOGIC uses the same model as the top.
- Differential electrical qualification: [LVCMOS33D and native differential digital IO execute](PIO_BANKS.md#differential-digital-io-r03-added-2026-11-17); the HP02 board wiring check remains open.
- Inverted CE and LSR (`CEMUX`/`LSRMUX=INV`) on the board, and CE, `LSR_OVER_CE` and
  `GSR=ENABLED` on the SIOLOGIC registers.
- Input and output delays (I06), the gearboxes' own ALIGNWD/SLIP (I04), memory paths (I07).
  CLKDIVF's ALIGNWD is executed as HI04 measured it.
- X2 on the right bank, X2 with a reset, and ALIGNWD's effect on an X2 word.
- The DDR tristate path (`IOLTOMUX=TDDR`).
