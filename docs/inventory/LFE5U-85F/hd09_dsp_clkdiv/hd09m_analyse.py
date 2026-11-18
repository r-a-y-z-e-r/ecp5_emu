"""Fit HD09m output timing or HD09p's isolated MULT18 stages (--probe).

Usage: hd09m_analyse.py observations_m.log [--probe] [--legacy]; ... --selftest
Default (NumPy): known HD07/HD09p controls; rising/falling/rise+fall, all /2,/4
phases, data latency 0..8, constant/frozen/transparent/GSR-only hypotheses.
Predictions condition on the preceding observed Q; print mismatch counts per bit.
Phases are searched per window and shared by the recorded bits of each cell.
--legacy retains the complete original CE/RST/reset-kind/GSR search.
"""
import argparse
import ast
import itertools
from pathlib import Path
import re


def read_lines(path, digits):
    lines, bad, loads = set(), 0, []
    for row in Path(path).read_text().splitlines():
        items = ast.literal_eval(row.partition("distinct=")[2]) if "distinct=" in row else [row.strip()]
        if "distinct=" in row:
            loads.append({item.upper() for item in items if re.fullmatch(r'M[0-9A-Fa-f]{' + str(digits) + '}', item)})
        for item in items:
            if re.fullmatch(r'M[0-9A-Fa-f]{' + str(digits) + '}', item):
                lines.add(item.upper())
            elif item.startswith("M"):
                bad += 1
    if not lines:
        raise ValueError("no complete M windows")
    print(f"distinct windows={len(lines)}, ignored incomplete/invalid lines={bad}")
    if loads:
        print(f"loads={len(loads)}; full windows/load={[len(x) for x in loads]}; identical={all(x==loads[0] for x in loads)}")
    return sorted(lines)


ALUS = [("ea", "a"), ("eb", "a"), ("ec", "b"), ("ed", "b")]
MULTS = [("ma", "b"), ("mb", "b"), ("mc", "b"), ("md", "b")]
ALU_BITS = [50, 30, 20, 0]
MULT_BITS = [33, 18, 16, 1]
MULT_DATA = {33: 16, 18: 1, 16: 16, 1: 1}
GSR_KINDS = ["ignored", "async", "async, no hold", "sync"]
RATES = [(1, None), (2, None), (2, "g"), (0, None)]
MODELS = [(rate, dres, ce, rst, kind, gsr)
          for (rate, dres), ce, rst, kind, gsr in itertools.product(
              RATES, [None, 0, 1, 2, 3], [None, 0, 1, 2, 3], ["sync", "async"], GSR_KINDS)
          if rst is not None or kind == "sync"]


def samples(line):
    if not re.fullmatch(r'M[0-9A-Fa-f]{3840}', line):
        raise ValueError("expected M + 3840 hex")
    bits = f"{int(line[1:], 16):015360b}"
    out = []
    for i in range(256):
        w = bits[60*i:60*i+60]
        def v(a, b):
            return int(w[a:b], 2)
        s = dict(cea=v(0, 4), rsta=v(4, 8), ceb=v(8, 12), rstb=v(12, 16), gsr=1-v(19, 20))
        s["c"] = {50: v(20, 21), 30: v(21, 22), 20: v(22, 23), 0: v(23, 24), 16: v(42, 43), 1: v(43, 44)}
        for n, (name, _) in enumerate(ALUS):
            s[name] = {b: v(24+4*n+k, 25+4*n+k) for k, b in enumerate(ALU_BITS)}
        for n, (name, _) in enumerate(MULTS):
            s[name] = {b: v(44+4*n+k, 45+4*n+k) for k, b in enumerate(MULT_BITS)}
        out.append(s)
    return out


def next_q(q, d, a, b, model, state, block):
    rate, _, ce, rst, kind, gsr = model
    active = rate == 1 or (rate == 2 and state == 0)
    r = rst is not None and (a["rst"+block] >> rst & 1)
    r2 = rst is not None and (b["rst"+block] >> rst & 1)
    g = {"ignored": False, "async": a["gsr"] or b["gsr"],
         "async, no hold": b["gsr"], "sync": active and a["gsr"]}[gsr]
    if g or (kind == "async" and (r or r2)) or (active and r):
        return 0
    if active and (ce is None or a["ce"+block] >> ce & 1):
        return d
    return q


def next_phase(state, a, model):
    rate, dres = model[:2]
    return 0 if rate != 2 or (dres == "g" and a["gsr"]) else state ^ 1


def phases(windows, name, block, bit, data, model):
    found = []
    for w in windows:
        valid = []
        for phase in ((0, 1) if model[0] == 2 else (0,)):
            state = phase
            for a, b in zip(w, w[1:]):
                if next_q(a[name][bit], a["c"][data], a, b, model, state, block) != b[name][bit]:
                    break
                state = next_phase(state, a, model)
            else:
                valid.append(phase)
        if not valid:
            return None
        found.append(tuple(valid))
    return found


def rate_name(model):
    rate, dres = model[:2]
    return {0: "never", 1: "every edge", 2: "every other edge"}[rate] + (", GSR resets divider" if dres else "")


