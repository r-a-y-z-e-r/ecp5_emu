# riscv_terminal

A computer you can see. This demo joins riscv_shell and video_life: the same
PicoRV32 RV32IMC system with its UART shell, plus an HDMI monitor that shows
everything the shell prints, like a terminal of the 1980s. Type `mandel` on
the UART console and the picture grows on the monitor at the same time as it
scrolls out of the serial port. It is the gallery's flagship: a complete small
computer — CPU, RAM, keyboard (the UART), and a text screen — built from two
EBR memories, one font ROM and a pipeline of shift registers.

For a student it shows how a real text terminal works: a character buffer the
CPU writes, a font ROM the display reads, a hardware cursor, and the same
memory-mapped I/O style as the UART and LEDs, just at video speed.

## What it shows

- 640 × 480 @ 60 Hz over the ULX3S's GPDI (HDMI) connector: 80 columns × 30
  rows of 8 × 16 pixel characters, amber on near-black.
- Every byte the firmware prints goes to the UART **and** to the screen:
  echo, CR/LF, backspace, wrap at column 80, scrolling. Scrolling does not
  copy the buffer: the CPU raises the hardware *top row* register and the
  display wraps around.
- A blinking block cursor (~2 Hz from the frame counter) that inverts the
  cell under it; the firmware moves it by writing one register.
- The `screen` shell command: hides the cursor, waits two complete frames,
  and prints the CRC-32 of the frame — a fingerprint of exactly what is on
  the monitor.

## Controls

