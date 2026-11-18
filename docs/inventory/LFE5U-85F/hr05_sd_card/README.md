# HR05: sd_read on the board, with a real micro-SD card

[sd_read](../../../../examples/ulx3s/sd_read/top.v) drives the ULX3S's micro-SD slot in SPI mode,
as [docs/PERIPHERALS_SD.md](../../../PERIPHERALS_SD.md) describes:
- the wake clocks, then CMD0, CMD8, CMD55 + ACMD41 until the card is ready, CMD58 at ~391 kHz;
- block 0 at 6.25 MHz;
- `SD OCR <OCR> B0 <first 16 bytes> <bytes 510, 511>` on the UART every second, or
  `SD ERR <step> <R1>`.

It ran on 2026-11-16 with the card that was in the board's slot. The card is unknown and was not
taken out.

## Setup

- **Board:** ULX3S "85K v3.0.8" (LFE5U-85), SRAM load with `openFPGALoader -b ulx3s`, under the
  shared board lock.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py) `BITSTREAM SD LOADS 3`.
- **Configuration:** [sd_read.config](sd_read.config) is the one of the second run below.
- **Records:** [TOOLS.txt](TOOLS.txt), [SHA256SUMS](SHA256SUMS),
  [observations.log](observations.log).

## Observations

| Run | Design | Loads | Lines |
|---|---|---|---|
| 1 | ACMD41 given up after 255 polls (~100 ms) | 1 | `SD ERR 04 01`: still idle at ACMD41 |
| 1 | the same | 2 | `SD OCR C0FF8000 B0 00000000000000000000000000000000 55AA` |
| 2 | ACMD41 polled up to 4095 times (~1.6 s) | 1, 2, 3 | `SD OCR C0FF8000 B0 00000000000000000000000000000000 55AA` |

- The first load of run 1 found the card still idle when the design stopped polling. SD cards may
  take up to a second to leave idle. The emulator's card is ready after two polls (an assumption
  of docs/PERIPHERALS_SD.md), so the emulator did not show this. The poll limit in the design
  was raised to 4095, about 1.6 s. A power-up from cold was not repeated: the card stayed powered
  between loads.
- The real card's OCR is the emulator's card's: `C0FF8000` (powered up, SDHC, 2.7–3.6 V). The
  command sequence, the R1/R3/R7 answers, the data token and the CRC-16 of a block read
  therefore worked on a real card as on the model.
- Block 0 of this card starts with 16 zero bytes and ends with the 55 AA signature: an MBR with
  no boot code, as the SD Association's formatter writes. The emulator's demo card has a
  different block 0, so the bytes differ by design. Given this block 0 (512 bytes: zeros, then
  55 AA) as `ECP5_SD_IMAGE`, the live runner printed exactly the board's line, also after going
  back before the read: `ECP5_SD_IMAGE=block0.img cargo test --release --test
  live_peripherals -- --ignored the_live_sd`.
