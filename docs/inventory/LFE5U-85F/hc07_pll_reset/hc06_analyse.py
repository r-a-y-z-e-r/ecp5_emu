"""Tabulates the HC06 lines (format in hc06_gen.py) by PLL and pulse width.

Usage: python3 hc06_analyse.py observations.log
       python3 hc06_analyse.py b observations_b.log

The runner prints, per load, the distinct lines it read; every distinct line
is one event here (a repeated event with equal fields counts once).
"""
import ast
import collections
import sys

PLLS = ["pa", "pb", "pc", "pd"]
FIELDS = [("drop", 5), ("during", 4), ("level", 1), ("first", 5), ("stable", 5),
          ("lock", 5), ("bmin", 3), ("bmax", 3), ("ph0", 2), ("ph1", 2)]


VARIANT = ""


def parse(line):
    per = 72 if VARIANT == "b" else 35
    if len(line) != 1 + 5 + per * 4:
        return None  # cut by the end of the reading window
    width = int(line[1:6], 16)
    pos = 6
    plls = {}
    for name in PLLS:
        rec = {}
        for field, n in FIELDS:
            rec[field] = int(line[pos:pos + n], 16)
            pos += n
        rec["lost"] = (rec["ph1"] - rec["ph0"]) % 64
        if VARIANT == "b":
            rec["lrise"] = int(line[pos], 16)
            pos += 1
            rec["ramp"] = [int(line[pos + 3 * i:pos + 3 * i + 3], 16) for i in range(12)]
            pos += 36
        plls[name] = rec
    return width, plls


def events(path):
    for row in open(path):
        if not row.startswith("load "):
            continue
        load = int(row.split()[1])
        for line in ast.literal_eval(row[row.index("distinct=") + 9:].strip()):
            event = parse(line)
            if event:
                yield load, event


def summary(values):
    values = sorted(values)
    if not values:
        return "-"
    if values[0] == values[-1]:
        return f"{values[0]}"
    mid = values[len(values) // 2]
    return f"{values[0]}..{values[-1]} (median {mid})"


def main(path):
    by = collections.defaultdict(list)
    loads = collections.Counter()
    for load, (width, plls) in events(path):
        loads[load] += 1
        by[width].append(plls)
    print(f"events per load: {dict(loads)}")
    for name in PLLS:
        print(f"\n== {name}")
        for width in sorted(by):
            recs = [p[name] for p in by[width]]
            cols = []
            extra = ["lrise"] if VARIANT == "b" else []
            for field in ["drop", "during", "level", "first", "stable", "lock", "bmin", "bmax", "lost"] + extra:
                vals = [r[field] for r in recs]
                if field == "drop":
                    vals = ["none" if v == 0xFFFFF else v for v in vals]
                    none = sum(1 for v in vals if v == "none")
                    nums = [v for v in vals if v != "none"]
                    text = (f"none x{none}" if none else "") + (
                        (", " if none and nums else "") + summary(nums) if nums else "")
                    cols.append(f"drop={text}")
                else:
                    cols.append(f"{field}={summary(vals)}")
            print(f"W={width:>6} n={len(recs):>4}: " + "  ".join(cols))
            if VARIANT == "b":
                ramp = [sorted(r["ramp"][i] for r in recs) for i in range(12)]
                print("         ramp medians (of 2048 per 1024 cycles): "
                      + " ".join(str(v[len(v) // 2]) for v in ramp)
                      + "  min: " + " ".join(str(v[0]) for v in ramp))


if __name__ == "__main__":
    if sys.argv[1] == "b":
        VARIANT = "b"
        main(sys.argv[2])
    else:
        main(sys.argv[1])
