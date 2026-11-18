# ULX3S reference designs

Small designs of the kind a student writes first, for the ULX3S (LFE5U-85F, CABGA381). Each one
builds with the open toolchain and runs in the emulator, with its signals named as in the Verilog.
`blinky_pll`, `uart_hello`, `debounce` and `ddr_pattern` are the exit set of Stage 1 in
[ROADMAP.md](../../ROADMAP.md); `uart_ram` adds block RAM, and `picorv32_console` is the first soft
CPU, for Stage 3.

| Design | What it shows | Board |
|---|---|---|
| [blinky_pll](blinky_pll/top.v) | `ecppll` 25 → 100 MHz, a 32-bit counter, its top byte on the LEDs | PLL behaviour measured in [HC05](../../docs/inventory/LFE5U-85F/hc05_pll_lock/README.md) |
| [blinky_cycle](blinky_cycle/top.v) | an 8-bit counter on the 25 MHz clock, no PLL, on the LEDs: `led[0]` toggles on every clock, for watching single cycles in the GUI | — |
| [blinky_sv](blinky_sv/top.sv) | the 8-bit LED counter in SystemVerilog using `logic` and `always_ff`, with the ordered HDL project flow and original source probing | — |
| [blinky_vhdl](blinky_vhdl/README.md) | the 8-bit LED counter in VHDL-2008 through the suite GHDL plugin; emulation and Waves work, source probing unavailable | — |
| [sv_package](sv_package/README.md) | package/import SystemVerilog with an explicit slang pin and original source locations | — |
| [audio_tone](audio_tone/top.v) | Output-only 440/660 Hz four-bit sines at 25 kHz, codes 4..12; V=8, sample count and CRC-32 on UART every second | [HR11](../../docs/inventory/LFE5U-85F/hr11_audio/README.md): first three UART lines per load match the emulator; analog signals unmeasured; [model and method](../../docs/PERIPHERALS_AUDIO.md) |
| [oled_pattern](oled_pattern/top.v) | SSD1331 OLED on 6.25 MHz SPI: a scrolling colour gradient and moving box; frame number and data CRC32 on the 115200 UART | [HR04](../../docs/inventory/LFE5U-85F/hr04_hdmi_oled/README.md): frames 3–219, every line the same as the emulator's, 3 of 3 loads; the panel not observed |
| [switches](switches/top.v) | Four input-only DIP switches on LEDs and 115200 UART, sw[0] first; 10 ms debounce and 1 s repeat; DOWN and UP constraint variants | [HR10](../../docs/inventory/LFE5U-85F/hr10_switches/README.md): all twelve lines SW 1111 with DOWN and UP, matching all ON; [method](switches/README.md) |
| [header_probe](header_probe/top.v) | Board-safe input-only J1/J2 pulls and four internal PWM period counters on UART; [driven variant](header_probe/driven/README.md) reads GP0..GP3 back through pad buffers at DRIVE=4 | [HR13](../../docs/inventory/LFE5U-85F/hr13_header/README.md): both variants match the emulator, including four driven-pad counts and 56 input pulls; startup capture offset explained; [model and method](../../docs/PERIPHERALS_HEADER.md) |
| [adc_read](adc_read/top.v) | MAX11125 manual SPI, eight safe input-only header pulls, 12-bit codes at 5 Hz on UART; channel 0 on LEDs | [HR08](../../docs/inventory/LFE5U-85F/hr08_adc/README.md): pulls read through the board's ADC: within 30 codes of the emulator |
| [rtc_read](rtc_read/top.v) | Read-only MCP7940N I2C, registers and 64-byte SRAM at 1 Hz on UART; open-drain SCL/SDA, no data-write state | [HR09](../../docs/inventory/LFE5U-85F/hr09_rtc/README.md): the emulator reproduces the board's read after the reserved-byte fix; [model and method](../../docs/PERIPHERALS_RTC.md) |
| [edid_read](edid_read/top.v) | Read-only monitor EDID at I2C 0x50, base-block hex and checksum at 1 Hz on UART; copied hdmi_bars DVI source, open-drain DDC, no data-write state | [HR12](../../docs/inventory/LFE5U-85F/hr12_edid/README.md): nine EDID nack lines, matching the disconnected model; real monitor bytes stay in out/edid/; [model and method](../../docs/PERIPHERALS_EDID.md) |
| [sd_read](sd_read/top.v) | SDHC micro-SD card in SPI mode: 80-clock wake and init (CMD0, CMD8, ACMD41, CMD58) at ~391 kHz, block 0 read at 6.25 MHz; OCR, first 16 bytes and bytes 510–511 on the 115200 UART | [HR05](../../docs/inventory/LFE5U-85F/hr05_sd_card/README.md): a real card read 3 of 3 loads, its OCR `C0FF8000` the emulator's |
| [sd_native_read](sd_native_read/top.v) | Read-only SD-bus initialization at 390.625 kHz, four-bit block 0 at 6.25 MHz; OCR/RCA/CSD, separate CID, block CRC32/CRC16 and raw block on UART | [HR14](../../docs/inventory/LFE5U-85F/hr14_sd_native/README.md): ten reports with valid four-bit CRC16, private-block replay PASS; CID and raw block stay in `out/sd/` only; [model and method](../../docs/PERIPHERALS_SD.md#native-reference-and-private-board-method) |
| [sdram_test](sdram_test/top.v) | the 32 MB SDRAM (MT48LC16M16A2) from a plain-Verilog controller at 25 MHz, CL 2: an LFSR pattern over 64 KB written and read back, `SDRAM W8000 E0000` (words checked, errors) on the UART | [HR06](../../docs/inventory/LFE5U-85F/hr06_sdram/README.md): the same line, 3 of 3 loads |
| [picorv32_sdram](picorv32_sdram/top.v) | PicoRV32 running its firmware from SDRAM: a boot ROM in EBR copies it there and jumps; each pass prints the primes below 2000 with the cycles they took, then a 256 KB memtest | [HR06](../../docs/inventory/LFE5U-85F/hr06_sdram/README.md): the emulator's cycle counts exactly (854997, 855452), 0 memtest errors, 3 of 3 loads |
| [uart_hello](uart_hello/top.v) | a UART transmitter: `Hello, ULX3S!` at 115200 8N1, ten times per second | run on the ULX3S: the host read exactly this message, 3 of 3 loads |
| [debounce](debounce/top.v) | `fire1` (FIRE1, the ULX3S's btn[1]) presses counted on the LEDs after 65 536 stable cycles (2.62 ms at 25 MHz); hold at a fast window rate or use the stimulus below | — |
| [uart_ram](uart_ram/top.v) | the message in block RAM: a ROM ([message.hex](uart_ram/message.hex)) copied into a 512×16 RAM at start and sent from there, both EBR | — |
| [picorv32_console](picorv32_console/top.v) | the stock [PicoRV32](picorv32_console/picorv32.v) (RV32I) with 8 KB of block RAM in four EBR, its register file in LUTRAM, a UART and the LEDs; the [firmware](picorv32_console/firmware/main.c) counts the primes below 2000, prints the count and the cycles it took, then echoes a console in capitals | HR01: same text as the emulator, cycle count included |
| [life](life/top.v) | Conway's Game of Life on a 27×27 torus: every cell a flip-flop, a generation of all of them every clock, the population divided by 8 on the LEDs; holding FIRE1 loads the start pattern again. Every 16th generation an LFSR turns over a few random cells, so the grid keeps changing instead of settling into still lifes and blinkers in a few hundred generations. About 10 850 LUT4 and 775 FF, 13% of the chip: the largest design here | — |
| [life_full](life_full/top.v) | The same Life on a 72×72 torus filling the chip, the population divided by 32 on the LEDs: about 78 700 LUT4 (94% of the chip) and 5 230 FF, the design that tells whether the emulator and the map keep up at full scale. nextpnr takes about 40 minutes and reaches 23.8 MHz on the population count to the LEDs (below 25: for the board that path wants a pipeline stage) | — |
| [ram32](ram32/top.v) | A 512×32 ROM ([rom.hex](ram32/rom.hex)) and RAM as yosys infers them: each one EBR read 36 bits at a time (PDPW16KD, `DATA_WIDTH_R=36`). The RAM is filled with an LFSR sequence and read back; the UART prints `ROM C9083F00 RAM 0000` (the ROM sum, the words read back wrong) ten times a second, and led[7] lights on an error | HR02: the same line, 3 of 3 loads ([hr02_ram32_mac](../../docs/inventory/LFE5U-85F/hr02_ram32_mac/README.md)); the order of the two 18-bit halves measured in HB01 |
| [mac](mac/top.v) | `*` in the DSP blocks: a signed 16×16 dot product over 4096 LFSR pairs and an unsigned 32×32 product (yosys splits it over four MULT18X18D), seven multipliers in all. The UART prints `MAC 003D16367D MIX CE196146` after every pass | HR02: the same line, 3 of 3 loads: the first board result for the DSP blocks |
| [ddr_pattern](ddr_pattern/top.v) | four GPDI lanes as in an HDMI transmitter: `ODDRX1F` into `LVCMOS33D` pairs, TMDS control symbols and the clock lane | `ODDRX1F` measured in [HI02](../../docs/inventory/LFE5U-85F/hi02_iologic/README.md); `LVCMOS33D` from the vendor document |
| [hdmi_bars](hdmi_bars/top.v) | 640×480 DVI on the GPDI (HDMI) connector: eight colour bars, 800×525 at 25 MHz (59.52 Hz). One PLL gives the 25 MHz pixel and 125 MHz shift clocks; three TMDS encoders; an `ODDRX1F` per lane sends two bits per shift clock. The emulator decodes the pads into frames ([PERIPHERALS.md](../../docs/PERIPHERALS.md)) | — |
| [hdmi_moving](hdmi_moving/top.v) | the `hdmi_bars` pipeline with the bars scrolling and a square bouncing, so every frame differs; the frame number on the LEDs, and after each frame `F0001 CD022261D` on the UART: its number and the CRC-32 of its pixels, which the emulator's decoded frame must match | [HR04](../../docs/inventory/LFE5U-85F/hr04_hdmi_oled/README.md): frames 4–243, every line the same as the emulator's, 3 of 3 loads; the HDMI pads themselves not observed |
| [flash_dump](flash_dump/top.v) | Read-only IS25LP128F ID/status, two flash windows and CRC over UART; USRMCLK at 12.5 MHz | [HR07](../../docs/inventory/LFE5U-85F/hr07_flash/README.md): ID 9D6018; the emulator reproduces the board's dump and CRC |
| [picorv32_flash](picorv32_flash/top.v) | EBR boot validates RVFW/CRC at 200000, then XIP prime firmware through read-only SPI; missing-header fallback | [HR07](../../docs/inventory/LFE5U-85F/hr07_flash/README.md): the read-only fallback on the board; XIP in the emulator |

The constraint files follow the official `ulx3s_v20.lpf`: the same port names (`clk_25mhz`, `led`,
`btn`, `ftdi_rxd`, `gpdi_dp`) and IO settings.

## Build

```sh
examples/ulx3s/build.sh examples/ulx3s/uart_hello target/reference/uart_hello
```

[build.sh](build.sh) runs Yosys, then nextpnr with `--write top_routed.json` (the routed design that
carries the HDL names), then ecppack. The bitstream `top.bit` can be loaded with
`openFPGALoader -b ulx3s top.bit`.

A design whose memory holds a firmware lists the images in `ecpbram.txt` (`picorv32_console`
does). `build.sh` then places and routes it with random placeholders of the same size
(`top_placeholder.config`, made with `ecpbram -g`). Then [swap.sh](swap.sh) puts the images in:
ecpbram replaces each placeholder, and ecppack writes `top.bit`. ecpbram finds a memory by the
bit patterns of its old content, so the old content must be random. A real firmware, with its
runs of zeros, gives "Conflicting from pattern".

## Run in the emulator

`--names` takes the routed design, and `net:NAME` then refers to a signal of the Verilog. A bus
name without an index expands to all its bits.

```sh
ecp5_emu emulate target/reference/uart_hello/top.config \
  --names target/reference/uart_hello/top_routed.json \
  --startup configured --scope observed --clock net:clk_25mhz --cycles 3 \
  --probe net:ftdi_rxd --probe net:led
```

```text
6 ns: ftdi_rxd=0 led[0]=1 led[1]=0 led[2]=1 led[3]=0 led[4]=0 led[5]=1 led[6]=1 led[7]=0
```

`led` is 0x65, the `e` after the `H` that is already on the line.

**Button.** Only `fire1` (FIRE1, the ULX3S's btn[1]) is used, on pin R1 (`net:fire1` in a stimulus).
A press must stay stable for 65 536 clock cycles (2.62144 ms of board time at 25 MHz),
plus a few cycles for synchronising the input and counting the press on the LEDs.
In the window, the wait depends on the actual
emulation rate: at **as fast as possible** (`max`), this tiny design's window runner measured
about 63 MHz here, a wait of about **1 ms** (your computer and window activity affect the rate);
at 1 MHz it takes about **65.5 ms**, at 10 kHz about **6.55 s**,
and at 4 cycles/s about **4 hours 33 minutes**. A quick click at a slow rate will not count.
Run at a fast rate and hold FIRE1 until the LED count changes; release it for the same stable
interval before pressing again. Or use the command line with a bouncy press from
[debounce/press.json](debounce/press.json):

```sh
ecp5_emu emulate target/reference/debounce/top.config \
  --names target/reference/debounce/top_routed.json \
  --startup configured --scope observed --clock net:clk_25mhz --cycles 70000 \
  --stimulus examples/ulx3s/debounce/press.json \
  --probe net:level --probe net:led --vcd debounce.vcd > /dev/null
```

The CLI clock has a 2 ns period by default. After the last bounce at 1 700 ns, `level` rises
at 132 775 ns (65 536 stable cycles plus the synchroniser); the LED count becomes 1 at
132 777 ns.

**At board speed.** `--fast` runs the design compiled to C ([FAST_MODE.md](../../docs/FAST_MODE.md))
and `--period-ns 40` gives the board's 25 MHz clock. One second of `blinky_pll`:

```sh
ecp5_emu emulate target/reference/blinky_pll/top.config \
  --names target/reference/blinky_pll/top_routed.json --fast \
  --startup configured --scope observed --clock net:clk_25mhz --period-ns 40 --cycles 25000000 \
  --probe net:led --probe net:locked
```

```text
0 ns: led[0]=0 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0 locked=1
1000000000 ns: led[0]=1 led[1]=0 led[2]=1 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0 locked=1
```

The counter runs at the PLL's 100 MHz: 10⁸ − 6 counts make `led` 5. This takes about 2 s on an
M3 Max; `uart_hello` and `ddr_pattern` run faster than the board.

## A soft CPU

`picorv32_console` runs a C firmware on the stock PicoRV32 core (ISC licence, copied unchanged
from YosysHQ). The memory map is in [top.v](picorv32_console/top.v): 8 KB of RAM at 0, the UART
at `0x0200_0000` and the LEDs at `0x0300_0000`.

**Firmware.** [firmware/build.sh](picorv32_console/firmware/build.sh) compiles `start.S`, `main.c`
and `lib.c` for `rv32i`. It links them at 0 and writes the image as four byte lanes,
`fw0.hex` … `fw3.hex`, one EBR each. It needs two tools:
- a clang with the RISC-V target (Homebrew's `llvm` has one; Apple's clang does not);
- `ld.lld`, or the `rust-lld` that ships with Rust.

The design is placed and routed once. After that, a new firmware is swapped in, in about a
second:

```sh
examples/ulx3s/build.sh examples/ulx3s/picorv32_console target/reference/picorv32_console   # once
examples/ulx3s/picorv32_console/firmware/build.sh
examples/ulx3s/swap.sh examples/ulx3s/picorv32_console target/reference/picorv32_console
```

The hex files are committed, so building the design needs no RISC-V toolchain.

The swap was checked three ways.
- **The same bitstream.** With the committed firmware, the swapped `top.bit` is byte for byte the
  one a build with the firmware in place gives. ecpbram repeats tile names in the `.tile_group`
  lines of `top.config` on every pass; ecppack and the emulator read a repeat as the same tile.
- **A new firmware.** One with `LIMIT 5000` and another banner printed `primes below 5000: 669,
  474901 cycles` in the emulator's console, as Verilator does on the RTL.
- **No recompilation.** Fast mode keeps the memory contents out of its C source, so the new
  firmware reused the compiled program: compiled in 0.09 s.

**In the emulator.** About 13 ms of the board take about a fifth of a second in fast mode, which
reaches the console prompt:

```sh
ecp5_emu emulate target/reference/picorv32_console/top.config \
  --names target/reference/picorv32_console/top_routed.json --fast \
  --startup configured --scope observed --clock net:clk_25mhz --period-ns 40 --cycles 320000 \
  --probe net:ftdi_rxd --probe net:led --vcd console.vcd
```

```text
Event kernel ran the first 127 cycles, until every register and latch fast mode reads was known
Fast mode: 2404 gates (1334 on demand), 637 FF, 128 LUTRAM columns, 0 IOLOGIC, 4 EBR, 0 PLL, 0 OSC; compiled in 2.75 s
12800000 ns: ftdi_rxd=1 led[0]=1 led[1]=0 led[2]=0 led[3]=0 led[4]=0 led[5]=0 led[6]=0 led[7]=0
Fast mode ran 319873 cycles (0.012795 s of board time) in 0.185 s: 1.73 MHz
```

The UART line in `console.vcd` decodes to `PicoRV32 on ECP5`, then
`primes below 2000: 303, 151493 cycles` and the prompt `> `. An RTL simulation of `top.v` with
iverilog gives the same text, cycle count included.

**The console.** `--console-out` puts a terminal on the UART: what the firmware sends appears in
the terminal, and what you type goes to the receiver. It runs until Ctrl-C:

```sh
ecp5_emu emulate target/reference/picorv32_console/top.config \
  --names target/reference/picorv32_console/top_routed.json --fast \
  --startup configured --scope observed --clock net:clk_25mhz --period-ns 40 \
  --console-out net:ftdi_rxd --console-in net:ftdi_txd
```

```text
PicoRV32 on ECP5
primes below 2000: 303, 151493 cycles
> HELLO
>
```

The status lines go to stderr, so the console text alone can be redirected. At 1.9 MHz the
emulated UART carries about 880 characters per second, a thirteenth of the board's.

**On the board:** load `openFPGALoader -b ulx3s target/reference/picorv32_console/top.bit` and
then open a terminal at 115200 baud on `/dev/cu.usbserial-…`. Typing echoes in capitals, and Enter
gives a new prompt.

The banner is printed once, about 12 ms after the load, and the terminal cannot be open by then:
the FT231X carries both JTAG and the UART, so openFPGALoader cannot load while the port is open.
To see the cycle count, the HR01 run used a firmware that waits for a key before the banner. It
was swapped in with `swap.sh`. On the board it printed `151485 cycles` in 3 loads out of 3, the
same text the emulator printed for that bitstream beforehand
([HR01](../../docs/inventory/LFE5U-85F/hr01_picorv32_console/README.md)).

## Names that are not there

Yosys keeps the names of signals it does not optimise away. In `debounce`, the counter `presses`
drives the port `led` directly, so it survives only as `led`. Look the names up in
`top_routed.json` or in the error, which lists similar names.

## Tests

`tests/reference_designs.rs` (`--ignored`, ~55 s) builds all six designs from these folders and
checks them by name:

| Design | Check |
|---|---|
| `blinky_pll` | `count` is exactly 4n − 6 after n reference cycles: the PLL starts at the second reference edge and runs at ×4; `led` is its top byte |
| `uart_hello` | `ftdi_rxd` decodes to the message the board printed |
| `uart_ram` | two EBR; `ftdi_rxd` decodes to `Hello from block RAM!`, and `led` ends at 22, the index read back with the last character |
| `debounce` | two bouncy presses count 2; a 1 ms pulse does not count |
| `picorv32_console` | four EBR and 128 LUTRAM columns; the program counter follows an RTL simulation of `top.v` (iverilog) cycle for cycle through the reset and the clearing of `.bss`. The event kernel is too slow to reach the console; `tests/fast_mode.rs` does that in fast mode, and through the CLI's `--console-out`/`--console-in`. It also swaps in a firmware with another banner through `swap.sh`, and that firmware runs on the program compiled for the first |
| `ddr_pattern` | lanes 0–2 carry `1101010100` bit 0 first, lane 3 five ones and five zeros aligned to the symbol, and every complement pin is the inverse of its lane |
