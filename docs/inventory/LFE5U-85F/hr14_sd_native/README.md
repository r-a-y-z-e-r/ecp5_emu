# HR14: native SD-bus reads on the board, against the emulator

[sd_native_read](../../../../examples/ulx3s/sd_native_read/top.v) initializes an SDHC card
on the native SD bus, selects four-bit data, and reads block 0, as
[PERIPHERALS_SD.md](../../../PERIPHERALS_SD.md) describes. It cannot write the card's
media: its command list contains only initialization, configuration and one CMD17 read,
and its DAT output drivers are permanently disabled.

## Setup

- **Board:** ULX3S 85K v3.0.8, with the same microSD card as
  [HR05](../hr05_sd_card/README.md), still in its slot. SRAM loads only, under the shared
  board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `SDN`, three loads
  of 3 s each at `21:22:47Z`, `21:22:56Z` and `21:23:05Z`.
- **Configuration:** [sd_native_read.config](sd_native_read.config), copied from
  `target/reference/sd_native_read/top.config`, built on the Mac with
  `examples/ulx3s/build.sh` (nextpnr seed 1). Max frequency was **72.85 MHz** at 25 MHz.
  The loaded `top.bit` is not stored; its hash is in [SHA256SUMS](SHA256SUMS).
- **Records:** [observations.log](observations.log), copied verbatim from
  `out/sd/board_native_redacted.log`; [TOOLS.txt](TOOLS.txt); [SHA256SUMS](SHA256SUMS).
  CID and B0RAW lines were already redacted in that source. The private original
  `out/sd/board_native.log` was not opened when preparing this record.

## Observations

| Load time (UTC, 2026-11-17) | UART lines | Complete read reports |
|---|---:|---:|
| 21:22:47Z | 16 | 4 |
| 21:22:56Z | 12 | 3 |
| 21:23:05Z | 12 | 3 |

All ten complete read reports were identical. Each load reads block 0 once, then repeats
the four UART lines from that saved result. Their non-private protocol reports were:

```text
SDN OCR C0FF8000 RCA AAAA CSD 400E00325B590000EDC87F800A4040C3
SDN B0 93FA47A1 CRC16 ok
```

The card completed CMD0/8, CMD55 + ACMD41, CMD2/3/9/7, CMD55 + ACMD6 to select four-bit
data, CMD16, and CMD17 of block 0. `CRC16 ok` checks all four DAT-line CRC16s and the
data end bit. The OCR, `C0FF8000`, is HR05's.

The CSD has CSD_STRUCTURE=1 (CSD v2.0). Its C_SIZE (bits 69..48) is `0x00EDC8` = 60,872.
By the v2.0 capacity rule, `(C_SIZE + 1) × 512 KiB` is **31,914,983,424 bytes**
(31.915 GB, 29.723 GiB), or 62,333,952 sectors of 512 bytes. Capacity is not identifying.

## Private data

The CID never appears in git: its manufacturer, product, serial number and date identify
the card. Raw block 0 also stays private. Both remain only in `out/sd/`; the committed
log keeps redaction markers in their places. A CRC-32 would not hide the serial and is
not a privacy hash. The block CRC above is a transfer check, not a substitute for these
host-computed SHA-256 hashes supplied with the board results:

| Private source | SHA-256 |
|---|---|
| CID hex text | `87ec105be92cae674bedae1e63dfe14a6096219e34b7fbd44c79538954bbb3ea` |
| B0RAW hex text | `257e0055beb442408dcee2a08e75c5c0d14cd216c2dc31448c2edb4fffcd3dbe` |
| Whole private log, `out/sd/board_native.log` | `28fe8f39bc2da0f10105d490f1484be0154598fa47b9110ab06a9bd01957bfdd` |

## The comparison

[tests/sd_native.rs](../../../../tests/sd_native.rs)
`the_private_board_block_reproduces_in_the_emulator` was reported **PASS on rserv,
2026-11-17**. With an image built from the board's private block 0, the emulator prints
the same `SDN B0 93FA47A1 CRC16 ok`. CID/CSD/OCR/RCA are not compared: the emulated card
is a different, fictional card. The private replay skips cleanly when its log is absent.

The plain `reference_can_only_send_read_only_initialization_and_cmd17` audit was also
reported **PASS on rserv**. The read-only evidence has three parts:

- The only commands are CMD0, CMD8, CMD55 + ACMD41, CMD2, CMD3, CMD9, CMD7,
  CMD55 + ACMD6, CMD16 and CMD17 (block 0).
- DAT0..3 are permanently tristated: each `BB` has `T=1`. The host sends no data block.
- The audit checks that command whitelist and the disabled DAT drivers, and seals the
  HDL and constraints against additional send paths. It requires no FPGA toolchain.

The checks can be run with these plain commands, **on rserv**:

```sh
cargo test --release -p ecp5_emu --test sd_native -- --ignored --exact the_private_board_block_reproduces_in_the_emulator
cargo test --release -p ecp5_emu --test sd_native -- --exact reference_can_only_send_read_only_initialization_and_cmd17
```

## What this establishes

The real card completed native-mode initialization and a four-bit block read with valid
per-line CRC16, and the emulator reproduced that block's CRC report. This does not
establish writes (the reference cannot perform them), high-speed mode, or CMD/DAT timing
margins. The reference uses 6.25 MHz data clocks, not a high-speed-mode check. No model
identity or timing parameter was fitted to this card.
