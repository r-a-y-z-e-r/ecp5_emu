# HR02: ram32 and mac on the board, against the emulator

Two reference designs of [examples/ulx3s](../../../../examples/ulx3s/README.md), both run on the
board on 2026-11-11 in the HR series (whole designs compared with the emulator, see
[HR01](../hr01_picorv32_console/README.md)). Each checks what yosys writes for a construct
students use all the time:
- [ram32](../../../../examples/ulx3s/ram32/top.v): `reg [31:0] mem [0:511]`, as a ROM from
  `$readmemh` and as a RAM. Each is one EBR read 36 bits at a time (PDPW16KD,
  `DATA_WIDTH_R=36`).
- [mac](../../../../examples/ulx3s/mac/top.v): `*`, seven MULT18X18D without registers. A
  signed 16×16 dot product over 4096 LFSR pairs, and an unsigned 32×32 product that yosys splits
  over four multipliers and adds in the fabric.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Configurations:** [ram32.config](ram32.config) and [mac.config](mac.config) are the
  configurations that ran: `examples/ulx3s/build.sh`, nextpnr seed 1.
- **Runner:** `run.py BITSTREAM PREFIX 3 1.5` ([run.py](run.py)) loads the bitstream three
  times. After each load it reads the serial port for 1.5 s and prints the distinct lines that
  start with PREFIX. The last line of a window can be cut by the window's end.
- **Prediction:** before the board run, both executors of the emulator printed the lines below
  for these configurations, and the tests computed them independently:
  - `tests/reference_designs.rs` `ram32_reads_36_bit_block_ram_words_back`: the sum of
    `rom.hex`;
  - `mac_prints_the_dot_product_of_its_dsp_multipliers`: the arithmetic of `top.v`, in Rust.

## Observations

[observations.log](observations.log):

| Design | Loads | Lines per load | Line |
|---|---:|---:|---|
| ram32 | 3 | 16 | `ROM C9083F00 RAM 0000` |
| mac | 3 | 597–599 | `MAC 003D16367D MIX CE196146` |

The other lines in the mac log, `MAC 003D163` and `MAC 003D1636`, are the last line of a window,
cut when the reader stopped.

## Reading

1. **ram32.** A 32-bit memory that yosys infers works on the chip: the ROM sums to the value of
   `rom.hex`, and all 512 RAM words read back as written. This also confirms the order that
   [HB01](../hb01_pdp36/README.md) measured pin by pin.
2. **mac.** The products of the seven MULT18X18D, signed and unsigned and their sums in the
   fabric, are what the arithmetic gives. This is the first board result for the DSP blocks.
   It covers the profile yosys writes (no register stages, CIBOUT_BYP=ON, static SIGNEDA/B per
   multiplier); the open D01–D10 questions about registers and the ALU are not touched.
3. The emulator, event kernel and fast mode alike, prints the same lines for the same
   configurations.
