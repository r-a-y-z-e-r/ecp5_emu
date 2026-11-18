#!/usr/bin/env python3
"""locality.py PROGRAM.c : how local each thread's registers are in a generated fast-mode program.

For each part of phase 0: the registers its lines read, the cache lines (128 bytes) they are on,
and how many of those registers and lines belong to another thread, a register belonging to the
part that computes its next value (the part writing what its capture reads; the caller for the
rest), as owner capture would have it. Fewer foreign lines: less data moving between cores a cycle.
"""
import re
import sys
from collections import defaultdict

src = open(sys.argv[1]).read().split("\n")
body = {}
cur = None
for l in src:
    m = re.match(r"^.*void (piece\d+)\(u8 \*restrict v\) \{$", l)
    if m:
        cur = m.group(1)
        body[cur] = []
        continue
    if cur:
        if l.startswith("}"):
            cur = None
        else:
            body[cur].append(l)


def fn(name):
    i = [k for k, l in enumerate(src) if l.startswith(name)][0]
    out = []
    for l in src[i + 1:]:
        if l.startswith("}"):
            break
        m = re.match(r"^\s+(piece\d+)\(v\);", l)
        if m:
            out += body[m.group(1)]
        elif "pool_run" in l:
            break
    return out


def out_slot(l):
    m = re.search(r"; v\[(\d+)\] =", l) if l.lstrip().startswith("{") else re.match(r"^\s*v\[(\d+)\]\s*=", l)
    return int(m.group(1)) if m else None


def reads(l):
    s = set(int(x) for x in re.findall(r"v\[(\d+)\]", l))
    s.discard(out_slot(l))
    return s


threads = int(re.search(r"#define THREADS (\d+)", "\n".join(src)).group(1))
conv = [k for k, l in enumerate(src) if l.startswith("static int converge")][0]
cap = []
for l in src[conv:]:
    if l.startswith("}"):
        break
    m = re.match(r"^\s+(piece\d+)\(v\);", l)
    if m:
        cap += body[m.group(1)]
parts = [fn(f"static void settle_group0_p0_t{t}(") for t in range(threads)]
writer = {}
for t, ls in enumerate(parts):
    for l in ls:
        o = out_slot(l)
        if o is not None:
            writer[o] = t
owner = {}
for l in cap:
    q = out_slot(l)
    if q is None:
        continue
    ws = {writer[s] for s in reads(l) if s in writer}
    owner[q] = ws.pop() if len(ws) == 1 else 0
line_owners = defaultdict(set)
for q, t in owner.items():
    line_owners[q // 128].add(t)
mixed = sum(1 for o in line_owners.values() if len(o) > 1)
print(f"{len(owner)} registers on {len(line_owners)} lines, {mixed} lines holding registers of several threads")
tf = tl = 0
for t, ls in enumerate(parts):
    r = set()
    for l in ls:
        r |= reads(l)
    regs = {s for s in r if s in owner}
    foreign = {s for s in regs if owner[s] != t}
    lines = {s // 128 for s in regs}
    flines = {s // 128 for s in foreign}
    tf += len(foreign)
    tl += len(flines)
    print(f"t{t}: {len(ls)} lines, reads {len(regs)} registers on {len(lines)} lines; "
          f"another thread's: {len(foreign)} registers on {len(flines)} lines")
print(f"total another thread's: {tf} registers, {tl} lines")
