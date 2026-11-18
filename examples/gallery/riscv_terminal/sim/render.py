#!/usr/bin/env python3
"""Independent check of the `screen crc` lines of riscv_terminal.

Everything the shell prints goes to the UART and to the screen, so the bytes
in expected/uart.txt are also the bytes the screen received. This script is a
terminal: it replays those bytes onto a model of the 80 x 30 screen (CR, LF,
backspace, wrap at column 80, scrolling), then, at each `screen crc XXXXXXXX`
line the design printed, renders the screen as it stood just before that
line — the glyphs of font.hex, amber on near-black, no cursor (the command
hides it) — and computes the CRC-32 of the 640 x 480 frame exactly as the
hardware computes it (R, G, B of each pixel in raster order, CRC-32/ISO-HDLC,
which is zlib's crc32). The two numbers must be equal.

Usage: sim/render.py [expected/uart.txt]
"""
import sys
import zlib
from pathlib import Path

FG = (0xFF, 0xB0, 0x00)  # amber
BG = (0x0C, 0x0C, 0x0C)  # near-black
COLS, ROWS = 80, 30


class Terminal:
    """The firmware's screen model: what cell holds what code."""

    def __init__(self):
        self.grid = [[0x20] * COLS for _ in range(ROWS)]
        self.col = 0
        self.row = 0

    def scroll(self):
        self.grid.pop(0)
        self.grid.append([0x20] * COLS)

    def put(self, b):
        if b == 0x0D:  # CR
            self.col = 0
        elif b == 0x0A:  # LF
            if self.row == ROWS - 1:
                self.scroll()
            else:
                self.row += 1
        elif b == 0x08:  # BS
            if self.col:
                self.col -= 1
        elif 0x20 <= b < 0x7F:
            self.grid[self.row][self.col] = b
            self.col += 1
            if self.col == COLS:
                self.col = 0
                if self.row == ROWS - 1:
                    self.scroll()
                else:
                    self.row += 1

    def frame_crc(self, font):
        """The CRC of the 640 x 480 frame: each cell's glyph row, bit 7 left."""
        stream = bytearray()
        for y in range(480):
            row, line = divmod(y, 16)
            for col in range(COLS):
                code = self.grid[row][col]
                glyph = font[(code - 0x20) * 16 + line] if 0x20 <= code < 0x7F else 0
                for bit in range(8):
                    stream += bytes(FG if glyph >> (7 - bit) & 1 else BG)
        return zlib.crc32(stream) & 0xFFFFFFFF


def main():
    folder = Path(__file__).resolve().parents[1]
    data = Path(sys.argv[1] if len(sys.argv) > 1 else folder / "expected/uart.txt").read_bytes()
    font_hex = (folder / "font.hex").read_text().split()
    font = [int(h, 16) for h in font_hex]
    assert len(font) == 2048
    term = Terminal()
    checks = 0
    for i, b in enumerate(data):
        if data[i:i + 11] == b"screen crc " and data[i + 19:i + 21] == b"\r\n":
            claimed = int(data[i + 11:i + 19], 16)
            rendered = term.frame_crc(font)
            status = "OK" if rendered == claimed else "MISMATCH"
            print(f"byte {i}: screen crc {claimed:08x}, rendered {rendered:08x}: {status}")
            checks += 1
            if rendered != claimed:
                sys.exit(1)
        term.put(b)
        # cls and the boot code (also run by reboot) clear the screen through
        # MMIO alone: nothing in the UART stream shows it. The echo of the
        # command line is the signal to clear the model too.
        if data[i - 6:i + 1] == b"> cls\r\n" or data[i - 9:i + 1] == b"> reboot\r\n":
            term.__init__()
    print(f"{checks} screen CRCs checked, all OK")
    if checks < 2:
        sys.exit("wanted at least two screen crc lines")


if __name__ == "__main__":
    main()
