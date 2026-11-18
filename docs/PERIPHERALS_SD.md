# ULX3S SD card

The SD card model is in [src/peripherals/sdcard.rs](../src/peripherals/sdcard.rs), with the native
protocol in [sd/native.rs](../src/peripherals/sd/native.rs). It models an SDHC card in SPI mode
and native 1-bit or 4-bit SD-bus mode following the SD Physical Layer Simplified Specification
v2.00. It is pure
Rust and does not depend on either executor. The ULX3S balls from [board.rs](../src/board.rs)
are `SD_CLK` (`H2`), `SD_CMD` (`J1`, MOSI/DI), `SD_D[0]` (`J3`, MISO/DO, FPGA input) and
`SD_D[3]` (`K2`, CS). Native mode uses CMD bidirectionally and DAT0..3 at `J3/H1/K1/K2`.
Card detect `N5` is low (present) and write protect `P5` is low (writable) when read by a native
host. The following SPI sections retain the HR05 behaviour; native mode is described below.

## Sampling and timing

Create `SdCard::new(image)`, then call `feed(time_ps, SdPins { cs, clk, mosi })` in time order
after every executor instant. Pin values are `0`, `1`, `2` for `X`, and `3` for `Z`. Identical
samples do nothing; the model acts on edges only and imposes no clock-frequency or pulse-width
limits.

SPI mode 0, zero-delay:

- While `cs` is non-zero (deselected or unknown), DO returns `1` via `miso()`, matching the board's
  pull-up on `SD_D[0]`. Raising `cs` aborts any partial bit, incomplete 6-byte command frame, or
  active block read/write transfer and returns the card to `Idle` or `Ready`, while preserving
  card initialisation, CRC mode, `APP_CMD` state, and committed blocks.
- When `cs` falls to `0`, `miso()` immediately presents bit 7 of the next outgoing byte (`0xFF`
  when idle).
- While `cs == 0`, each rising `clk` edge shifts one `mosi` bit into the card (MSB first). Unknown
  `mosi` levels (`> 1`) discard the partial byte.
- Each falling `clk` edge shifts the next outgoing DO bit onto `miso()`, holding it steady through
  the next rising `clk` edge where the host samples it.

In fast mode, `tests/sdcard.rs` advances one board-clock edge at a time with
`FastSim::run_edges_sampled(1, &probes)`, feeds `SdCard`, and whenever `miso()` changes schedules
`fast.set_at(now + 1 ps, &[(miso_driver, level)])` so the new level reaches `J3` before the next
clock edge.

## Storage and commands

The card is block-addressed (512-byte sectors). Its capacity is `image.len()` rounded up to
512 bytes, with a minimum of 64 MB (131,072 blocks). Unwritten blocks beyond the supplied image
read as zero. Writes are kept in a sparse block map over the base image; `image()` materialises
the full capacity back as a `Vec<u8>`.

Commands are 6-byte frames (`0x40 | index`, 4-byte big-endian argument, `(CRC7 << 1) | 1`). The
card replies after a fixed `Ncr = 1` byte of `0xFF`. CRC7 ($x^7 + x^3 + 1$) is always checked on
`CMD0` and `CMD8`, and on all commands when `CMD59` turns CRC on; bad CRCs increment `crc_errors`
and return R1 with bit 3 (`0x08`) set. Unsupported commands or `CMD41` without `CMD55` increment
`illegal_commands` and return R1 with bit 2 (`0x04`) set.

Implemented commands:

- `CMD0`: enters SPI `Idle` state, clears init/CRC/app state, returns R1 `0x01`.
- `CMD8`: echoes the 4-byte argument in R7 (voltage accepted).
- `CMD55` + `ACMD41` (HCS): returns R1 `0x01` on the first two polls, then transitions to `Ready`
  and returns `0x00`.
- `CMD58`: returns R3 with OCR `0x00FF8000` before init and `0xC0FF8000` (powered up, CCS=1)
  afterwards.
- `CMD9` / `CMD10`: returns R1 `0x00`, a 2-byte `0xFF` gap, `0xFE`, the 16-byte CSD v2.0
  (reflecting capacity) or CID, and 2-byte CRC16-CCITT ($x^{16} + x^{12} + x^5 + 1$, initial 0).
- `CMD13`: returns R2 (`R1`, `0x00`).
- `CMD16`: accepts block length `512`; any other length returns R1 with parameter error (`0x40`).
- `CMD17` / `CMD18` (+ `CMD12` stop): returns R1 `0x00`, then for each block a fixed 2-byte
  `0xFF` gap, `0xFE` token, 512 data bytes, and CRC16-CCITT. `CMD12` stops `CMD18` mid-stream
  or between blocks and returns R1 `0x00`.
