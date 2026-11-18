"""Check the HD02 record against the ALU54B operation model.
Usage: python3 hd02_model.py observations.log

Model (R is the output register, captured once per step):
  X = OP[1:0]: 0 R (the register itself), 1 MA, 2 A, 3 zero
  Y = OP[3:2]: 0 MB << 18, 1 MB, 2 B, 3 zero
  Z = OP[6:4]: 0 zero, 1 CIN >> 18, 2 CIN, 3 C, 4 A, 5 R, 6 RNDPAT, 7 zero
  OP[10:9]: 1 R = (OP8 ? -Z : Z) + X + (OP7 ? -Y : Y) modulo 2^54;
            0, 2, 3 bitwise, X unused: with Y' = OP7 ? ~Y : Y and
            Z' = OP8 ? ~Z : Z, R = ~(Y' & Z'), Y' & Z', ~(Y' ^ Z').
  MA and MB are the 36-bit products, sign-extended to 54 bits when the
  multiplier's SIGNEDP (SIGNEDA | SIGNEDB) is 1, zero-extended otherwise;
  A = {ROB, ROA} of the first multiplier and B of the second.
This design, the first of the series, connects little around the ALU, so
three things read as zero here and are measured in HD03 and HD04:
- CIN: no ALU stands before this one (the first block of its row);
- the top 18 bits of A and of B, which are the C inputs of the multipliers;
- C[17:0] and C[44:27], which also come through those C inputs: the ALU's own
  pins carry only C[26:18] and C[53:45]."""
import re, sys

M54 = (1 << 54) - 1
RNDPAT = 0x0123456789ABC5
C_ARRIVES = 0x3FE00007FC0000


def lfsr_steps(count):
    l, mask = 1, (1 << 168) - 1
    for _ in range(count):
        for _ in range(168):
            l = (l << 1 | (l >> 167 ^ l >> 165 ^ l >> 152 ^ l >> 150) & 1) & mask
        yield l


def sx(v, bits, signed):
    return v - (1 << bits) if signed and v >> (bits - 1) & 1 else v


def alu(op, x, y, z):
    """The operation on three 54-bit sources."""
    if op >> 9 & 3 == 1:
        return ((-z if op >> 8 & 1 else z) + x + (-y if op >> 7 & 1 else y)) & M54
    y ^= M54 * (op >> 7 & 1)
    z ^= M54 * (op >> 8 & 1)
    return {0: ~(y & z), 2: y & z, 3: ~(y ^ z)}[op >> 9 & 3] & M54


def sources(op, s, r, cin=0, c=None):
    sa0, sb0, sa1, sb1 = (s["sg"] >> k & 1 for k in range(4))
    ma = sx((sx(s["a0"], 18, sa0) * sx(s["b0"], 18, sb0)) & (1 << 36) - 1, 36, sa0 | sb0) & M54
    mb = sx((sx(s["a1"], 18, sa1) * sx(s["b1"], 18, sb1)) & (1 << 36) - 1, 36, sa1 | sb1) & M54
    a, b = s["b0"] << 18 | s["a0"], s["b1"] << 18 | s["a1"]
    c = s["c"] if c is None else c
    return ([r, ma, a, 0][op & 3], [mb << 18 & M54, mb, b, 0][op >> 2 & 3],
            [0, cin >> 18, cin, c, a, r, RNDPAT, 0][op >> 4 & 7])


def parse(item, n_digits):
    v = int(item[1:], 16)
    s = {}
    for name, bits in [("f", 12), ("r", 56), ("c", 56), ("b1", 20), ("a1", 20), ("b0", 20), ("a0", 20), ("sg", 4)]:
        s[name] = v & (1 << bits) - 1
        v >>= bits
    s["n"] = v
    return s


def main():
    text = open(sys.argv[1]).read()
    digits = 56 if re.search(r"'T[0-9A-F]{56}'", text) else 55
    loads = [set(re.findall(r"'(T[0-9A-F]{%d})'" % digits, line)) for line in text.splitlines()]
    lines = set.union(*loads)
    print(f"{len(loads)} loads, {[len(l) for l in loads]} distinct lines, {len(lines)} in all")
    samples = [parse(item, digits) for item in lines]
    # The operands are the LFSR's: that also gives the step when the record
    # prints only its low 12 bits (the first build's line was 4 bits short).
    by_operands = {}
    for n, l in enumerate(lfsr_steps(16384)):
        key = (l >> 126 & 15, l & 0x3FFFF, l >> 18 & 0x3FFFF, l >> 36 & 0x3FFFF, l >> 54 & 0x3FFFF, l >> 72 & M54)
        by_operands[key] = n
    steps = {}
    for s in samples:
        n = by_operands.get((s["sg"], s["a0"], s["b0"], s["a1"], s["b1"], s["c"]))
        assert n is not None, "operands are not the LFSR's"
        assert s["n"] == n & ((1 << 14) - 1 if digits == 56 else 0xFFF), "step number"
        assert n not in steps, "two lines for one step"
        steps[n] = s
    assert len(steps) == 16384
    bad, flags = [], set()
    for n in range(16384):
        s, r = steps[n], steps[(n - 1) % 16384]["r"]
        op = n >> 3
        x, y, z = sources(op, s, r, c=s["c"] & C_ARRIVES)
        if alu(op, x, y, z) != s["r"]:
            bad.append(n)
        flags.add(s["f"])
    print(f"16384 steps (2048 opcodes x 8), R differs from the model in {len(bad)} {bad[:8]}")
    print(f"flag field values seen: {sorted(hex(f) for f in flags)}")


if __name__ == "__main__":
    main()
