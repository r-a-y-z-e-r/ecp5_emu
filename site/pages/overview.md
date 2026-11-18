# What it does in detail

See the [introduction](introduction.md) for the project goal and execution model,
[verification](verification.md) for evidence boundaries, and [the window guide](window-guide.md)
for the interface.

## What is unusual about it

Tools that start from a bitstream exist for several FPGA families: IceStorm's `icebox_vlog`
(iCE40), Apicula's `gowin_unpack` (Gowin), F4PGA's `fasm2bels` (Xilinx 7-series) and an ECP5
bitstream decompiler. They turn the configuration back into a Verilog netlist of the used logic
and hand it to an external simulator such as Icarus Verilog or Verilator. Die viewers exist too
(nextpnr's GUI draws the ECP5 die), and so do virtual boards (Digital, DigitalJS, the Makerchip
Virtual FPGA Lab), but those simulate the design's HDL, not the placed and routed chip. In a
survey on 2026-11-16 ([related work](related-work.md)) we found no tool that executes
an ECP5 bitstream directly, and none that shows live values on the routed die. What ecp5_emu
does differently:

- **It executes the bitstream, not the HDL or a netlist.** The input is the configuration
  memory that the chip itself would receive. Behaviour comes from the bits; the HDL names
  from nextpnr's routed JSON are only an overlay for display and probing
  ([HDL_NAMES.md](../../docs/HDL_NAMES.md)).
- **The whole die is modelled**, used or not: 14 231 tiles of 148 types, 53 690 resources and
  28 million route descriptions over all tile placements of the LFE5U-85F
  ([CORE_STATUS.md](../../docs/CORE_STATUS.md), [STRUCTURAL_MODEL.md](../../docs/STRUCTURAL_MODEL.md)).
- **Two executors, one plan.** An exact event kernel with 0/1/X/Z values and delta waves can
  explain every transition. A fast mode compiles the configured fabric to C and runs it on all
  cores. They are checked against each other on identical stimuli
  ([EVENT_KERNEL.md](../../docs/EVENT_KERNEL.md), [FAST_MODE.md](../../docs/FAST_MODE.md)).
- **Behaviour is measured on a real board where the documents leave it open.** 36 experiment series
  on a ULX3S v3.0.8 (LFE5U-85F, JTAG IDCODE `0x41113043`) pinned down PLL lock, reset and standby,
  the internal oscillator's divider table, IOLOGIC DDR and gearboxes, IO delays and latches, block
  RAM read order, start-up and read-before-write at widths 1, 2, 4, 9 and 18, clock gating, the
  ALU54B operation table, equal-width and pseudo-dual-port block RAM collisions, DSP capture edges
  with dividers disabled, slice preload, DTR conversion sequences and restart, and DCS protected
  clock switching, with the raw records kept in [docs/inventory](../../docs/inventory/LFE5U-85F)
  ([HARDWARE_VALIDATION.md](../../docs/HARDWARE_VALIDATION.md)). Whole designs are compared too: the
  PicoRV32 console, block RAM and DSP designs, the gallery demos, HDMI and OLED designs, the SDRAM
  test, PicoRV32 running from SDRAM and `flash_dump` print the same lines on the board as in the
  emulator (HR01–HR04, HR06–HR07). The flash comparison uses the board's image and status register
  in the emulator. The ADC design reads the header pins pulled up and down and gave board codes
  within 30 of the emulator's 0 and 4095; its 32-code bound (0.8% of full scale) was measured on
  this board, not taken from the datasheet (HR08). The read-only `rtc_read` design read 32 RTC
  registers and 64 SRAM bytes on the board; using them as its image, the emulator printed the same
  register and SRAM lines after fixing an error found by the comparison: reserved registers 0x09,
  0x10 and 0x17 now return the chip's bytes; the board's oscillator was stopped, so timekeeping is
  checked only in the emulator (HR09). All four DIP levers read ON on the board with both DOWN and
  UP pull builds, matching the emulator with `ECP5_SWITCHES=1111`; the OFF level and lever-to-pad
  mapping are not checked on the board (HR10).
- **Unknown behaviour is never replaced by a convenient constant.** It is reported as `x`,
  rejected with a message, or taken from an explicitly named assumption profile.

## What runs today

Trellis describes the chip's configuration as 3 274 settings, 322 distinct fields once instance
numbers are folded. 204 fields execute for every value, 32 for some values (the others are refused
with an error), and 41 are not read because they have no digital effect (drive strength, slew,
termination, PLL loop trims). 23 of the 28 resource families execute
([SETTINGS_COVERAGE.md](../../docs/SETTINGS_COVERAGE.md), 2026-11-17, with the code line behind each
verdict).

