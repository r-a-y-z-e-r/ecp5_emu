#!/usr/bin/env python3
"""Independent mathematical renderer of README.md's fixed-point definition.

No RTL, ROM file, or simulator-internal values are read. UART events replay
the public state machine; zlib checks every complete reported frame. PNG
writing uses struct/zlib alone, and samples every second pixel (320x240).
"""
import argparse
import math
from pathlib import Path
import re
import struct
import zlib

HERE = Path(__file__).resolve().parent
SINE = tuple(math.floor(255*math.sin(2*math.pi*p/1024)+0.5) for p in range(1024))
# These are the published RGB anchors, independently interpolated here.
ANCHORS = (
    ((7,10,40),(24,44,112),(13,130,159),(71,222,181),
     (237,246,168),(250,157,93),(162,54,112),(55,20,84)),
    ((18,12,25),(83,22,55),(190,47,49),(246,134,44),
     (255,231,158),(157,195,155),(30,117,126),(35,45,78)),
    ((18,9,47),(43,41,149),(17,153,220),(152,247,239),
     (251,215,243),(229,80,177),(108,27,119),(40,21,84)),
)

def palette_bytes(p):
    anchors = ANCHORS[p]
    return tuple(bytes((a*(32-i%32) + b*(i%32))//32
                       for a,b in zip(anchors[i//32], anchors[(i//32+1)%8]))
                 for i in range(256))

COLORS = tuple(palette_bytes(p) for p in range(3))

def render(effect, palette, tick):
    """Evaluate a whole field, without reproducing hardware pipeline stages."""
    assert effect in (0,1) and palette in (0,1,2) and 0 <= tick <= 65535
    colors = COLORS[palette]
    drift = tick//4
    sin_t, cos_t = SINE[tick%1024], SINE[(tick+256)%1024]
    rows = []
    if effect == 0:
        horizontal = [SINE[(3*x+2*tick)%1024] for x in range(640)]
        squares = [(x-320)**2 for x in range(640)]
        diagonal = [SINE[(2*d+tick+sin_t//2)%1024] for d in range(1119)]
        for y in range(480):
            vertical = SINE[(4*y-3*tick)%1024]
            r2_y = (y-240)**2
            rows.append(b''.join(colors[((horizontal[x] + vertical + diagonal[x+y] +
                SINE[((squares[x]+r2_y)//128-2*tick)%1024] + 1020)//8 + drift)%256]
                for x in range(640)))
    else:
        zoom = 192 + sin_t//4
        dark = tuple(bytes(c//8 for c in color) for color in colors)
        shine = tuple(bytes(112+c//2 for c in color) for color in colors)
        for y in range(480):
            v = y-240
            line = []
            for x in range(640):
                u = x-320
                # Floors at BOTH Q8 reductions are part of the definition.
                a = (((u*cos_t-v*sin_t)//256)*zoom//256 + tick//4)%256
                b = (((u*sin_t+v*cos_t)//256)*zoom//256 - tick//8)%256
                f, g = a%64, b%64
                index = (5*min(f,63-f) + 3*min(g,63-g) +
                         64*((a//64 ^ b//64)&1) + drift)%256
                radius = abs(f-32)+abs(g-32)
                bank = dark if f<2 or g<2 else shine if 29<=radius<=31 else colors
                line.append(bank[index])
            rows.append(b''.join(line))
    return b''.join(rows)

def write_png(path, pixels):
    def chunk(tag, data):
        return struct.pack('>I',len(data))+tag+data+struct.pack('>I',zlib.crc32(tag+data))
    scanlines = b''.join(b'\0' + b''.join(pixels[(y*640+x)*3:(y*640+x)*3+3]
                         for x in range(0,640,2)) for y in range(0,480,2))
    path.write_bytes(b'\x89PNG\r\n\x1a\n' +
                     chunk(b'IHDR',struct.pack('>IIBBBBB',320,240,8,2,0,0,0)) +
                     chunk(b'IDAT',zlib.compress(scanlines,9)) + chunk(b'IEND',b''))

FRAME = re.compile(r'F([0-9A-F]{4}) E([01]) P([012]) C([0-9A-F]{8})')
EVENT = re.compile(r'SET E([01]) P([012]) S([0-3]) H([01])')

def check_transcripts(paths):
    cache = {}
    count = [0,0]
    negative_count = [0,0]
    states = set()
    for path in paths:
        n = tick = effect = palette = paused = 0
        speed = 1
        markers = 0
        total = 0
        events = []
        for line in path.read_text().splitlines():
            if line == 'RESTART':
                n = tick = effect = palette = paused = 0
                speed = 1
                markers += 1
                continue
            match = EVENT.fullmatch(line)
            if match:
                new = tuple(map(int,match.groups()))
                assert new != (effect,palette,speed,paused), f'{path}: redundant SET'
                effect,palette,speed,paused = new
                events.append(new)
                continue
            match = FRAME.fullmatch(line)
            assert match, f'{path}: malformed line: {line!r}'
            fn,e,p,c = match.groups()
            assert (int(fn,16),int(e),int(p)) == (n,effect,palette), \
                f'{path}: nonsequential frame or state: {line}, expected F{n:04X} E{effect} P{palette}'
            key = effect,palette,tick
            if key not in cache:
                cache[key] = zlib.crc32(render(*key))
            want = cache[key]
            assert int(c,16) == want, \
                f'{path}: {line}, T={tick}: software CRC {want:08X} != RTL {c}'
            count[effect] += 1
            if tick%1024 >= 512:
                negative_count[effect] += 1
            total += 1
            states.add((effect,palette,speed,paused,(tick%1024)//256))
            n = (n+1)%65536
            if not paused:
                tick = (tick+(1<<speed))%65536
            if total%20 == 0:
                print(f'Model: {path.name}: {total} frames matched', flush=True)
        assert markers, f'{path}: missing RESTART'
        if path.name == 'controls_uart.txt':
            # Explicitly verify the P18 pulses, FIFO burst order, clamp,
            # pause/resume and restart; CRC equality alone could miss a
            # button pulse that the RTL forgot entirely.
            expected = [(1,0,1,0),(1,2,1,0),
                        (1,0,1,0),(1,1,1,0),(1,2,1,0),
                        (1,2,2,0),(1,2,3,0),(1,2,3,1),(1,2,3,0),
                        (1,0,3,0),(1,0,2,0),(1,0,3,0),(0,0,3,0),
                        (0,0,3,1),(0,0,3,0),(0,0,3,1),(0,0,3,0),
                        (1,0,3,0),(0,0,3,0)]
            assert events == expected, f'lost/extra/out-of-order controls: {events}'
            assert markers == 2, 'restart during UART transmission lost'
        print(f'Model: {path.name}: all {total} CRCs equal ({markers} restart markers)', flush=True)
    assert min(count)>=5, f'need at least five frames of each effect: {count}'
    assert min(negative_count)>=5, f'need five negative-sine frames per effect: {negative_count}'
    print(f'Model-vs-RTL PASS: A={count[0]}, B={count[1]} frames; '
          f'{len(cache)} distinct pictures, {len(states)} setting/quadrant combinations')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('transcripts',type=Path,nargs='*')
    parser.add_argument('--effect',type=int,choices=(0,1),default=0)
    parser.add_argument('--palette',type=int,choices=(0,1,2),default=0)
    parser.add_argument('--frame',type=int,default=37,help='constant-speed frame number')
    parser.add_argument('--speed',type=int,choices=range(4),default=1)
    args = parser.parse_args()
    if args.transcripts:
        check_transcripts(args.transcripts)
    else:
        tick = (args.frame*(1<<args.speed))%65536
        pixels = render(args.effect,args.palette,tick)
        print(f'F{args.frame%65536:04X} E{args.effect} P{args.palette} '
              f'C{zlib.crc32(pixels):08X} (T={tick})')
    write_png(HERE/'frame_a.png',render(0,0,74))
    write_png(HERE/'frame_b.png',render(1,2,144))

if __name__ == '__main__':
    main()