def describe(model):
    _, _, ce, rst, kind, gsr = model
    return (f"{rate_name(model)}, CE{ce if ce is not None else ' none'}, "
            f"RST{rst if rst is not None else ' none'} {kind}, GSR {gsr}")


def phase_name(found):
    return "phases per window=" + repr(found)


def legacy_analyse(lines, probe=False):
    windows = [samples(x) for x in lines]
    print("window IDs (phase lists use this order):")
    for i, line in enumerate(lines):
        print(f"  {i}: {line[:49]}...")
    for name, block in ALUS + MULTS:
        bits = ALU_BITS if name.startswith("e") else MULT_BITS
        reg = "ALU OUTPUT" if name.startswith("e") else (dict(ma="INPUTA", mb="INPUTB", mc="PIPELINE", md="OUTPUT")[name] if probe else "OUTPUT")
        for bit in reversed(bits):
            data = bit if name.startswith("e") else MULT_DATA[bit]
            found = [(m, p) for m in MODELS if (p := phases(windows, name, block, bit, data, m)) is not None]
            print(f"{name} {reg} bit {bit}: " + ("NO MODEL" if not found else ""))
            for m, p in found:
                print("  " + describe(m) + ("; " + phase_name(p) if m[0] == 2 else ""))


def analyse(lines, probe=False):
    from hd09_timing import report_direct
    windows = [samples(x) for x in lines]
    print(f"Unique windows={len(windows)}, samples={256*len(windows)}, transitions per bit={255*len(windows)}")
    print("Controls follow HD07/HD09p; mismatch counts condition on observed preceding Q.")
    print("Search: rising/falling/rise+fall, /2 and /4 with every phase, D latency 0..8,")
    print("       frozen/constants, combinational/CE-level latch, GSR-only changes.")
    print("Divided phases are selected per window, shared by all bits of a cell.")
    if len(lines) == 1:
        print("First observed output nibbles:", {n: ''.join(str(windows[0][0][n][b]) for b in (ALU_BITS if n.startswith('e') else MULT_BITS)) for n,_ in ALUS+MULTS})
    for name, block in ALUS+MULTS:
        bits = ALU_BITS if name.startswith("e") else MULT_BITS
        data = bits if name.startswith("e") else [MULT_DATA[b] for b in bits]
        reg = "ALU OUTPUT" if name.startswith("e") else (dict(ma="INPUTA", mb="INPUTB", mc="PIPELINE", md="OUTPUT")[name] if probe else "MULT18 OUTPUT")
        print(f"\n{name} {reg}")
        report_direct(windows, name, block, bits, data, probe)
    print("\nInference: rise+fall is a sample-level model; high-phase transparency cannot be excluded by pre-rising-edge observations.")


def selftest():
    import random
    rng = random.Random(909)
    for rate, dres in RATES:
        for phase in ((0, 1) if rate == 2 else (0,)):
            model = (rate, dres, 2, 1, "async", "async")
            rows, q, state = [], 0, phase
            for _ in range(256):
                rows.append(dict(cea=rng.randrange(16), ceb=rng.randrange(16), rsta=rng.randrange(16),
                                 rstb=sum((rng.randrange(8) == 0) << b for b in range(4)),
                                 gsr=int(rng.randrange(32) == 0), c={b: rng.randrange(2) for b in (50, 30, 20, 0, 16, 1)}))
            for a in rows:
                for name, _ in ALUS:
                    a[name] = dict.fromkeys(ALU_BITS, 0)
                for name, _ in MULTS:
                    a[name] = dict.fromkeys(MULT_BITS, 0)
            for k, a in enumerate(rows):
                a["ma"][1] = q
                if k != 255:
                    q = next_q(q, a["c"][1], a, rows[k+1], model, state, "b")
                    state = next_phase(state, a, model)
            # Serialize a full HD07-layout line and exercise the real parser.
            chunks = []
            for a in rows:
                bits = f'{a["cea"]:04b}{a["rsta"]:04b}{a["ceb"]:04b}{a["rstb"]:04b}000{1-a["gsr"]}'
                bits += ''.join(str(a["c"][b]) for b in ALU_BITS)
                bits += '0' * 16 + '00' + str(a["c"][16]) + str(a["c"][1])
                bits += ''.join(str(a[n][b]) for n, _ in MULTS for b in MULT_BITS)
                chunks.append(f'{int(bits, 2):015X}')
            parsed = [samples("M" + ''.join(chunks))]
            found = [m for m in MODELS if phases(parsed, "ma", "b", 1, 1, m) is not None]
            assert model in found, (model, found)
            assert {m[0] for m in found} == {rate}, (model, found)
            assert phase in phases(parsed, "ma", "b", 1, 1, model)[0]
    print("HD09m/p selftest: every edge, both divided phases, GSR divider reset, never; full synthetic lines: PASS")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?")
    parser.add_argument("--probe", action="store_true")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--legacy", action="store_true", help="run the original HD07 control/rate search")
    args = parser.parse_args()
    if args.selftest:
        selftest()
        from hd09_timing import selftest as timing_selftest
        timing_selftest()
    elif args.log:
        (legacy_analyse if args.legacy else analyse)(read_lines(args.log, 3840), args.probe)
    else:
        parser.error("supply a log or --selftest")


if __name__ == "__main__":
    main()