- `CMD24` / `CMD25`: returns R1 `0x00`, accepts data token `0xFE` (`0xFC` for multi-block, `0xFD`
  to stop `CMD25`), 512 bytes and CRC16, checks CRC16 if CRC is enabled (`0x0B` on mismatch),
  replies with data response `0x05` (`0b00000101`), and holds DO low (`0x00`) for a fixed 16 busy
  bytes.
- `CMD59`: enables or disables CRC checking (`arg & 1 != 0`).

Assumptions: fixed response delays (`Ncr = 1` byte, read gap `2` bytes, write busy `16` bytes)
and two `0x01` polls before `ACMD41` completes; `CMD8` echoes its argument without rejecting
non-VHS patterns; `CMD12` outside a multi-block read returns current `R1`.

## Status

`status()` returns `SdStatus` with `state` (`Idle`, `Ready`, `Reading`, `Writing`),
`last_command`, `last_argument`, `blocks_read`, `blocks_written`, `crc_errors`,
`illegal_commands`, and `version`. `version()` is a monotonic counter that advances whenever any
status field changes. It also exposes `mode` (`spi`, `sd1`, `sd4`), `native_state`, `rca`,
`bus_width`, `dat3_pullup`, `commands` and `responses`. A response count is per response frame,
not per byte. Native card states are `idle`, `ready`, `ident`, `stby`, `tran`, `data`, `rcv`, `prg`.

## Native SD-bus mode

