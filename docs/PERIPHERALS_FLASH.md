# ULX3S configuration flash

The pure Rust model [spiflash.rs](../src/peripherals/spiflash.rs) answers as U10,
an ISSI **IS25LP128F**: 128 Mbit (16 MB), three-byte addresses, JEDEC `9D 60 18`.
The balls are checked against `assets/ulx3s/ulx3s_v3.0.8.kicad_pcb`:
SCK U3, nCS R2, MOSI/IO0 W2, MISO/IO1 V2, nWP/IO2 Y2, nHOLD/IO3 W1.
They are `board::FLASH_*` in [board.rs](../src/board.rs).

## USRMCLK and sampling

SCK is the dedicated CCLK ball **U3**, driven through the executed `USRMCLK`
resource (`LFE5U-85F/R95C0/USRMCLK`, dev `9ca91a4`). Probe **`pin:U3`** in
both executors: it follows non-inverted USRMCLKI while USRMCLKTS=0 and GOE=1,
and is Z otherwise, with no inferred pull. With CCLK.MODE=NONE it is undriven.
The flash uses this pad just like any other peripheral ball; it does not probe
USRMCLK's internal input wires.

`compile::cclk_pad(device, PACKAGE)` supplies the configured pad direction to
`board::direction` and `board::used_balls`. CCLK ownership is read only from
**EFB3_PICB1 CCLK.MODE**. EFB1's apparent CCLK.MODE bit belongs to OSC.DIV and
must not be read as another enable or through the composed field. See
[SYSTEM_EXECUTION.md](SYSTEM_EXECUTION.md#usrmclk-and-the-dedicated-cclk-pad).
The test `usrmclk_pad_probe_follows_the_routed_clock_in_both_executors` compares
both executors on routed `flash_dump` and checks U3 against an ordinary output
pad on the same SCK net.

Create `SpiFlash::new(image)`, then feed `feed(time_ps, Pins { cs, clk, mosi })`
in time order. Levels are 0, 1, 2 (X), 3 (Z). SPI modes 0 and 3 both sample MOSI
on rising SCK and present MISO on falling SCK, MSB first. Identical samples do
nothing. CS high ends the transaction, discards incomplete commands and returns
MISO high. An unknown clock or MOSI aborts that transaction; unknown CS deselects.
No real-time pulse width, reset delay or power sequencing limits are imposed.

`Attached::new(used)` attaches the flash when nCS, MOSI and SCK (U3) are
outputs and MISO is an input. `Attached::balls` lists the traced pad columns,
including U3. Unknown or tristated SCK aborts the command until CS rises, as
for any unknown SPI clock sample. MISO starts at 1.
`horizon` is one board-clock edge while selected, 64 otherwise. Answers go to
V2 1 ps after the instant. The live runner watches every edge even in idle
batches: an eight-bit flash command could otherwise fit inside an idle batch.
The reference controller additionally holds CS low for 32 board cycles before
the first SCK edge, allowing the standalone test to batch idle intervals. The supported clock is at most half the board clock;
a faster PLL-driven SCK would need answers within a board-clock edge.

## Image, programming and erase

The image changes **in emulator memory only**. `ECP5_FLASH_IMAGE` remains a
read-only raw binary source, padded with FF to 16 MB (longer files are truncated).
The model never saves an image or writes to any file. With no file, Rust builds
an erased image with the reference XIP firmware at 200000: ASCII `RVFW`, a
little-endian length and CRC-32/ISO-HDLC, then the committed payload at 20000C.

The specification used here is ISSI **IS25LP128F/IS25WP128F Rev. A5,
10 October 2019** ([manufacturer link](https://www.issi.com/WW/pdf/25LP-WP128F.pdf),
[archived manufacturer PDF](https://atta.szlcsc.com/upload/public/pdf/source/20200428/C521289_AA48CBCD7831419257F95A73C518D4B5.pdf)).
Section/table references below refer to this revision; the timing tables in
§9.6 and §9.9 do not carry separate table numbers.

| Commands | Model behaviour | Datasheet reference |
|---|---|---|
| 06 WREN / 04 WRDI | Set / clear WEL at CS rising; no image change | §8.16–8.17; §6.1 Tables 6.1–6.3 |
| 02 PP | WEL required; 1..256 latched bytes, address wraps within the 256-byte page; later bytes replace earlier page-buffer bytes; commit ANDs the final buffer into storage | §8.10, Table 8.3, Figure 8.21 |
| 20 / D7 SER | Erase the containing aligned 4 KB sector to FF, with WEL | §8.13, Tables 6.3 and 8.3, Figure 8.27 |
| 52 BER32 / D8 BER64 | Erase the containing aligned 32 / 64 KB block, with WEL | §8.14, Tables 6.3 and 8.3 |
| C7 / 60 CER | Erase the entire 16 MB, only with WEL and BP=0 | §8.15; §6.1 Table 6.4; Table 8.3 |
| 01 WRSR | WEL required; write BP0..3, QE and SRWD, preserving the meaning of read-only WIP and WREN/WRDI-controlled WEL | §8.19, §6.1 Tables 6.1–6.3 |
| 05 RDSR | Repeated current status bytes | §8.18, Tables 6.1/6.2 |
| 48 RDFR / 81 RDERP | TBS/default function bits; extended status with WIP and latched protection/program/erase error bits | §8.20/8.29, Tables 6.5/6.6 and 6.12/6.13 |

The status byte is **bit 0 WIP, bit 1 WEL, bits 2..5 BP0..BP3, bit 6 QE,
bit 7 SRWD**, exactly Tables 6.1/6.2. `with_status(image, register)` configures
bits 2..7; WIP/WEL are volatile and start clear. Default status is 00. Completed
WRSR changes the status in memory; neither software reset nor a snapshot clone
writes it back to the source file.

Block protection uses the **standard Table 6.4**, with factory **TBS=0 (top)**
from Function Register bit 1 (§6.2, Tables 6.5/6.6). BP=0 protects nothing;
BP=1..8 protects respectively 1,2,4,8,16,32,64,128 top 64 KB blocks;
BP=9..15 protects all 256 blocks. A protected page program or sector/block
operation is rejected as a whole. Any nonzero BP rejects chip erase. Protected
attempts increment `protected` and `ignored_writes`, and set the extended
register's PROT_E and P_ERR/E_ERR (§6.3.2 Tables 6.12/6.13).

TBS is fixed at its default: WRFR 42 and OTP/ASP configuration are not executed.
The optional ordering-code **B** protection table is not modelled. This choice
is an assumption because HR07 did not identify that ordering option. HR07's
real-board **SR 18 has BP1/BP2 set**: BP=6, protecting the top 32 blocks (2 MB)
in either table. Reads are unaffected by protection.

### Busy timing in emulated time

A complete, byte-aligned command starts on CS rising. WIP remains 1 for the
following **typical** interval, and WEL stays set until completion, then clears.
The deadline is measured from `feed(time_ps, pins)`, never wall-clock time.
Even identical/deselected samples advance a pending operation.

| Operation | Typical interval | Datasheet timing table |
|---|---:|---|
| Page program | 0.2 ms (200 µs) | §9.6 AC Characteristics p.167, tPP; §9.9 Program/Erase Performance p.171 |
| 4 KB sector erase | 100 ms | §9.6 tEC sector row; §9.9 sector row |
| 32 KB block erase | 140 ms | §9.6 tEC 32 KB row; §9.9 32 KB row |
| 64 KB block erase | 170 ms | §9.6 tEC 64 KB row; §9.9 64 KB row |
| Chip erase | 35 s | §9.6 tEC chip row; §9.9 chip row |
| WRSR | 2 ms | §9.6 AC Characteristics p.167, tW |

The datasheet calls the four erase rows `tEC`; `T_SE_PS`, `T_BE32_PS`,
`T_BE64_PS` and `T_CE_PS` distinguish them in Rust. Typical timings are
reference values at typical VCC and 25°C, not guaranteed limits (§9.6 note 2).

While busy, only 05/48/81 polls and the 66+99 reset sequence are implemented as
permitted commands (§6.1 WIP description). Other commands are ignored and counted
in `busy_ignored`; rejected write commands also increment `ignored_writes` once.
Suspend/resume is not implemented, even though §8.23 permits it. Reset cancels
an in-flight operation. WREN/WRDI sent while busy cannot change WEL.

Existing reads remain: 9F returns repeating `9D 60 18`; 03 reads after three
address bytes and wraps at 16 MB; 0B adds eight dummy clocks; 90 alternates
manufacturer 9D / device 17, starting at the device if address bit 0 is one;
AB releases power-down and reads device 17 after three dummy bytes; B9 enters
power-down. CS high ends each transaction. Unsupported write/control opcodes
retain their previous ignored-and-counted behaviour. Dual 3B, quad, ASP,
OTP writes, HOLD/WP electrical timing and power sequencing remain unmodelled.

### Assumptions and limits

- Standard BP table, TBS=0, ASP sectors unlocked. The model cannot select bottom
  protection or the optional B ordering table. Function bits other than TBS are
  unmodelled and RDFR returns zero for them, rather than claiming their
  package-dependent reset defaults were measured.
- WP# is high, as both reference designs drive it. Therefore SRWD can be stored
  and read but alone does not block WRSR (§7.1 Table 7.1); low-WP hardware
  status-write protection is outside the pad model. QE is stored but does not
  add quad operation.
- All page programs use full-page tPP, including partial pages. Deterministic
  contents remain old until the completion deadline; partial progress is not
  exposed. Poll results latch a byte at its first outgoing bit.
- BP-rejected operations have zero latency and preserve WEL: no rejection
  duration is specified. Malformed/unknown-pin writes are discarded; commands
  requiring fixed lengths reject extra clocks. WREN/WRDI apply at CS rising.
- Reset is instantaneous and discards pending changes without corruption.
  Real reset during program/erase can corrupt the target (§8.37); recovery
  delays and power-loss behaviour are not simulated.

## Cheap checkpoints and the way back

The 16 MB base is immutable and shared. A second `Arc` shares an index of
4096 optional 4 KB sector `Arc`s. `SpiFlash::clone` copies only the small
protocol/status state (including a 256-byte touched mask) and shared pointers;
it never copies 16 MB or the sector index. An in-flight operation and its
page buffer are also shared.

On the first change after a checkpoint, copy-on-write copies at most **32 KB
of index pointers** (on a 64-bit host) plus **4 KB per changed sector**. Further
writes to uniquely owned sectors reuse them. Sector/block erase shares one
FF sector; chip erase switches to an erased-base flag and an empty 32 KB index,
without allocating or copying a 16 MB erased image. Explicit `image()` after
changes materialises 16 MB for callers; checkpoints never call it.

Restoring `Attached::Snapshot` restores sectors, pending deadlines, WEL/WIP,
statistics and warnings. Going back before program or erase therefore restores
old contents, not just the peripheral's display.

## The FPGA configuration area

`spiflash::CONFIGURATION_END` is **0x250000 (2,424,832 bytes, 37 × 64 KB)**.
Its source is the largest standard **uncompressed LFE5U-85F** stream, not the
size of a particular compressed design:

| Component | Verified source | Bytes |
|---|---|---:|
| CRAM | [Trellis devices.json](https://github.com/YosysHQ/prjtrellis-db/blob/9754ff473bfaa1e6edfaf57ce13bd447c76a88f6/devices.json): 13,294 frames × 1,136 bits / 8 | 1,887,748 |
| All EBR initialisation | [model summary](inventory/LFE5U-85F/core_model_summary.json): 208 EBRs; [EBR contract](inventory/LFE5U-85F/ebr_research/memory_contract.json): 2,048 × 9 = 18,432 bits each, × 208 / 8 | 479,232 |
| Frame CRC/padding | [Trellis Bitstream.cpp](https://github.com/YosysHQ/prjtrellis/blob/a31ff7042e712c0e2ad217054116ce2c4995f5c7/libtrellis/src/Bitstream.cpp), `BitstreamOptions` / `serialise_chip`: (2 CRC + 1 dummy byte) × 13,294 | 39,882 |
| EBR commands/CRC | Per EBR: address command 8 + write command 4 + CRC 2, × 208 | 2,912 |
| Fixed commands and header | Preamble 4 + padding 4 + reset CRC 4 + verify ID 8 + control 8 + init address 4 + frame command 4 + SED/security padding 12 + usercode/CRC 10 + done 4 + trailing padding 4 + optional SPI_MODE 4 + empty metadata header 3 | 73 |
| Total before rounding | 1,887,748 + 479,232 + 39,882 + 2,912 + 73 | **2,409,847** |
| Round up to 64 KB | ceil(2,409,847 / 65,536) × 65,536 | **2,424,832 = 0x250000** |

This calculation assumes standard Trellis commands and no arbitrary-length
metadata comments or extra user-inserted commands; serialized metadata is not
hardware configuration geometry and has no finite maximum. The block rounding
leaves 14,985 bytes beyond the counted standard stream. This is a conservative
model warning region, not a memory-allocation scheme for custom bitstreams.
Compressed streams (`ecppack --compress`, as used by the references) are smaller:
`life_full` is 1,369,899 bytes. The demo firmware at 200000 lies **inside** this
maximum configuration region; reading it is safe, and programming it warns too.

Every well-formed program/erase attempt whose affected range intersects
`[0, CONFIGURATION_END)` increments `config_writes`, even when WEL, BP or busy
prevents it. The first event is retained with its command and address envelope
(start inclusive, end exclusive). A wrapping page program reports the envelope
of its latched bytes; sector/block erases report the aligned range, and chip
erase reports `[000000,1000000)`. WRSR changes no array bytes and is not an array
configuration-write attempt.

The warning says plainly **“this would overwrite the board's FPGA configuration
(bitstream) on a real ULX3S”** and includes the opcode and range. It appears in
`Status` (count + first event), a separate CLI `peripheral: flash warning: …`
line, a `# flash warning: …` record note, and a visible yellow Flash-panel line.
Warnings and their counts also rewind with checkpoints.

## Board safety: SRAM loads only

**The emulator's model of program and erase is not a licence to run such a
design on a board.** On a real ULX3S the flash at address zero holds the FPGA's
own configuration memory. Programming or erasing it can wipe the board's
bitstream. The project's rule remains **SRAM loads only; never write, erase or
change the status of the board's flash**. No board run was performed for this model update.

All reference Verilog remains read-only, with the existing forbidden-opcode
audit unchanged. New program/erase tests feed SPI pins from Rust only.

## Status and reports

`status()` / `View::flash` retain JEDEC, selection, power-down, status register,
last command/address, bytes read and version, plus the new counters.
`programs` / `erases` count completed accepted commands, not bytes or sectors;
WRSR is not a program. `protected` counts BP-rejected array commands;
`busy_ignored` counts forbidden commands received while WIP=1. `ignored_writes`
now counts rejected write/control commands (unsupported legacy write opcodes,
malformed commands, no WEL, BP protection or busy), at most once per command.
Accepted WREN, WRDI, WRSR, programs and erases do not count as ignored.

The report preserves the previous fields and their order, appending:
`programs=N erases=N protected=N busy_ignored=N config_writes=N`.
The optional first warning is on its own line after the summary.

## Read-only reference designs

**The real board's configuration flash must never be written, erased, or have
its status written.** Both designs share
[flash_reader.v](../examples/ulx3s/flash_dump/flash_reader.v). Its hardware can
only select three constant commands: `9F`, `05`, `03`. There is no generic
command register and CPU stores to flash never start a transaction. SCK is
**12.5 MHz (25 MHz / 2)** through USRMCLK. nWP and nHOLD stay high.

- [flash_dump](../examples/ulx3s/flash_dump/) prints `FLASH ID 9D6018 SR 00`,
  then 32-byte lines `AAAAAA: <64 uppercase hex digits>` over [000000,010000)
  and [200000,201000), then `CRC <8 uppercase hex digits>` over both windows in
  that order. The CRC is ISO-HDLC, initial/final XOR FFFFFFFF. After one second
  it repeats. This is the read-only check of an existing board image.
- [picorv32_flash](../examples/ulx3s/picorv32_flash/) boots in EBR and prints
  `PicoRV32 flash ID 9D6018`. It checks the header at flash 200000 and bounds the
  payload to 4..65536 bytes, aligned to four bytes. It validates its CRC before
  jumping to **1020000C**, executing in place through a read-only memory mapping
  at **10000000** (one cached word). The firmware counts primes below 2000 and
  reports `pass N: primes below 2000: 303, C cycles` each pass. The sieve, stack
  and boot ROM use 8 KB of EBR. With an invalid header/CRC it instead prints
  `no firmware at 200000: <first 16 bytes>` once per second. UART is 115200 8N1,
  `ftdi_rxd` L4, in both designs.

Build the designs with `examples/ulx3s/build.sh`. The PicoRV32 source is a
symlink to `picorv32_console/picorv32.v`, as in `picorv32_sdram`. Its
`firmware/build.sh` uses RISC-V clang and lld, writes the four committed EBR
byte lanes and the committed flash payload hex. No flash programmer is used.

## Checks and speed

Plain `cargo test` runs the Verilog command audit without FPGA tools. It scans
every `.v` in both designs, following the shared controller, UART and PicoRV32
symlinks. Command constants (CMD/command/opcode and write/erase/program mnemonics) must be
literal 03/0B/05/9F; binary, octal, decimal and hex literals are understood.
WREN/WRDI, program, erase, WRSR, OTP and protection opcodes are forbidden; the
audit also has a test with deliberately forbidden literals in different bases.

Unit tests cover both SPI polarities, IDs/status, reads/dummy clocks/wrapping,
padding, power-down/reset, unsupported commands and unknown inputs, WEL/WRDI,
page wrapping/AND-only programming, all erase aliases/sizes, WIP deadlines,
busy filtering, BP/WRSR, configuration warnings (including rejected attempts),
and copy-on-write snapshot/restore cost across programs and erases.
`tests/spiflash.rs` (ignored: clang, Yosys/nextpnr, Trellis) checks the CCLK
pad probe, every dump line and CRC for known windows, two XIP prime passes with the
demo image, and the missing-header fallback. It uses fast mode's answering loop
and reports cycles/s excluding build, loading and C compilation.
`tests/live_peripherals.rs` runs the demo through the live runner and verifies
identical UART text after going back before boot. The additional read-only CLI
smoke test uses the existing XIP design to check the unchanged report prefix and
zero write counters. A Rust-pad test checks the warning line through
`Attached::report` and rewind, and an ordinary CLI help test documents it.
No writing FPGA design is created: a real CLI run cannot emit a configuration
write warning using these intentionally read-only controllers. No board is needed.

On rserv (2026-11-16), the standalone answering tests measured:

| Design / image | Board cycles | Wall time | Board cycles/s |
|---|---:|---:|---:|
| flash_dump, known windows | 349,824,571 | 109.253 s | 3,201,981 |
| picorv32_flash, demo, two passes | 17,174,708 | 23.323 s | 736,388 |
| picorv32_flash, erased image, fallback | 182,607 | 0.279 s | 654,996 |

The dump matched every line (2,176 lines of 32 bytes), including:

```text
FLASH ID 9D6018 SR 00
000000: 00254A6F94B9DE03284D7297BCE1062B50759ABFE4092E53789DC2E70C31567B
200FE0: 80A5CAEF14395E83A8CDF2173C6186ABD0F51A3F6489AED3F81D42678CB1D6FB
CRC 9DEE7942
```

The erased image printed:

```text
PicoRV32 flash ID 9D6018
no firmware at 200000: FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF
```

The routed designs pass 25 MHz timing: nextpnr reported 109.18 MHz for
flash_dump and 95.21 MHz for picorv32_flash (seed 1). These are emulator
checks; no board was accessed.

## On the board

[HR07](inventory/LFE5U-85F/hr07_flash/README.md), 2026-11-16 (SRAM loads only;
nothing was written to the flash):
- `flash_dump` printed `FLASH ID 9D6018 SR 18` (IS25LP128F, with block-protect
  bits `BP1` and `BP2` set) and `CRC 055E6742` across two complete passes over
  `0x000000`–`0x00FFFF` and `0x200000`–`0x200FFF`.
- `tests/spiflash.rs` `the_board_dump_reproduces_in_the_emulator` ran
  `flash_dump` in fast mode on the reconstructed 16 MB image with `SR 0x18` and
  matched every line (ID/SR, every window line, and CRC; 349,824,571 cycles at
  4.44 MHz on `rserv`).
- `picorv32_flash` ran its read-only fallback (`no firmware at 200000:`) on both
  loads, since nothing is written to the board's flash; in the emulator the
  demo image executes in place (2 passes of `primes below 2000: 303, 5849188 cycles`
  at 0.74 MHz).

## Board check (HR07)

To keep the real board's configuration flash contents out of git, the raw UART
dump from the board is never committed:

- **Dump location:** `out/flash/board_dump.log` (git-ignored under `out/`),
  containing the output of
  [run.py](inventory/LFE5U-85F/hr04_hdmi_oled/run.py) for `flash_dump`
  (`load N …` headers followed by two-space-indented UART lines).
- **Reproduction check:** `tests/spiflash.rs`
  `the_board_dump_reproduces_in_the_emulator` reads `out/flash/board_dump.log`,
  reconstructs a 16 MB image in memory from the first complete dump (the two
  windows `[000000,010000)` and `[200000,201000)`, `FF` elsewhere) without
  writing the dump or image anywhere under git, runs `flash_dump` in fast mode
  on that image, and asserts exact agreement on the `FLASH ID`/`SR` line, every
  window line and the `CRC` line. When `out/flash/board_dump.log` is missing,
  the test prints where to place it and returns without failing.
- **What HR07 records instead:** only non-sensitive verification metadata — the
  JEDEC ID, the window addresses and lengths, the `CRC` over those windows, the
  SHA-256 of the dump, and the comparison result.
