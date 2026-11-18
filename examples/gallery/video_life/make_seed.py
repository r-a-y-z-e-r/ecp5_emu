#!/usr/bin/env python3
"""Place mathematical Life patterns on a 64 by 48 torus; no borrowed code."""
from pathlib import Path
rows = [0]*48
def place(x,y,pattern):
    for dy,row in enumerate(pattern.split('/')):
        for dx,c in enumerate(row):
            if c=='#': rows[(y+dy)%48] |= 1 << ((x+dx)%64)
# Gosper's period-30 gun, with space to its southeast for the outgoing stream.
place(2,2,'........................#.........../'
          '......................#.#.........../'
          '............##......##............##/'
          '...........#...#....##............##/'
          '##........#.....#...##............../'
          '##........#...#.##....#.#.........../'
          '..........#.....#.......#.........../'
          '...........#...#..................../'
          '............##......................')
place(43,20,'.#./..#/###')
place(11,31,'.#./..#/###')
place(43,38,'.##/##./.#.') # R-pentomino
Path(__file__).with_name('seed.hex').write_text(''.join(f'{r:016x}\n' for r in rows))
print('seed population',sum(r.bit_count() for r in rows))
