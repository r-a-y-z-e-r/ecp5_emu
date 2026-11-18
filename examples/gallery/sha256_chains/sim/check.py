#!/usr/bin/env python3
"""Independent content oracle: hashlib supplies every expected chain value."""
import hashlib
import json
from pathlib import Path
import re
import sys


def require(ok, detail):
    if not ok:
        raise SystemExit('FAIL: ' + detail)

prefix = Path(sys.argv[1])
raw = prefix.with_suffix('.txt').read_bytes()
config = json.loads((prefix.parent/'model.json').read_text())
cores, every, steps = (config[k] for k in ('CORES', 'STEP_EVERY', 'STEPS'))
seed = b'Parallel chain seed / '
long_message = b'abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq'
previous = {}
software = {}
count = 0
# Check every hardware-computed link, including values between UART reports.
for line in prefix.with_suffix('.values').read_text().splitlines():
    epoch, p, i, k, h = line.split()
    epoch, p, i, k = int(epoch), int(p,16), int(i,16), int(k,16)
    key = (epoch,p,k)
    require(0 <= k < cores, f'core outside configured range: {line}')
    if i == 0:
        require(key not in previous, f'duplicate seed: {line}')
        expected = hashlib.sha256(seed + k.to_bytes(4,'big') + p.to_bytes(4,'big')).digest()
    else:
        require(key in previous and previous[key][0] == i-1, f'link skipped/repeated: {line}')
        expected = hashlib.sha256(previous[key][1]).digest()
    require(h.upper() == expected.hex().upper(), f'epoch {epoch}, pass {p}, step {i}, core {k}: hardware {h} != hashlib {expected.hex()}')
    previous[key] = (i,expected)
    software[(epoch,p,i,k)] = expected
    count += 1

events = [e.split() for e in prefix.with_suffix('.events').read_text().splitlines()]
require(events[-1][0] == 'END', 'missing end counters')
epoch = int(events[-1][1])
controls = len(sys.argv) == 2
late = len(sys.argv) > 2 and sys.argv[2] == '--late'
require(epoch == (5 if controls else 2), f'restart coverage differs: {epoch}')
require(b'RESTART\r\n' in raw, 'no complete marker')
if late:
    require(raw.count(b'RESTART') == 1, 'late r must produce exactly one marker in the entire UART stream')
    prefix_text = raw.split(b'RESTART\r\n',1)[0]
    require(prefix_text.startswith(b'KAT abc ') and b'\r\nKAT 448 ' in prefix_text,
            'power-on must start both KAT lines before the sole marker')
text = raw.rsplit(b'RESTART\r\n',1)[1]
require(text.endswith(b'\r\n'), 'partial final line')
lines = text.decode('ascii').splitlines()
kat = [f'KAT {name} {hashlib.sha256(m).hexdigest().upper()} OK' for name,m in [('abc',b'abc'),('448',long_message)]]
require(lines[:2] == kat, f'KAT output differs: {lines[:2]}')
chain_events = [(int(e[2]),int(e[3])) for e in events if e[0]=='CHAIN' and int(e[1])==epoch]
p, i, k = 0, every, 0
rounds, summaries = 0, []
for line in lines[2:]:
    m = re.fullmatch(r'STEP ([0-9A-F]{8}) ([0-9A-F]{2}) ([0-9A-F]{64})', line)
    if m:
        require(i <= steps, f'expected CHAIN before another STEP: {line}')
        require((int(m[1],16),int(m[2],16)) == (i,k), f'missing/reordered STEP: {line}, expected {i:08X} {k:02X}')
        expected = software.get((epoch,p,i,k))
        require(expected is not None and m[3] == expected.hex().upper(), f'STEP differs from hashlib: {line}')
        k += 1
        if k == cores:
            rounds += 1
            k = 0
            i += every
        continue
    m = re.fullmatch(r'CHAIN ([0-9A-F]{8}) STEPS ([0-9A-F]{8}) DIGEST ([0-9A-F]{64}) CYCLES ([0-9A-F]{16})', line)
    require(m is not None, f'unrecognized line: {line!r}')
    require(i > steps and k == 0, f'CHAIN preceded final STEP round: {line}')
    require((int(m[1],16),int(m[2],16)) == (p,steps), f'pass/length differs: {line}')
    final_values = b''.join(software[(epoch,p,steps,lane)] for lane in range(cores))
    expected = hashlib.sha256(final_values).hexdigest().upper()
    require(m[3] == expected, f'CHAIN digest {m[3]} != hashlib {expected}')
    summaries.append((p,int(m[4],16)))
    p += 1
    i, k = every, 0
require(summaries == chain_events[:len(summaries)], f'CYCLES differ from measured edges: {summaries} != {chain_events}')
require(rounds >= 3 and len(summaries) >= 1 and k == 0, 'window requires three complete STEP rounds and a CHAIN')
if late:
    require(b'RESTART\r\n'+text == Path(sys.argv[3]).read_bytes(),
            'late-r transcript from the sole marker differs byte for byte from expected UART')
    print('Late-r marker contract PASS: both power-on KAT lines began, exactly one marker, byte-identical expected UART')
elif not controls:
    Path(sys.argv[2]).write_bytes(b'RESTART\r\n'+text)
print(f'hashlib PASS: all {count} hardware values; {rounds} ordered STEP rounds, {len(summaries)} CHAIN digests, both KATs and cycle counts')
