# HI02 slice: IOLOGIC DDR X1 and IO registers

The project's third hardware observation, made on 2026-10-29. It answers part of question I02
(exact publication of RXDATA, latency in clock cycles, resets) from
[IOLOGIC_RESOURCES.md](../../../IOLOGIC_RESOURCES.md), for the left-side IOLOGIC and the
top-bank SIOLOGIC.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written. Tool versions are in
  [TOOLS.txt](TOOLS.txt) and checksums in [SHA256SUMS](SHA256SUMS).
- **Clocks:** `sclk = 25 MHz / 16`, the top bit of a 4-bit counter on the 25 MHz reference,
  placed by nextpnr on a global net. All primitives under test run on `sclk`. The recorders
  sample at 25 MHz, so every half `sclk` period holds 8 samples.
- **Stimulus:** 7-bit LFSRs. Data for outputs comes from LFSRs in the `sclk` domain. Inputs are
  driven by the fabric onto the pad through the same `BB` and change only at 25 MHz edges
  between `sclk` edges, so every capture is unambiguous.
- **Pins, left bank (HI02, HI02b):** ULX3S LEDs, each in a different PIO pair so that no pair
  shares IOLOGIC bits: `B2` (R35C0 PIOC), `D1` (R38C0 PIOB), `D2` (R38C0 PIOC),
  `E2` (R44C0 PIOD) and `H3` (R41C0 PIOD).
- **Pins, top bank (HI02c):** the ULX3S GPDI (HDMI) pins, with nothing connected to the HDMI
  port: `A16` (`gpdi_dp[0]`, R0C110 PIOA), `A14` (`gpdi_dp[1]`, R0C83 PIOA),
  `A12` (`gpdi_dp[2]`, R0C76 PIOA), `A17` (`gpdi_dp[3]`, R0C116 PIOA) and
  `B16` (`gpdi_dn[0]`, R0C110 PIOB). The top SIOLOGIC A and B have no shared bits. The pin
  names are confirmed in the official `ulx3s_v20.lpf`.
- **Report:** a line over the FTDI UART (`ftdi_rxd` = L4, 115200 8N1), read by [run.py](run.py).
  No person is needed to observe.

Three designs:

1. [hi02_ddr.v](hi02_ddr.v) with [hi02.lpf](hi02.lpf), steady state.
   - `ODDRX1F` on `B2`, `D0`/`D1` from two LFSRs, read back through its `BB`.
     The report holds 128 samples of `{sclk, pad}`.
   - `IDDRX1F` on `D1`. The report holds `{Q0, Q1}` for 32 `sclk` cycles.
2. [hi02b.v](hi02b.v) with [hi02b.lpf](hi02b.lpf), start-up, reset and IO registers. A 6-bit sample
   `{oddr pad, Q0, Q1, oreg pad, tsreg pad, ireg Q}` is taken on every 25 MHz edge in two
   windows: from the first edge after configuration (`t = 0..127`) and around a reset pulse
   (`t = 1008..1263`).
   - The reset `rst` drives `RST` of both DDR primitives and the asynchronous set of the OREG.
     It rises in the middle of a low phase and falls in the middle of a high phase.
   - OREG, TSREG and IREG are flip-flops with `(* syn_useioff *)`, all with init value 1.
     nextpnr writes `REGSET=SET`, `SRMODE=ASYNC`, `GSR=DISABLED`.
   - The TSREG pin has `PULLMODE=DOWN` and drives a constant 1, so a released pad reads 0.
3. [hi02c.v](hi02c.v) with [hi02c.lpf](hi02c.lpf): the HI02b design, only renamed and moved to the
   top-bank pins.

## Observations

Every load printed the same line: 4 of 4 for each design
([hi02_observations.log](hi02_observations.log), [hi02b_observations.log](hi02b_observations.log),
[hi02c_observations.log](hi02c_observations.log)).

```
O=5555AAAA5555FFFF5555AAAA5555AAAA0000FFFF5555FFFF5555AAAA0000FFFF I=DADEC697732AFE04
```

