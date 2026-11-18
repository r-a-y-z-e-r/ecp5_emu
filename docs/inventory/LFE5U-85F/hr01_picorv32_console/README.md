# HR01: picorv32_console on the board, against the emulator

The first run of a whole reference design on the board, made on 2026-11-08. The design is the
soft CPU of [examples/ulx3s/picorv32_console](../../../../examples/ulx3s/picorv32_console/top.v):
the stock PicoRV32 with 8 KB of EBR, its register file in LUTRAM, a UART and the LEDs. HR is a
new series: whole reference designs compared with the emulator, the hardware loop of Stage 2 in
[ROADMAP.md](../../../../ROADMAP.md).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written. Tool versions are in
  [TOOLS.txt](TOOLS.txt) and checksums in [SHA256SUMS](SHA256SUMS).
- **Runner:** [run.py](run.py) loads a bitstream, then opens the serial port, types the given
  text and prints what comes back. No person is needed to observe.
- **The serial port cannot be open during the load.** JTAG and the UART share the FT231X. With
  the port open, openFPGALoader cannot claim the device and loads nothing. So the port opens after
  configuration. The committed firmware prints its banner once, about 12 ms after configuration,
  and that banner is missed.
- **Two bitstreams:**
  - **official:** `target/reference/picorv32_console/top.bit`, the committed firmware;
  - **variant:** the same placed-and-routed design with a firmware that waits for a key before the
    banner ([variant/key_first.patch](variant/key_first.patch)). It was swapped in with
    `examples/ulx3s/swap.sh` from [variant/fw0.hex](variant/fw0.hex) … `fw3.hex`. The sieve
    still runs first, and the banner reports the cycles it took.
- **Prediction:** before the board run, the emulator ran the variant's `top.config`
  (`emulate --fast --console-out net:ftdi_rxd --console-in net:ftdi_txd`) with the same keys.

## Observations

[observations.log](observations.log) has all of them.

- **Official:** `hi` CR gives `HI\r\n> `, and `Hello, ULX3S` CR gives `HELLO, ULX3S\r\n> `, as in
  the emulator.
- **Variant, three loads:** each printed, after the key `x`:

  ```
  PicoRV32 on ECP5\r\nprimes below 2000: 303, 151485 cycles\r\n> 
  ```

  and the same echoes. The emulator printed exactly this text, `151485` included, before the board
  ran.

## Reading

1. **The emulator is cycle-exact with the chip on a soft CPU.** The firmware reads `rdcycle` around
   the sieve, so `151485` counts every cycle of the sieve below 2000. That covers the EBR reads and
   writes, the LUTRAM register file, the carry chains of the ALU and the branch logic. The
   emulator's count is the board's.
2. **The count belongs to the bitstream, not to the C source.** The committed firmware gives
   `151493` in the emulator, Verilator and iverilog. The variant's extra `get_char` call made
   clang lay out `main` differently, 8 cycles fewer. That is why the prediction was made for the
   variant itself.
3. **The swap is real on the chip.** The variant came from `swap.sh` without place and route, and
   it ran as the emulator predicted from the same `top.config`.
4. **Echo and the receiver work on both bitstreams.** The keys reach the design's receiver from
   the FTDI on the board, and from `--console-in` in the emulator, with the same result.

Not covered: power-up from flash, and the banner of the committed firmware on the board. A host
that keeps the port open during the load would show the banner; this one cannot.
