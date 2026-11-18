# HC06 slice: internal oscillator OSCG, frequency, dividers and start-up

The project's fourth hardware observation, made on 2026-10-29. It belongs to the clock series
in [CLOCK_NETWORK.md](../../../CLOCK_NETWORK.md).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written. Tool versions are in
  [TOOLS.txt](TOOLS.txt) and checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hc06_osc.v](hc06_osc.v) with [hc06.lpf](hc06.lpf), built with `-DOSC_DIV=<n>`. An
  `OSCG` with that `DIV` is measured against the 25 MHz reference. The report goes over the FTDI
  UART (`ftdi_rxd` = L4, 115200 8N1), one line per window, all values in hex:
  - `E`: reference cycles until the first oscillator toggle is seen through a 2-flop synchronizer;
  - `S`: oscillator rising edges in the first window, which starts at reference cycle 0;
  - `W`: the window number;
  - `F`: oscillator rising edges in that window.
  
  A window is 65 536 reference cycles (2.62144 ms) with the gate high, then as many low.
- **Runners:**
  - [run.py](run.py) loads a bitstream and summarises every window it reads;
  - [sweep.py](sweep.py) measures every `OSC.DIV` bit pattern ([patterns.json](patterns.json)) by
    editing only the two `OSC.DIV` lines of the `DIV=2` configuration and repacking it.

  No person is needed to observe.

## Observations

**Per divider** (`observations_div<n>.log`, 2–3 loads each, ~570–760 windows per load):

| DIV | F mean per load | Divider | Base frequency |
|---|---|---|---|
| 2 | 408 604 / 408 794 / 408 627 | 2 | 311.79 MHz |
| 3 | 272 412 / 272 446 / 272 496 | 3 | 311.80 MHz |
| 4 | 204 342 / 204 364 / 204 332 | 4 | 311.81 MHz |
| 5 | 163 492 / 163 532 | 5 | 311.9 MHz |
| 8 | 102 234 / 102 184 | 8 | 311.9 MHz |
| 16 | 51 088 / 51 097 / 51 105 | 16 | 311.87 MHz |
| 32 | 25 562 / 25 565 | 32 | 312.0 MHz |
| 33 | 25 562 / 25 564 | **32** | — |
| 64 | 12 784 / 12 782 | 64 | 312.1 MHz |
| 100 | 8 180 / 8 178 | 100 | 312.0 MHz |
| 127 (= 128) | 6 385 / 6 385 / 6 386 | **128** | 311.78 MHz |

**Sweep** ([sweep.log](sweep.log), [sweep.json](sweep.json)): all 63 bit patterns, one load each.
`DIV=2` was measured before and after the sweep and gave 311.97 MHz both times. Every pattern
gives an integer divider within 0.11%; neighbouring dividers differ by at least 0.8%.

**Start-up:**
- `E` = 2–13 reference cycles over all loads;
- `S` equals the steady `F` within the spread.

## Reading

1. **The base frequency belongs to the chip:** 311.8 MHz in the first hour, 311.97 MHz an hour
   and a half later. The nominal value is 310 MHz (nextpnr's timing model), so this chip runs
   0.58–0.64% fast.
2. **Within a load the frequency wanders.** σ is about 0.03% of a 2.6 ms window, extremes reach
   ±0.1%, and means differ by ~0.05% between loads.
3. **The divider depends only on the `OSC.DIV` bit pattern**, and the pattern loses resolution
   above 31:

   | Values | Divider applied by the chip |
   |---|---|
   | 2–31 | the value |
   | 32–63, in pairs (32–33, 34–35, …) | the even one: 32, 34, …, 62 |
   | 64–66 | 64 |
   | 67–126, in fours (67–70, …, 123–126) | the multiple of 4: 68, 72, …, 124 |
   | 127 | 128 |

   So `DIV=33` gives ÷32, and a requested `DIV=127` gives ÷128. `DIV=128`, the primitive's
   default, is written as 127 by nextpnr and gives ÷128 as intended.
4. **The oscillator runs before user logic starts.** `E` is only the synchronizer plus an
   arbitrary phase, and the first window already has the full count.

**Not covered:**
- the duty cycle;
- other chips or temperatures;
- power-up from flash;
- `SEDSTDBY`.

## Emulator check

[OSC_EXECUTION.md](../../../OSC_EXECUTION.md) builds the emulator's oscillator on this result.
- `tests/oscillator_native.rs` runs [hc06_16.config](hc06_16.config) and
  [hc06_127.config](hc06_127.config) with the base frequency 311.8 MHz. The emulator's `F` is
  within 0.1% of every load's mean, and `S` and `E` lie in the board's range.
- `tests/oscillator_dividers.rs` derives the planner's divider table from
  [sweep.json](sweep.json).