[hi02_analyse.py](hi02_analyse.py) searches the order and latency hypotheses against the
reproduced LFSR sequences. Exactly one hypothesis fits each signal.
[hi02b_simulate.py](hi02b_simulate.py) runs the explicit register model below over all
384 × 6 samples of the second design and finds no mismatch. It finds none for the third design
either. HI02c differs from HI02b only in the TSREG pad after a release (item 6).

## Reading

1. **ODDRX1F.**
   - `D0` (`TXDATA0`) is on the pad during the high phase of `SCLK`, and `D1` (`TXDATA1`) during
     the following low phase.
   - A pair captured at a rising edge appears two rising edges later. In the high phase after
     rising edge *r* the pad shows `D0` from edge *r−2*, and in the low phase after it `D1` from
     edge *r−2*.
   - The pad changes only at `SCLK` edges, at the 40 ns resolution of the recorder.
2. **IDDRX1F.** `Q0` (`RXDATA0`) is the pad at a rising edge and `Q1` (`RXDATA1`) is the pad at
   the following falling edge. Both are published together at the next rising edge.
3. **Start-up.**
   - The DDR stages start at 0 after configuration with `GSR=DISABLED`. The ODDR pad is 0 until
     the first captured pair arrives, and Q0/Q1 are 0 until the first published pair.
   - The IO registers start at their REGSET value (`SET` → 1). This matches the PLC flip-flops in
     HS01.
4. **Asynchronous reset** (`LSR`, `SRMODE=ASYNC`).
   - Within the same 40 ns sample, reset forces the ODDR pad and Q0/Q1 to 0 and the OREG pad to
     its REGSET value 1.
   - All DDR stages are cleared. After release, capture restarts at the first edge: the ODDR pad
     shows 0 until data captured after release arrives two edges later, and Q0 lags Q1 by one
     cycle as in a fresh start.
5. **IO registers.** OREG, TSREG and IREG are ordinary rising-edge flip-flops with a latency of
   one edge. The IREG samples the pad at the `SCLK` rising edge.
6. **Analog side effect, not digital behaviour.**
   - When the TSREG releases the pad, the readback stays high for 14 samples (≈560 ns) before it
     reads 0. The pad discharges slowly through the pull-down and the LED; the delay was the same
     in all six releases.
   - When the TSREG enables the driver, the readback changes at the edge.
   - On the unloaded HDMI pin (HI02c) the same release reads high for 3 samples (≈120 ns), which
     confirms that the tail depends on the load, not on the logic.
   - The emulator has no analog model; a released pad with a pull-down reads 0 at once.
7. **Top-bank SIOLOGIC (HI02c).** Every digital signal is identical to the left-bank run, sample
   for sample: the same DDR order and latency, start-up values, reset and IO registers.

**Not covered:**
- the bottom bank and the right bank (only left and top were run);
- `CE` of the IO registers (`CEIMUX`/`CEOMUX=CEMUX`);
- `SRMODE=LSR_OVER_CE` (synchronous reset);
- `GSR=ENABLED`;
- `REGSET=RESET` (only `SET` was loaded; `RESET` → 0 is the plain reading of the setting);
- inverted clocks and resets (`CLK*MUX=INV`, `LSRMUX=INV`);
- the input/output delay (`DELAY.*`);
- X2/7:1 gearing and the memory modes;
- power-up from flash.

## Emulator check

[IOLOGIC_EXECUTION.md](../../../IOLOGIC_EXECUTION.md) builds the emulator's IOLOGIC model on
this result. `tests/iologic_native.rs` runs **these exact configurations**
([hi02.config](hi02.config), [hi02b.config](hi02b.config), [hi02c.config](hi02c.config)) with a
40 ns reference and decodes the UART from L4.
- HI02: the emulator prints the board's line exactly.
- HI02b: every digital signal matches the board sample for sample. The TSREG pad differs in
  90 samples, all inside the analog discharge windows described in item 6: 14 samples after each
  of the six releases, and the start of the second window. The test checks exactly that.
- HI02c: the same check with a 3-sample tail.