- UART (115200 8N1, the window's console or a terminal): the whole
  riscv_shell command set — `help`, `info`, `led`, `buttons`, `knight`,
  `primes`, `mandel`, `calc`, `peek`, `poke`, `dump`, `memtest`, `uptime`,
  `reboot` — plus:
  - `screen` — print `screen crc <8 hex>`, the CRC-32/ISO-HDLC of the last
    complete frame, cursor hidden so the value depends only on the text;
  - `cls` — clear the screen.
- Buttons: as in riscv_shell, at the prompt every debounced press prints an event
  line such as `[button FIRE1]`. The LEDs are the `led` command's.

## Memory map

| Address | Meaning |
| --- | --- |
| `0x0000_0000`–`0x0000_7fff` | 32 KB RAM in 16 EBRs, firmware swapped by ecpbram |
| `0x0200_0000` | UART data (write: transmit; read: take from the 511-byte RX FIFO) |
| `0x0200_0004` | UART status |
| `0x0300_0000` | LEDs |
| `0x0400_0000` | button levels; `+4` press events (read clears) |
| `0x0500_0000` | reset register |
| `0x0600_0000`–`0x0600_095f` | character buffer: 2 400 bytes, one per cell, row-major |
| `0x0600_1000` | top row (scrolling offset) |
| `0x0600_1004` | cursor: bit 16 = on, bits 12:8 = row, bits 6:0 = column |
| `0x0600_1008` | CRC-32 of the last complete frame |
| `0x0600_100c` | number of the last complete frame |

Everything but the TMDS shift register runs in the 25 MHz pixel clock; the
CPU is held in reset until the PLL locks. The character buffer is two EBRs
(2 048 + 352 bytes) with one port for the CPU's byte stores and one for the
display. PicoRV32 aligns store addresses down to the word boundary, so the
write logic adds back the position of the single `mem_wstrb` lane; only
single-lane (byte) strobes write a cell. As in riscv_shell, a display read
that collides with a CPU write of the same byte is skipped for one clock —
during a `screen` measurement the CPU writes nothing, so no checked frame can
contain a stale cell.

## The font

`font.hex` holds 95 original glyphs (printable ASCII 32–126), 8 × 16 pixels,
one byte per row, bit 7 leftmost. They were drawn by hand in `make_font.py` —
no font file from anywhere — as plain sans-serif bitmaps with 1-pixel
strokes, caps on rows 2–12 and descenders to row 14. Rerun
`python3 make_font.py` to regenerate; `--preview` prints the glyphs as text.

## The frame CRC, and how the screen is checked

The display pipeline feeds every active pixel's R, G and B bytes, in raster
order, into the same parallel CRC-32/ISO-HDLC as video_life. At pixel
(639, 479) the CRC and the frame number are latched into the registers above.

- The emulator's Monitor window decodes the GPDI pads and computes the same
  CRC per frame (`# monitor frame N WxH crc XXXXXXXX`).
- `sim/render.py` is an independent reference: it replays `expected/uart.txt`
  through a model of the terminal, renders the 80 × 30 text with `font.hex`,
  and computes the CRC the same way. It reproduces both `screen crc` values
  in the check script (run it any time: `python3 sim/render.py`).
- On the board, `screen` prints the CRC of whatever is on the monitor; with
  the cursor hidden it depends only on the bytes received, so it is the same
  on the board, in the emulator and in the RTL simulation. (The frame number
  is deliberately not printed: when a command runs depends on when its bytes
  arrive.)

## Resources and Fmax

Measured by nextpnr-ecp5 0.11.1 (seed 1) on the LFE5U-85F:

| Resource | Used |
| --- | --- |
| LUT4 (TRELLIS_COMB) | 5 170 / 83 640 (6 %) |
| TRELLIS_FF | 1 498 / 83 640 (1 %) |
| DP16KD (EBR) | 20 / 208 (9 %) — 16 firmware, 2 cells, 1 font, 1 RX FIFO |
| MULT18X18D | 4 / 156 (2 %) |
| EHXPLLL | 1 |

Fmax after routing: 57.6 MHz for the 25 MHz pixel-clock domain (the whole system), and
295 MHz for the 125 MHz TMDS shift clock. Both pass their constraints.

## Build, check, run

```sh
examples/ulx3s/build.sh examples/gallery/riscv_terminal target/gallery/riscv_terminal
python3 examples/gallery/check.py riscv_terminal --build --emulator
```

The check runs 11 000 000 cycles. The emulator printed the same 2 837 bytes from `reboot` on as
the RTL simulation, at 0.21–0.25 MHz on the remote Linux machine.

**In the window**, open the demo and watch the Monitor window. The banner appears, then
[gui.script](gui.script) types `help`, `led 5a`, `mandel` and `screen`, and presses FIRE1.

```sh
ECP5_GUI_SCRIPT=examples/gallery/riscv_terminal/gui.script \
  target/release/ecp5_gui target/gallery/riscv_terminal/top.config
```

The window's record of one run (`ECP5_GUI_TRACE`, summed up by [gui_run.py](../gui_run.py)):
- 90 s at 60 frames per second (median frame 16.6 ms);
- 0.30 MHz with the HDMI receiver attached;
- the receiver decoded 61 frames from the GPDI pads, all 640 × 480.

The console's text ends with:

```text
> screen
screen crc ea168d99
>
[button FIRE1]
```

The frame with CRC `EA168D99` is among the frames the receiver decoded. So the screen the
emulated pads carried is the screen the design computed its CRC over: text, font and colours,
pixel for pixel. A second run printed the same CRC.

**On the board**, connect a monitor to the HDMI connector and a terminal to the USB-serial port at
115200, then type `help`.

## Changing the firmware

`firmware/main.c` is riscv_shell's shell with the terminal added
(`screen_putc`, scrolling via the top-row register, `screen` and `cls`).
Rebuild with the Homebrew RISC-V LLVM:

```sh
sh firmware/build.sh    # writes the 16 fw_<bank>_<lane>.hex images
```

The images are committed, and the build is reproducible byte for byte. `swap.sh` puts them into
the built bitstream with ecpbram ([ecpbram.txt](ecpbram.txt) lists them), so only a change to the
Verilog needs place and route again:

```sh
examples/ulx3s/swap.sh examples/gallery/riscv_terminal target/gallery/riscv_terminal
```

## Expected output (excerpt)

```
> mandel
                    .........................
                ......................*-.........
             ........................::=--+.........
...
> screen
screen crc c44adea5
> cls
> screen
screen crc 2cd64349
```

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 2 837 bytes from `reboot` on. That
includes both `screen crc` lines, `c44adea5` and `2cd64349`, which the board's own CRC
hardware computed over the frames it sent to the HDMI connector; `sim/render.py` gives the same two
values from the text alone. The monitor itself was not filmed. Records are in
[docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