The references below are to the SD Association's **Physical Layer Simplified Specification
v2.00**, available from the [SDA archive](https://www.sdcard.org/downloads/pls/archives/)
and as the [original publication mirrored by UT Austin](https://users.ece.utexas.edu/~valvano/EE345M/SD_Physical_Layer_Spec.pdf).
This is SD, whose CMD signalling is push-pull (§4.7.1), rather than MMC's open-drain
identification signalling. Electrical drive strength and analogue contention are not simulated.

| Behaviour | Specification | Model |
|---|---|---|
| CMD0 selects protocol | §7.2.1 | DAT3 low selects SPI until a new card/power cycle; high selects native idle |
| Command/data framing | §3.6.1, figures 3-5..3-8; §4.7.2 | 48-bit command, CRC7 and end bit; data start/end, CRC16 on each active DAT line |
| Identification and selection | §4.2, §4.8 table 4-28 | CMD0/8, CMD55 + ACMD41, CMD2, CMD3, CMD7, CMD9, CMD10 |
| Response formats | §4.9, tables 4-29..4-33 | R1, R1b with DAT0 busy, R2 (136 bits), R3 with reserved CRC bits, R6, R7 |
| Transfers and bus width | §4.3; §4.7.4 command tables and ACMD table 4-26 | CMD17/18 + CMD12; CMD24/25 + CMD12; ACMD6 selects 1 or 4 bits |
| CRC and errors | §4.5; §4.6.1; §4.10 table 4-35 | CRC7 checked on every command; per-line CRC16 checked on every write; bad/illegal commands have no response and latch status errors |
| Card registers and status | §5.2, §5.3.3, §5.6; §4.10 table 4-37 | CID, CSD v2.0, SCR (ACMD51), SD status (ACMD13), CMD13; CMD16 accepts 512 bytes for SDHC |
| Switch function | §4.3.10, table 4-27 (CMD6) | Group 1 query and set default/high-speed flag; 64-byte switch status, no faster electrical timing model |
| DAT3 pull-up | ACMD42 in table 4-26 | Internal nominal 50 kΩ pull-up enabled initially; argument bit 0 connects/disconnects it |

`SdCard::feed_bus(time_ps, native::Pins { clk, cmd, dat })` samples resolved pads with the
same 0/1/X/Z encoding as SPI. The card releases CMD and DAT transmitters with `None` when idle.
Outputs change after falling SD clock edges; the answering runner installs changes at `now + 1 ps`,
so the host samples them at the next rising edge. Four-bit data sends the high nibble first,
with bit 3 on DAT3, and a separate CRC16 on each line. A write returns a DAT0 CRC-status token
(`00101` accepted, `01011` CRC error), followed by busy for an accepted block. CMD and DAT can
run concurrently, for example CMD12 while streaming data. Writes alter the same sparse in-memory
overlay as SPI and never modify `ECP5_SD_IMAGE` on disk. Cloning the card preserves protocol,
queued responses, partial blocks and that overlay for checkpoints.
After a multi-block write CRC error, further data is ignored until CMD12 (§4.3.4).

The DAT3 bias is separate from the push-pull transmitters. `Attached` represents it as an ideal
weak logical high: it supplies high while the pad is released/high, withdraws on a host low or
unknown, and is disabled during host data writes. Resistance and analogue settling are not
computed. ACMD42 removes this card bias, not the FPGA pad's own `PULLMODE=UP`. Bidirectional
CMD/DAT releases reach the external driver as Z. An input-only DAT pad instead gets Boolean
idle high: fast mode is two-valued there and refuses a Z input, as with SPI MISO. This assumes
the host's required pull-ups; arbitrary input-only pull-down configurations are not modelled.

Native attachment requires a driven CLK, bidirectional CMD, and DAT0 input or bidirectional.
Every configured CMD/DAT ball is traced; only receiving pads get answering drivers. The old SPI
attachment (driven CLK/CMD/DAT3, read DAT0) is unchanged. An active command, response, data block,
write or busy interval has horizon 1 board edge. Idle native horizon is 16 edges: even with one
SD edge per board edge, a 48-bit command takes at least 96 edges before the response delay, so
the next chunk detects a partial command before any answer is due. SPI's idle horizon stays 64.

### Native identity, capacity and assumptions

The native CID uses MID `00` as an explicit fictional
placeholder, OID `XE`, product `EMU00`, revision 1.0, serial zero, January 2026. No real
manufacturer is claimed; the manufacturer registry was not verified. The default 64 MiB card's
CSD is **`400E003251590000007F7F800A40000F`**. CSD v2.0 expresses capacity in 512 KiB units,
so native mode rounds the supplied raw image up to that unit (minimum 64 MiB), zero-filling the
tail. C_SIZE is derived from that accessible capacity and both register CRC7s are computed.
The 64 MiB virtual SDHC fixture is retained for compatibility, below physical SDHC's 2 GB minimum.
The legacy SPI CID bytes and 512-byte capacity rounding stay unchanged for existing users and
HR05. The identities therefore differ between protocols; this is a compatibility assumption,
not a claim that a physical card has different CIDs. Image format and storage are shared.

RCA is fixed at `0001`; this is one card, without bus arbitration. ACMD41 completes on the third
HCS/compatible-voltage poll. OCR is `00FF8000` before ready and `C0FF8000` afterwards. SCR advertises
SD specification 2.00 and 1/4-bit widths. SD status contains the bus width with other fields zero;
the switch status advertises only group 1 functions 0 and 1. These minimal status values, reset
latency and no variable write latency are assumptions, not measurements of a particular card.

The simplified v2.00 publication omits the full §4.12 timing tables. The N_CR range 2..64 clocks
and identification delay of at least 5 clocks are corroborated by the SD timing table in
[Freescale MC9328MX1, table 31](https://media.digikey.com/PDF/Data%20Sheets/Freescale%20Semi/MC9328MX1.pdf).
The model chooses fixed N_CR=4 clocks, identification delay=5, read gap=4, write status gap=2
and accepted-write/select/stop busy=8 clocks. The latter fixed delays are modelling assumptions.
CMD16 lengths up to 512 are accepted but do not change SDHC memory transfers; larger lengths
set BLOCK_LEN_ERROR (§4.3.14). The card-lock length itself is unused because CMD42 is not modelled.
Clock rate, rise/fall time, power-up delay and timeouts of a real card are not enforced.

### Native reference and private board method

[sd_native_read/top.v](../examples/ulx3s/sd_native_read/top.v) wakes at 390.625 kHz, uses only
CMD0, CMD8, CMD55+ACMD41 (up to 4096 polls), CMD2, CMD3, CMD9, CMD7, CMD55+ACMD6, CMD16 and
CMD17 block 0, then receives four-bit data at 6.25 MHz. CMD is driven only for those command
frames; all DAT drivers are permanently disabled. There is no data-write state or media-write,
erase or lock command. The plain `reference_can_only_send_read_only_initialization_and_cmd17`
test checks the command ROM and disabled DAT drivers and seals all HDL/constraint sources
against extra send paths. It needs no FPGA toolchain.

With the demo image the first complete UART reports at 115200 8N1 are:

```text
SDN OCR C0FF8000 RCA 0001 CSD 400E003251590000007F7F800A40000F
SDN CID <32 hex characters of the fictional identity described above>
SDN B0 958745C6 CRC16 ok
SDN B0RAW <1024 hex characters of the synthetic demo block>
```

The four lines repeat once per second from the saved read. `tests/sd_native.rs` checks the whole
demo output through the fast CLI; unit tests cover 1-bit and 4-bit reads/writes, each response,
multi-block stop, state/error handling, per-line CRCs and restoration. The live test rewinds
before initialization, then replays the same pad answers and UART; CLI tests check the report.

The native board check [HR14](inventory/LFE5U-85F/hr14_sd_native/README.md) captured ten identical
block-0 reports across three SRAM loads on 2026-11-17, with valid four-bit per-line CRC16 and a
passing private-block replay on rserv. For another check, build this read-only design with
`examples/ulx3s/build.sh`, load SRAM under the shared board lock, and capture prefix `SDN` with
the existing HR04 `run.py` into **`out/sd/board_native.log` only**. Its CID (manufacturer, product, serial, date) and
`B0RAW` (512 raw bytes) are private and must never enter git, including test failure output.
Committed records represent those private fields only with **SHA-256 hashes computed on the
host**, alongside non-identifying protocol and comparison results. A CRC-32 is not a privacy
hash: a serial can be recovered from it. The block CRC in HR14 checks the transfer; it does
not replace the SHA-256 hashes of the private data.

`the_private_board_block_reproduces_in_the_emulator` skips cleanly without that private log.
With it, the test extracts the first complete indented `B0RAW` and `B0 ... CRC16 ok` lines,
creates a temporary private image in `out/sd/`, and checks the emulator's same block CRC line
and `CRC16 ok`, without dumping either payload. CID/CSD/OCR/RCA are deliberately not compared:
the emulated card is a different card. Temporary replay images are removed after the test.
HR14 records that comparison as PASS; it does not establish writes, high-speed mode or CMD/DAT
timing margins.

## Reference design and checks

[sd_read/top.v](../examples/ulx3s/sd_read/top.v) runs on the 25 MHz clock with plain fabric
logic only (no PLL, EBR or DSP). It drives 80 wake clocks at ~391 kHz (`25 MHz / 64`) with CS
high, runs `CMD0`, `CMD8`, `CMD55`+`ACMD41` (HCS) and `CMD58`, switches SCLK to `6.25 MHz`
(`25 MHz / 4`), reads block 0 via `CMD17`, and prints
`SD OCR C0FF8000 B0 <first 16 bytes hex> <bytes 510,511 hex>\r\n` (or `SD ERR <step> <R1>\r\n`)
on `ftdi_rxd` (`L4`) at 115200 8N1 once per second, showing the step reached on `led[7:0]`.

Build and test commands:

```sh
examples/ulx3s/build.sh examples/ulx3s/sd_read target/reference/sd_read
cargo build --release
cargo test --release --lib sdcard
cargo test --release --test sdcard -- --ignored --test-threads=1 --nocapture
```

All seven unit tests in `peripherals::sdcard::tests` passed (`init`, block 0 and block 12345
reads, CRC checks with `CMD0`/`CMD8`/`CMD59`, `CMD18` + `CMD12`, `CMD24`/`CMD25` write/readback,
illegal commands, and `CS` deassertion mid-command).

Measured in `tests/sdcard.rs` against a 64 MB MBR-like image, the design needed 0 warm-start
cycles and printed:

```text
SD OCR C0FF8000 B0 EB3C90454350355F454D550002080100 55AA
```

It reached the end of that UART line in 194,859 board cycles (7.79 ms of board time) in **0.079 s**
of wall time (**2.46 MHz**), excluding synthesis, model loading and C compilation.

nextpnr (seed 1) reported:

```text
Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 84.05 MHz (PASS at 25.00 MHz)
Info: Max frequency for clock '$glbnet$clk_25mhz$TRELLIS_IO_IN': 88.93 MHz (PASS at 25.00 MHz)
```

Resource counts were 759 LUT4s, 821 `TRELLIS_COMB`, 378 `TRELLIS_FF`, 14 `TRELLIS_IO` and 1
`DCCA` (0 `DP16KD`, `MULT18X18D`, `ALU54B`, `EHXPLLL`, `IOLOGIC`, `TRELLIS_RAMW`). No emulator
feature was refused.

## Limits

Not modelled: UHS, 1.8 V switching, SDUC, clock-rate or pulse-width limits (including the
400 kHz init cap), erase commands (`CMD32`/`CMD33`/`CMD38`), card lock/unlock (`CMD42`), physical
write-protect changes or hot removal, SDSC byte-addressed cards (< 2 GB v1/v2), power sequencing,
analogue loading and contention. Native CDn/WP report a present writable card when used; the
legacy SPI attachment is unchanged. CMD6 high-speed selection is a flag, not a timing-limit model.

## On the board

[HR05](inventory/LFE5U-85F/hr05_sd_card/README.md): with the card in the board's slot, `sd_read`
read block 0 on 3 of 3 loads, and the card's OCR was the model's, `C0FF8000`. One earlier load gave
up during ACMD41 while the card was still idle: a real card may need up to a second there, the model
none. The design now polls up to about 1.6 s.

## In the window

The live runner attaches SPI and native cards by the rules above. It answers
while the design runs (docs/PERIPHERALS.md, *In the window*), and an *SD card* window shows
its protocol, native state, RCA, width, command/response counts and transfer/error counts. `# sd`
record notes include those fields and the DAT3 pull-up flag. The image is the file `ECP5_SD_IMAGE`
names, else `SdCard::demo`: a boot sector with the
OEM name `ECP5_EMU` and the 55 AA signature in an otherwise empty 64 MB card.
`tests/live_peripherals.rs` runs `sd_read` there: the line for the demo card, and the same line
after going back to before the read.