| Area | State |
|---|---|
| LUT4, LUT5–7 muxes, flip-flops, CCU2 carry chains | Executed from the bitstream |
| Distributed RAM (LUTRAM/RAMW) | Executed ([LUTRAM_EXECUTION.md](../../docs/LUTRAM_EXECUTION.md)) |
| EBR block RAM | DP16KD with every width pair, PDPW16KD with every read width, including the 36-bit read yosys infers for `reg [31:0] mem[0:511]` (order of halves measured on the board, HB01); READBEFOREWRITE outputs the old word at widths 1, 2, 4, 9 and 18 (HB11); both executors return the new word for a pseudo-dual-port read of the word being written in the profile measured on the board (36/36, NORMAL, NOREG; HB09P) ([EBR_EXECUTION.md](../../docs/EBR_EXECUTION.md)) |
| DSP | MULT18X18D with input, pipeline and output stages; ALU54B with its full operation table, operands, cascades, flags and output/C/opcode registers, measured on the board in HD01–HD08 (one opcode register stage that fits no model is refused) ([DSP_ALU.md](../../docs/DSP_ALU.md), [DSP_EXECUTION.md](../../docs/DSP_EXECUTION.md)) |
| Clocks | Global clocks, static DCC, DCCA with a dynamic CE (HC01), event-kernel DCSC (HC02; fast mode refused), ECLKSYNCB, CLKDIVF /2 and /3.5, ECLKBRIDGECS ([CLOCK_NETWORK.md](../../docs/CLOCK_NETWORK.md)) |
| PLL (EHXPLLL) | Static settings with a periodic reference: dividers, phases and LOCK, and RST/STDBY holding and restarting the PLL as measured on the board (HC05, and the HC07 PLL series) ([PLL_EXECUTION.md](../../docs/PLL_EXECUTION.md)); fast mode refuses a live enabled RST/STDBY, which the event kernel runs; dynamic phase stepping is rejected |
| Internal oscillator OSCG | Free-running at a given base frequency over the divider measured on the board for each `OSC.DIV` bit pattern ([OSC_EXECUTION.md](../../docs/OSC_EXECUTION.md)) |
| PIO | Single-ended IO, tristate and bidirectional pads (also with a board model driving them from outside, as SDRAM data pins need), pulls and open drain; differential outputs and inputs run digitally (output P=data, N=complement; input reads P), including the pseudo-differential `LVCMOS33D` output of ULX3S HDMI designs; differential bidirectional and open-drain output pairs are refused ([PIO_BANKS.md](../../docs/PIO_BANKS.md)) |
| IOLOGIC | `ODDRX1F`/`IDDRX1F`, X2 and 7:1 gearboxes, IO registers and latches, DELAYF/DELAYG control, on side and top/bottom banks, as measured on the board ([IOLOGIC_EXECUTION.md](../../docs/IOLOGIC_EXECUTION.md)) |
| System | Async GSR, a user GSR from logic, GOE/GWDIS, logical wake-up phases ([SYSTEM_EXECUTION.md](../../docs/SYSTEM_EXECUTION.md)) |
| DTR (HY04, event kernel and fast mode) | Board-profile initial/final byte (ULX3S default 0x9E); 00 during high/conversion (fall-relative steps are not uniform), latest fall + measured pooled median 32.740 us; retrigger restarts. Fast mode schedules completion at the exact latest fall + 32 740 000 ps, together with any coincident clock edge before capture; no Celsius scale |
| Not executed (rejected when used) | DQSBUFM, DDRDLL, DLLDELD, JTAGG, SEDGA |

Around the chip, the ULX3S's HDMI (GPDI) connector, its SSD1331 OLED, its 32 MB SDRAM, its ISSI
IS25LP128F configuration flash, MAX11125 ADC, MCP7940N RTC, audio jack and an SD card are emulated
from what the design drives on their pads: a DVI receiver decodes the TMDS lanes into frames, the
OLED model follows the controller's command set, the SDRAM model answers on the data pins (a
PicoRV32 runs its firmware from it in `picorv32_sdram`), the flash model answers SPI reads and
programs and erases in memory through `USRMCLK`, with standard block protection and typical busy
timing from the datasheet ([PERIPHERALS_FLASH.md](../../docs/PERIPHERALS_FLASH.md)), the ADC answers the
design's SPI with its eight inputs on the header's own balls, taking each voltage from the pad (a
pull as its rail) or from the window ([PERIPHERALS_ADC.md](../../docs/PERIPHERALS_ADC.md)), the RTC model
answers the design's I2C reads of its registers and SRAM
([PERIPHERALS_RTC.md](../../docs/PERIPHERALS_RTC.md)), DIP switches SW1–SW4 supply live inputs with
starting positions set by `ECP5_SWITCHES`, and the SD card answers in SPI mode from a disk image
([PERIPHERALS.md](../../docs/PERIPHERALS.md)).

Flash changes stay in emulator memory: `ECP5_FLASH_IMAGE` remains a read-only source, and the model
writes nothing to disk. A program or erase attempt below `0x250000` warns that it would overwrite
the board's FPGA configuration on a real ULX3S, even when protection stops it.

The audio jack's three 4-bit resistor DACs (L, R, V) give each channel's open-circuit voltage,
modelled from the schematic and not measured, assuming nothing is plugged in; there is no playback.
The "Audio (jack)" window shows scopes, and [audio_tone](../../examples/ulx3s/audio_tone/README.md)
generates nominal 440 Hz on L and 660 Hz on R ([PERIPHERALS_AUDIO.md](../../docs/PERIPHERALS_AUDIO.md)).
The window attaches these models automatically; on the command line, `emulate --fast --peripherals`
attaches the same models (`--peripherals` requires `--fast`).

