#!/usr/bin/env python3
"""Independent integer instrument, score and mixer model from README.

No Verilog parsing, generated score import, phase/envelope probes or RTL CRC code.
Only the DAC/LED/SPI/UART pads are observed by tb.cpp. Python uses zlib CRC32.
"""
from pathlib import Path
import bisect
import re
import statistics
import sys
import zlib

BASS = [48,0,48,0,55,0,48,0, 45,0,45,0,52,0,45,0,
        53,0,53,0,60,0,53,0, 43,0,50,0,55,0,43,0]
LEAD = [72,76,74,67,69,0,74,76, 72,69,67,64,67,0,69,74,
        77,76,72,69,72,79,76,0, 74,67,71,74,79,76,74,0]
HARM = [60,0,64,0,67,0,64,0, 57,0,60,0,64,0,60,0,
        60,0,65,0,69,0,65,0, 59,0,62,0,67,0,62,0]
SINE = [0,3,6,8,11,12,14,15,15,15,14,12,11,8,6,3,
        0,-3,-6,-8,-11,-12,-14,-15,-15,-15,-14,-12,-11,-8,-6,-3]
SUSTAIN = [144,128,96,176]
KEYS = dict(zip('asdfgh', [72,74,76,77,79,81]))

class Synth:
    def __init__(self):
        self.phase = [0]*4
        self.inc = [0]*4
        self.env = [0]*4
        self.stage = ['R']*4
        self.n = self.step = self.beat = self.age = self.note = 0
        self.paused = False
        self.mix = (0,0)

    def start(self, voice, note):
        self.phase[voice] = self.env[voice] = 0
        self.inc[voice] = round(440 * 2**((note-69)/12) * 2**24 / 25000)
        self.stage[voice] = 'A'

    def sample(self, key=''):
        p = [v >> 19 for v in self.phase]
        wave = [2*p[0]-15 if p[0]<16 else 47-2*p[0],
                15 if p[1]<8 else -15, p[2]-16, SINE[p[3]]]
        a = [w*e for w,e in zip(wave,self.env)]
        l, r = 2*a[0]+2*a[1]+a[2]+2*a[3], 2*a[0]+a[1]+2*a[2]+2*a[3]
        self.mix = (l,r)
        result = (max(4,min(12,8+(l+1024)//2048)),
                  max(4,min(12,8+(r+1024)//2048)))
        self.phase = [(p+i) % 2**24 for p,i in zip(self.phase,self.inc)]
        if self.n % 16 == 0:
            for v in range(4):
                e, s, sustain = self.env[v], self.stage[v], SUSTAIN[v]
                if s == 'R': self.env[v] = max(0,e-2)
                elif s == 'A':
                    self.env[v] = min(255,e+4)
                    if e >= 251: self.stage[v] = 'D'
                elif s == 'D':
                    self.env[v] = max(sustain,e-1)
                    if e <= sustain+1: self.stage[v] = 'S'
                else: self.env[v] = sustain
        if not self.paused:
            if self.beat == 0:
                for v, track in enumerate((BASS,LEAD,HARM)):
                    note = track[self.step]
                    if note: self.start(v,note)
                    else: self.stage[v] = 'R'
                self.note = LEAD[self.step]
            if self.beat == 4687: self.stage[:3] = ['R']*3
            self.beat += 1
            if self.beat == 6250:
                self.beat = 0
                self.step = (self.step+1) % 32
        if self.age:
            if self.age == 1563: self.stage[3] = 'R'
            self.age -= 1
        if key in KEYS:
            self.start(3,KEYS[key]); self.age = 6250; self.note = KEYS[key]
        if key == 'p': self.paused = not self.paused
        self.n += 1
        return result


def segments(path):
    result = [[]]
    for line in path.read_text().splitlines():
        if line.startswith('RESTART '): result.append([])
        else: result[-1].append(tuple(map(int,line.split())))
    return [s for s in result if s]


def check_plain(out, name):
    rows = segments(out / (name+'.samples'))[-1]
    synth = Synth()
    data = bytearray()
    for n, (cycle,l,r) in enumerate(rows):
        want = synth.sample()
        assert (l,r) == want, f'{name}: sample {n} at {cycle}: RTL {(l,r)} != Python {want}'
        data.extend(want)
    printed = re.findall(rb'^AUDIO 3125 ([0-9A-F]{8})\r$', (out/(name+'.uart')).read_bytes(), re.M)
    # Late restart includes a pre-marker frame, but no pre-marker audio block.
    want = [f'{zlib.crc32(data[i:i+6250]):08X}'.encode()
            for i in range(0,len(data)-6249,6250)]
    assert printed == want, f'{name}: model AUDIO CRC list differs: {printed!r} != {want!r}'
    print(f'{name}: Python == RTL for {len(rows)} sample pairs / {len(want)} AUDIO blocks: PASS')
    return rows


def check_controls(out):
    rows = segments(out/'controls.samples')[-1]
    times = [row[0] for row in rows]
    uart = (out/'controls.uart').read_bytes()
    ons = [(int(n,16),int(m)) for m,n in re.findall(
        rb'^NOTE ([0-9]{3}) ([0-9A-F]{8})\r$',uart,re.M)]
    assert [m for n,m in ons] == list(KEYS.values())*3, 'live note order'
    actions = {n: next(k for k,v in KEYS.items() if v==m) for n,m in ons}
    event_rows = [line.split() for line in (out/'controls.events').read_text().splitlines()]
    pauses = [int(e[1]) for e in event_rows if e[0]=='PAUSE']
    assert len(pauses)==2, 'pause/resume must both reach LED6'
    for cycle in pauses:
        n = bisect.bisect_left(times,cycle)
        assert times[n]==cycle and n not in actions, 'pause must be on sample edge'
        actions[n]='p'
    hosts = [(int(e[1]),e[2]) for e in event_rows if e[0]=='UART' and e[2] in KEYS]
    for i,(start,key) in enumerate(hosts):
        onset = times[ons[i][0]]
        assert onset>=start+2060, 'note began before UART stop bit'
        assert onset-start < (3100 if i<6 else 400000), 'UART note not promptly latched'
    buttons = [int(e[1]) for e in event_rows if e[0]=='BUTTON']
    for (n,m), start in zip(ons[12:], buttons):
        assert 25000 <= times[n]-start < 27000, '1ms button edge lost or misfiltered'
    pause_hosts = [int(e[1]) for e in event_rows if e[0]=='UART' and e[2]=='p']
    assert all(2060<=cycle-start<3100 for cycle,start in zip(pauses,pause_hosts))
    synth=Synth(); data=bytearray()
    for n,(cycle,l,r) in enumerate(rows):
        want=synth.sample(actions.get(n,''))
        assert (l,r)==want, f'controls sample {n}: RTL {(l,r)} != Python {want}'
        data.extend(want)
    printed=re.findall(rb'^AUDIO 3125 ([0-9A-F]{8})\r$',uart,re.M)
    expected=[f'{zlib.crc32(data[i:i+6250]):08X}'.encode()
              for i in range(0,len(data)-6249,6250)]
    assert printed==expected, 'live model AUDIO CRCs differ'
    print(f'Live sine, 18 onsets, button filter/latch, queued UART and pause/resume: '
          f'Python == RTL for {len(rows)} pairs: PASS')

# Original 3x5 font, as specified by the drawing in README/render.v.
FONT={
 '0':'111101101101111', '1':'010110010010111', '2':'110001111100111',
 '3':'110001010001110', '4':'101101111001001', '5':'111100110001110',
 '6':'011100111101111', '7':'111001010010010', '8':'111101111101111',
 '9':'111101111001110', 'N':'101111111111101', 'T':'111010010010010',
 'S':'011100010001110', 'P':'110101110100100'}

def render(history, step, note, paused=False):
    result=bytearray()
    peak=max(abs(v) for v in history)
    def trace_row(value):
        height=22*abs(value)//peak if peak else 0
        return 38+height if value<0 else 38-height
    text=f'N{note:03d}  T120  S{step:02d}   '+('P' if paused else ' ')
    for y in range(64):
        for x in range(96):
            c=0x0863
            yy=trace_row(history[x])
            prev=trace_row(history[max(0,x-1)])
            if y>=13:
                if y==38 or x in (0,95) or y in (13,63): c=0x2269
                elif x%16==0 and y%4==0: c=0x1946
                if min(yy,prev)<=y<=max(yy,prev): c=0xbfe9
                if y==yy: c=0xeff4
            if 3<=x<79 and 3<=y<8:
                col,xx=divmod(x-3,4)
                glyph=FONT.get(text[col],'0'*15)
                if xx<3 and glyph[(y-3)*3+xx]=='1': c=0xfcd3
            if y==10 and 3<=x<3+(step%8+1)*10: c=0x4bd7
            result.extend(c.to_bytes(2,'big'))
    return result


def check_scope(out,rows):
    times=[row[0] for row in rows]
    synth=Synth(); states=[(0,0)]; mixes=[]
    for _ in rows:
        synth.sample();states.append((synth.step,synth.note));mixes.append(synth.mix[0])
    count=0
    for line in (out/'base.frames').read_text().splitlines():
        f,end,snapshot,crc=line.split(); f=int(f); snapshot=int(snapshot)
        # The snapshot takes the old history if a sample coincides with it.
        n=bisect.bisect_left(times,snapshot)
        history=([0]*96+mixes[:n])[-96:]
        step,note=states[n]
        want=render(history,step,note)
        have=(out/f'base.frame{f}.rgb565').read_bytes()
        assert have==want, f'frame {f}: drawing/history disagrees with independent model'
        assert zlib.crc32(want)==int(crc,16), f'frame {f}: model CRC differs'
        count+=1
    assert count==40
    print(f'All {count} OLED pictures == independent history/font renderer: PASS')


def report_levels(out, rows):
    # Exactly the first 8-second score loop, including rests and note attacks.
    rows=rows[:200000]
    assert len(rows)==200000
    synth=Synth(); raw=[[],[]]
    for _ in rows:
        synth.sample()
        for ch in range(2): raw[ch].append(synth.mix[ch])
    lines=['First loop: 200000 pairs at 25 kHz (8 s), no live keys.',
           'Typical = median peak-to-peak over all 320 non-overlapping 25 ms windows, including rests.']
    for ch in range(2):
        codes=[row[ch+1] for row in rows]
        pp=[max(codes[i:i+625])-min(codes[i:i+625]) for i in range(0,len(codes),625)]
        ordered=sorted(codes)
        clipped=sum(not 4<=8+(v+1024)//2048<=12 for v in raw[ch])
        lines.append(f'{"LR"[ch]}: min={min(codes)} max={max(codes)} '
                     f'typical_pp={statistics.median(pp):g} codes; '
                     f'p05={ordered[9999]} p95={ordered[189999]}; '
                     f'AC_RMS={statistics.pstdev(codes):.3f} code; '
                     f'clamped={clipped}/200000 ({clipped/2000:.4f}%); '
                     f'raw_min={min(raw[ch])} raw_max={max(raw[ch])}')
        assert min(codes)==4 and max(codes)==12, 'loop must use the safe limits'
        assert statistics.median(pp)>=4, 'typical passage must span several DAC codes'
    (out/'levels.txt').write_text('\n'.join(lines)+'\n')
    print('\n'.join(lines))


def main():
    out = Path(sys.argv[1])
    base = check_plain(out,'base')
    check_plain(out,'repeat')
    check_plain(out,'late')
    check_plain(out,'power')
    loop = check_plain(out,'loop')
    check_controls(out)
    check_scope(out,base)
    assert len(loop)>200000, 'must cross the 8-second tune wrap'
    late=(out/'late.uart').read_bytes().split(b'RESTART\r\n',1)[1]
    golden=(out/'base.uart').read_bytes().split(b'RESTART\r\n',1)[1]
    assert golden.startswith(late), 'late restart is not the same deterministic prefix'
    assert (out/'base.uart').read_bytes()==(out/'repeat.uart').read_bytes()
    report_levels(out,loop)
    print('Full tune, loop boundary, late restart and byte-identical rerun: PASS')

if __name__ == '__main__': main()
