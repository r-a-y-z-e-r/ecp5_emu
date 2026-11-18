import re, sys
from collections import defaultdict
c = sys.argv[1]
src = open(c).read().split("\n")
body = {}; cur = None
for l in src:
    m = re.match(r"^__attribute__\(\(.*\)\) void (piece\d+)\(u8", l)
    if m: cur = m.group(1); body[cur] = []; continue
    if cur:
        if l.startswith("}"): cur = None
        else: body[cur].append(l)
def fn(name):
    i = [k for k, l in enumerate(src) if l.startswith(name)][0]; out = []
    for l in src[i + 1:]:
        if l.startswith("}"): break
        m = re.match(r"^\s+(piece\d+)\(v\);", l)
        if m: out += body[m.group(1)]
    return out
def out_slot(l):
    if l.lstrip().startswith("{"): return int(re.search(r"; v\[(\d+)\] =", l).group(1))
    return int(re.match(r"^\s*v\[(\d+)\]", l).group(1))
def reads(l):
    s = set(int(x) for x in re.findall(r"v\[(\d+)\]", l)); s.discard(out_slot(l)); return s
conv = [k for k, l in enumerate(src) if l.startswith("static int converge")][0]
cap = []
for l in src[conv:]:
    if l.startswith("}"): break
    m = re.match(r"^\s+(piece\d+)\(v\);", l)
    if m: cap += body[m.group(1)]
Q = set(out_slot(l) for l in cap)
n = int(re.search(r"#define THREADS (\d+)", "\n".join(src)).group(1))
writes = {}; readsets = {}
for t in range(n):
    ls = fn(f"static void settle_group0_p0_t{t}(")
    writes[t] = set(out_slot(l) for l in ls)
    r = set()
    for l in ls: r |= reads(l)
    readsets[t] = r
line_writer = defaultdict(set)
for t in range(n):
    for s in writes[t]: line_writer[s // 128].add(t)
tot = 0
for t in range(n):
    # lines this thread reads that another thread writes during phase 0
    bad = set(s // 128 for s in readsets[t] if any(w != t for w in line_writer.get(s // 128, ())))
    qbad = set(s // 128 for s in readsets[t] & Q if any(w != t for w in line_writer.get(s // 128, ())))
    tot += len(bad)
    print(f"t{t}: reads {len(readsets[t])} slots; lines written by another part during phase 0: {len(bad)} (holding registers it reads: {len(qbad)})")
print("total", tot)
