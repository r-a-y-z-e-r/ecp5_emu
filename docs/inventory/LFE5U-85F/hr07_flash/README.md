# HR07: the SPI flash on the board, against the emulator

Two read-only reference designs that access the ULX3S's configuration flash, U10 (an ISSI
IS25LP128F), through `USRMCLK`, as [docs/PERIPHERALS_FLASH.md](../../../PERIPHERALS_FLASH.md)
describes. Both ran on the board on 2026-11-16:

- [flash_dump](../../../../examples/ulx3s/flash_dump/top.v): reads the JEDEC ID, status register,
  and two windows (`0x000000`–`0x00FFFF` and `0x200000`–`0x200FFF`), then prints `CRC` over both
  windows in that order over the UART.
- [picorv32_flash](../../../../examples/ulx3s/picorv32_flash/top.v): boots from EBR, checks for an
  `RVFW` header and CRC at `0x200000`, and either runs that firmware in place or falls back to
  printing `no firmware at 200000:` with the 16 bytes found there.

Both designs share [flash_reader.v](../../../../examples/ulx3s/flash_dump/flash_reader.v), whose
hardware can only issue `0x03` READ, `0x9F` RDID and `0x05` RDSR. A plain `cargo test` scans their
Verilog for write, erase and status-write opcodes, and the emulator counted zero write-class
commands in every flash test. The flash was never written, erased or had its status written:
These checks used SRAM loads only.

## Setup

- **Board:** ULX3S "85K v3.0.8" (LFE5U-85), SRAM load with `openFPGALoader -b ulx3s`, under the
  shared board lock.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py) with prefix `""` (every line).
  - `flash_dump`: 1 load, 35 s, at `2026-11-16T16:00:17Z`.
  - `picorv32_flash`: 2 loads, 4 s each, at `16:00:58Z` and `16:01:09Z`.
- **Configurations:** [flash_dump.config](flash_dump.config) and
  [picorv32_flash.config](picorv32_flash.config), built with `examples/ulx3s/build.sh` (nextpnr
  seed 1).
- **Records:** [TOOLS.txt](TOOLS.txt), [SHA256SUMS](SHA256SUMS). The raw UART logs stay in
  `out/flash/` and are not stored in git because they hold content from the board's flash.

## flash_dump observations

- The UART gave 5 153 lines, among them 5 148 dump lines: two complete passes, 4 096 + 128 lines
  each, plus the cut edges of the window.
- Every pass printed `FLASH ID 9D6018 SR 18` and `CRC 055E6742`.
- `9D 60 18` is the JEDEC ID of the IS25LP128F named on the PCB.
- `SR 0x18` has the block-protect bits `BP1` and `BP2` set: this board's flash is write-protected
  over part of its array. The model's status register defaults to `0x00`, and the comparison below
  gives it the board's value (`0x18`).
- The windows are `0x000000`–`0x00FFFF` (64 KB) and `0x200000`–`0x200FFF` (4 KB). The CRC is
  CRC-32/ISO-HDLC over both windows' bytes in that order.

## The comparison

- The dump was turned into a 16 MB image in memory: the two windows, the rest `0xFF`.
- [tests/spiflash.rs](../../../../tests/spiflash.rs) `the_board_dump_reproduces_in_the_emulator`
  ran `flash_dump` in fast mode on that image with `SR 0x18`. It printed exactly the board's lines:
  the ID/SR line, every window line and the `CRC` line.
- Result line on 2026-11-16 (`rserv`): `test result: ok. 1 passed`, 349 824 571 cycles at 4.44 MHz.
- The dump itself stays in `out/flash/` and is not in git, because the board's flash holds whatever
  was last programmed there.

## picorv32_flash observations

- On both loads it printed `no firmware at 200000:` with the 16 bytes found there; the bytes are
  not recorded.
- The boot ROM's banner comes before the serial port opens after a load.
- In the emulator, the demo image holds a firmware at `0x200000`, run in place: 2 passes of
  `primes below 2000: 303, 5849188 cycles` at 0.74 MHz. On the board there is none, since nothing
  is written to its flash.