```sh
examples/ulx3s/build.sh examples/ulx3s/sdram_test target/reference/sdram_test
target/release/ecp5_emu emulate target/reference/sdram_test/top.config \
  --names target/reference/sdram_test/top_routed.json \
  --fast --peripherals --startup configured --scope observed \
  --clock net:clk_25mhz --period-ns 40 --cycles 1000000 \
  --console-out net:ftdi_rxd
```

The console prints `SDRAM W8000 E0000`.

## Measured numbers

All on an Apple M3 Max (10 performance and 4 efficiency cores); each number links to the measurement and its conditions. Fast-mode speeds depend on how busy the machine is: [FAST_MODE.md](../../docs/FAST_MODE.md#the-full-chip-life-life_full) shows `life_full` falling from 110 kHz to 36 kHz with eight busy loops beside it.

| What | Number | Source |
|---|---|---|
| Event kernel, board configurations (HI02: 418 FF, 439 LUT4; HC05 with a PLL) | 90 000–91 000 cycles/s | [EVENT_KERNEL.md](../../docs/EVENT_KERNEL.md#performance) |
| Fast mode, `uart_hello` (25 MHz board clock) | 41–46 M cycles/s, 1.6–1.85× real time | [FAST_MODE.md](../../docs/FAST_MODE.md#speed) |
| Fast mode, `ddr_pattern` (4 IOLOGIC DDR lanes) | 109–116 M cycles/s, 4.4–4.6× real time | [FAST_MODE.md](../../docs/FAST_MODE.md#speed) |
| Fast mode, PicoRV32 soft CPU from its bitstream | 2.31–2.39 MHz (2026-11-10) | [FAST_MODE.md](../../docs/FAST_MODE.md#the-full-chip-life-life_full) |
| The same SoC in Verilator 5.054, same session as fast mode's 1.93–1.94 MHz | 1.16–1.20 MHz from its Yosys `synth_ecp5` netlist; 12.2–16.6 MHz from the RTL | [FAST_MODE.md](../../docs/FAST_MODE.md#where-a-soft-cpu-spends-its-time) |
| Fast mode, Life 27×27 (10 376 LUT4, 3 threads) | 811–862 kHz | [FAST_MODE.md](../../docs/FAST_MODE.md#the-full-chip-life-life_full) |
| Fast mode, `life_full` 72×72 (78 700 LUT4, 94% of the chip), quiet M3 Max | 100–112 kHz (2026-11-10), 110–117 kHz (2026-11-11), both at 10 threads; default owner capture: 225–235 kHz at 10 threads (139–155 with caller capture), 185–204 kHz at 8 threads (98–109 with caller capture), build 7054d98, 200 000 cycles, load average 3.6–4.9, `FAST_TUNE=0 FAST_CHOOSE=0`, 2026-11-17 | [FAST_MODE.md](../../docs/FAST_MODE.md#measured-on-the-m3-max-2026-11-17) |
| The same in the window at its fastest rate, while other builds loaded the machine (load average 57–104) | 58–69 kHz (10th–90th percentile of 487 frames), from the window's own record of the clip above, 2026-11-16 | [ECP5_GUI_TRACE](../../gui/src/trace.rs) |
| The window's die map drawing `life_full`, 0 changes missed: `vsync.script` at 4 and 100 cycles/s; 1000 cycles/s; as fast as it goes; as fast as it goes over 75 s, including the busier phase from about 18 s | 59.4–59.8 fps; 59.6 fps; 58.4–58.9 fps; 58.0–58.3 fps (about 42 in the busier phase with the previous mask), from the window's own record, 2026-11-17 | GPU map commits 83b9561 and 2a0f011; [ECP5_GUI_TRACE](../../gui/src/trace.rs) |
| Opening `life_full` in the window: first open (no caches); re-open (plan and overlay from disk) | 50.2 s / 11.4 s until the run is loaded and paused; die on screen after 8.5 s / 0.37 s (55–62 s whole load before the caches), Apple M3 Max, 2026-11-17 | [WINDOW.md](../../docs/WINDOW.md#measured) |
| Compiling `life_full` to a fast-mode library, cold | 6.3 s (build 20e5d19, one cold compile on a first open; one clang call used to take 449 s) | [FAST_MODE.md](../../docs/FAST_MODE.md#measured-on-the-m3-max-2026-11-17) |
| Fast mode start on a re-open of `life_full` | 1.23 s plus one settings check of 0.45 s (build 20e5d19); a first open with no settings memory runs no trials since 2850aac (the full search took 82.4 s) | [FAST_MODE.md](../../docs/FAST_MODE.md#measured-on-the-m3-max-2026-11-17) |
| PicoRV32 sieve on the board vs the emulator | `151485` cycles in both (HR01, 3 of 3 loads) | [HARDWARE_VALIDATION.md](../../docs/HARDWARE_VALIDATION.md#results) |
