"""Fit HS02 preload and empirical release-timing models, per FF and setting.

Usage: hs02_analyse.py observations.log [--legacy]; ... --selftest
Default (NumPy): M/REGSET, sync/async-level/assertion-only/assertion-latched/
clocked-M, LSR over/under CE, explicit enable hypotheses, and old/new/AND/OR M
on release. Enable overrides diagnose the CEMUX=1/routed-CE configuration
collision; they are not universal CEMUX=1 hardware semantics. Release functions
fit coincident M/LSR transitions and are not universal asynchronous timing rules.
--legacy retains the original nominal-CEMUX and preload models.

Accept raw P lines or runner distinct=[...] records; deduplicate loads, count
invalid fragments, reject empty input. Sample zero is not assumed by fitting.
Assertion-latched models explicitly count unknown initial state as unpredicted.
"""
import argparse
import ast
from pathlib import Path
import re
import itertools
from hs02_gen import SEED, SETTINGS, step, traffic

MODELS = list(itertools.product(("M", "REGSET"), ("sync", "async-level", "async-assert"), ("over", "under")))


def read_lines(path):
    lines, bad, loads = set(), 0, []
    for row in Path(path).read_text().splitlines():
        if "distinct=" in row:
            candidates = ast.literal_eval(row.partition("distinct=")[2])
        else:
            candidates = [row.strip()]
        if "distinct=" in row:
            loads.append({line.upper() for line in candidates if re.fullmatch(r'P[0-9A-Fa-f]{1040}', line)})
        for line in candidates:
            if re.fullmatch(r'P[0-9A-Fa-f]{1040}', line):
                lines.add(line.upper())
            elif line.startswith("P"):
                bad += 1
    if not lines:
        raise ValueError("no complete HS02 lines")
    print(f"distinct windows={len(lines)}, ignored incomplete/invalid lines={bad}")
    if loads:
        print(f"loads={len(loads)}; full windows/load={[len(x) for x in loads]}; identical={all(x==loads[0] for x in loads)}")
    return sorted(lines)


def samples(line):
    if not re.fullmatch(r'P[0-9A-Fa-f]{1040}', line):
        raise ValueError("expected P + 16 seed hex + 1024 Q hex")
    x = int(line[1:17], 16)
    out = []
    for k in range(128):
        q = int(line[17 + 8*k:25 + 8*k], 16)
        out.append((traffic(x), q))
        x = step(x)
    return out


def predict(q, a, b, setting, model):
    source, timing, priority = model
    _, _, regset, cemux = setting
    da, ma, ca, la = a
    _, mb, cb, lb = b
    ea, eb = (True, True) if cemux == "1" else (ca, cb)
    va, vb = (ma, mb) if source == "M" else (int(regset == "SET"),) * 2
    if la and (priority == "over" or ea):
        q = va
    elif not la and ea:
        q = da
    if timing != "sync" and lb and (priority == "over" or eb):
        if timing == "async-level" or not la:
            q = vb
    return q


def score(windows, i, model):
    miss = total = 0
    for w in windows:
        for a, b in zip(w, w[1:]):
            want = predict((a[1] >> i) & 1, a[0][i], b[0][i], SETTINGS[i], model)
            miss += want != ((b[1] >> i) & 1)
            total += 1
    return miss, total


def best(scores):
    minimum = min(v[0] for v in scores.values())
    return minimum, [m for m, v in scores.items() if v[0] == minimum]


def describe(m):
    return f"{m[0]}, {m[1]}, LSR {m[2]} CE"


def legacy_analyse(lines):
    windows = [samples(line) for line in lines]
    scores = [{m: score(windows, i, m) for m in MODELS} for i in range(32)]
    for i, setting in enumerate(SETTINGS):
        miss, models = best(scores[i])
        print(f"ff{i:02} {'/'.join(setting)}: {miss}/{next(iter(scores[i].values()))[1]} mismatches; "
              + "; ".join(map(describe, models)))
    for setting in dict.fromkeys(SETTINGS):
        indices = [i for i, s in enumerate(SETTINGS) if s == setting]
        grouped = {m: (sum(scores[i][m][0] for i in indices), sum(scores[i][m][1] for i in indices)) for m in MODELS}
        miss, models = best(grouped)
        print(f"group {'/'.join(setting)} FFs={indices}: {miss}/{next(iter(grouped.values()))[1]} mismatches; "
              + "; ".join(map(describe, models)) + ("; NO EXACT MODEL" if miss else ""))
    for line in lines:
        if int(line[1:17], 16) == SEED:
            print(f"configuration sample Q[31:0]={line[17:25]}")


def analyse(lines):
    import hs02_models
    hs02_models.analyse(lines, samples)


def selftest():
    # Independent trace generator: propagate the hidden Q, never observed Q.
    for known in MODELS:
        lines = []
        for seed in (SEED, 0xA913B7481256EFC0, 0x71CA0385B2D946EF):
            x, q, words = seed, 0x96A55A69, []
            for _ in range(128):
                words.append(f"{q:08X}")
                a, b = traffic(x), traffic(step(x))
                nxt = 0
                for i, setting in enumerate(SETTINGS):
                    d, m, ce, lsr = a[i]
                    _, m2, ce2, lsr2 = b[i]
                    ce = ce or setting[3] == "1"
                    ce2 = ce2 or setting[3] == "1"
                    value = (q >> i) & 1
                    if lsr:
                        if ce or known[2] == "over":
                            value = m if known[0] == "M" else int(setting[2] == "SET")
                    elif ce:
                        value = d
                    if lsr2 and (ce2 or known[2] == "over"):
                        if known[1] == "async-level" or (known[1] == "async-assert" and not lsr):
                            value = m2 if known[0] == "M" else int(setting[2] == "SET")
                    nxt |= value << i
                q, x = nxt, step(x)
            lines.append("P" + f"{seed:016X}" + "".join(words))
        windows = [samples(line) for line in lines]
        for i in range(32):
            scores = {m: score(windows, i, m) for m in MODELS}
            miss, found = best(scores)
            assert miss == 0 and known in found, (known, i, scores)
            # The source must be identifiable. Finite traces can leave priority
            # tied; REGSET also makes both asynchronous variants equivalent.
            if SETTINGS[i][3] == "CE":
                assert all(m[0] == known[0] for m in found), (known, i, found)
        broken = lines[0][:-1]
        try:
            samples(broken)
        except ValueError:
            pass
        else:
            raise AssertionError("accepted a truncated window")
    print("HS02 selftest: 12 known models, 32 FFs, three synthetic lines each: PASS")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", nargs="?")
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--legacy", action="store_true", help="original nominal CEMUX and async models")
    args = parser.parse_args()
    if args.selftest:
        selftest()
        import hs02_models
        hs02_models.selftest(samples)
    elif args.log:
        (legacy_analyse if args.legacy else analyse)(read_lines(args.log))
    else:
        parser.error("supply a log or --selftest")


if __name__ == "__main__":
    main()
