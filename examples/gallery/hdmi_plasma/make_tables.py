#!/usr/bin/env python3
"""Reproduce the original sine and palette ROM assets; no external inputs."""
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
PALETTES = [
    [(7,10,40), (24,44,112), (13,130,159), (71,222,181),
     (237,246,168), (250,157,93), (162,54,112), (55,20,84)],
    [(18,12,25), (83,22,55), (190,47,49), (246,134,44),
     (255,231,158), (157,195,155), (30,117,126), (35,45,78)],
    [(18,9,47), (43,41,149), (17,153,220), (152,247,239),
     (251,215,243), (229,80,177), (108,27,119), (40,21,84)],
]

def main():
    sine = [math.floor(255 * math.sin(2 * math.pi * p / 1024) + 0.5)
            for p in range(1024)]
    (HERE / 'sine.hex').write_text(''.join(f'{s & 511:03x}\n' for s in sine))
    colors = []
    for anchors in PALETTES:
        for i in range(256):
            k, w = divmod(i, 32)
            colors.append(tuple(((32-w)*a + w*b)//32
                                for a, b in zip(anchors[k], anchors[(k+1)%8])))
    colors += [(0,0,0)] * 256
    (HERE / 'palette.hex').write_text(''.join(
        f'{r:02x}{g:02x}{b:02x}\n' for r,g,b in colors))

if __name__ == '__main__':
    main()
