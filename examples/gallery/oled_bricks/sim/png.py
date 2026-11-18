#!/usr/bin/env python3
"""Nearest-neighbour x4; RGB565 expansion matches SSD1331. Stdlib only."""
import struct
import sys
import zlib
from pathlib import Path
raw = Path(sys.argv[1]).read_bytes()
assert len(raw) == 96 * 64 * 2
rows = []
for y in range(64):
    row = bytearray()
    for x in range(96):
        p = int.from_bytes(raw[(y * 96 + x) * 2:][:2], 'big')
        r, g, b = p >> 11, (p >> 5) & 63, p & 31
        row.extend(bytes(((r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2))) * 4)
    rows.extend([b'\0' + row] * 4)
def chunk(kind, content):
    return struct.pack('>I', len(content)) + kind + content + struct.pack('>I', zlib.crc32(kind + content))
Path(sys.argv[2]).write_bytes(b'\x89PNG\r\n\x1a\n' +
    chunk(b'IHDR', struct.pack('>IIBBBBB', 384, 256, 8, 2, 0, 0, 0)) +
    chunk(b'IDAT', zlib.compress(b''.join(rows), 9)) + chunk(b'IEND', b''))
