"""Show HI04 lines as edge lists.
Usage: python3 hi04_show.py observations.log  (or a file with one line each)

A line "E<id> <384 hex>" holds 256 samples of {eclk_in, pad, p2, n2, p35,
n35}, sample 0 first. Sample u holds the state before the 25 MHz edge u + 1
of the window, and div = u mod 8 in it: eclk_in is high for u mod 8 in 4..7,
so its rising edges fall between samples 8m+3 and 8m+4. An edge listed at u
lies between samples u-1 and u."""
import ast
import sys

NAMES = ["eclk_in", "pad", "p2", "n2", "p35", "n35"]


def samples(hexs):
    bits = bin(int(hexs, 16))[2:].zfill(4 * len(hexs))
    return [[int(bits[6 * i + k]) for k in range(6)] for i in range(len(bits) // 6)]


def edges(col):
    """Sample indices where a column changes, with the new value."""
    return [(u, col[u]) for u in range(1, len(col)) if col[u] != col[u - 1]]


def lines(path):
    out = set()
    for l in open(path):
        if "distinct=" in l:
            out |= {x for x in ast.literal_eval(l.partition("distinct=")[2]) if len(x) == 388}
        elif l.startswith("E") and len(l.strip()) == 388:
            out.add(l.strip())
    return sorted(out)


if __name__ == "__main__":
    for line in lines(sys.argv[1]):
        ident, hexs = line.split()
        s = samples(hexs)
        print(ident)
        for k, name in enumerate(NAMES[1:], 1):
            col = [x[k] for x in s]
            print(f"  {name:4} start {col[0]}: " + " ".join(f"{u}{'+' if v else '-'}" for u, v in edges(col)))
