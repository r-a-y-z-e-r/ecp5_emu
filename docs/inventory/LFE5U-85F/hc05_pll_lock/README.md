# HC05 slice: EHXPLLL lock time, pre-lock output and frequency ratio

The project's second hardware observation, made on 2026-10-29. It covers part of the clock
questions C06/C07 (PLL lock behaviour) from [CLOCK_NETWORK.md](../../../CLOCK_NETWORK.md) and is a
slice of the planned series **HC05**.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written. Tool versions are in
  [TOOLS.txt](TOOLS.txt) and checksums in [SHA256SUMS](SHA256SUMS).
- **PLL:** `ecppll -i 25 -o 100` ([pll100.v](pll100.v)):
  - `CLKI_DIV 1`, `CLKFB_DIV 4`, `CLKOP_DIV 6`, `FEEDBK_PATH CLKOP`;
  - `CLKOP_ENABLE ENABLED` with `ENCLKOP` tied to 0.
- **Design:** [hc05_lock.v](hc05_lock.v) with [hc05.lpf](hc05.lpf). The observer runs on the
  25 MHz reference, not on the PLL under test. Its flip-flops start from their init values, as
  HS01 established. It reports over the FTDI UART (`ftdi_rxd` = L4, 115200 8N1):
  - `L`: reference cycles until the 2-flop-synchronized LOCK is seen;
  - `P`: PLL output rising edges before LOCK is seen in the PLL domain;
  - `F`: PLL output edges during 65 536 reference cycles;
  - `D`: LOCK drops after the first lock.
- **Runner:** [run.py](run.py) loads the bitstream and reads the serial port. No person is needed
  to observe.

## Observations

All six loads ([observations.log](observations.log)) printed the same line:

```
L=000002 P=000002 F=040000 D=00
```

## Reading

1. **The PLL is locked before user logic starts.** `L=2` and `P=2` are exactly the latency of
   the 2-flop synchronizers. During that time the PLL output already toggled, at the locked rate.
   The lock is acquired while the configuration loads, so the conflict between "16 ms" and
   "16 µs" in the guide does not apply to start-up after configuration with a running reference.
2. **The frequency ratio is exactly ×4.** `F = 262144 = 4 × 65536`. This matches
   `Fout = Fin × CLKFB_DIV × Dfb / (CLKI_DIV × Dout)` with feedback through `CLKOP`.
3. **A static `CLKOP_ENABLE=ENABLED` runs the output with `ENCLKOP=0`.**
4. **No LOCK drop** occurred during the measurement.

**Not covered:**
- relock after `RST` or `STDBY` while running: measured in [HC07](../hc07_pll_reset/README.md);
- `INT_LOCK_STICKY` behaviour;
- output phase between several outputs;
- duty cycle;
- other divider settings;
- power-up from flash.

## Emulator check

[PLL_EXECUTION.md](../../../PLL_EXECUTION.md) builds the emulator's PLL model on this result.
`tests/pll_native.rs` runs **this exact configuration** ([hc05.config](hc05.config)) with a
40 ns reference and decodes the UART from L4. The emulator prints the same line,
`L=000002 P=000002 F=040000 D=00`.
