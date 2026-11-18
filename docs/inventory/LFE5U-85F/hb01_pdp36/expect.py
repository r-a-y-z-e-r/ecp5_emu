"""The HB01 report lines under each hypothesis, from program.json alone (no
emulator code): a reference to read the board's lines against.

Hypotheses (each is one argument, all combinations by default):
  low=A|B      the read path whose pins (DOA/DOB) carry bits 17:0 of the
               36-bit word; yosys's PDP wrapper assumes A, nextpnr's public
               PDPW16KD packing assumes B (EBR_ADDRESSING.md, B01);
  ctl=B|own    path A of the 36-bit read takes OCE and RST from the read
               port B, or its own OCEA/RSTA (on CLKB edges);
  ce0=reset|hold   a SYNC reset at an edge with CEB=0 clears the latch, or
               the latch holds.
Usage: python3 expect.py [low=A] [ctl=B] [ce0=reset]
Prints one pass per combination, then which combinations give identical
lines (they are not told apart by this bench)."""
import itertools, json, os, sys

here = os.path.dirname(os.path.abspath(__file__))
program = json.load(open(os.path.join(here, "program.json")))
INIT = [int(w, 16) for w in program["init"]]
DATA = [int(d, 16) for d in program["data"]]
STEPS = program["steps"]
M18 = (1 << 18) - 1


def run(low, ctl, ce0):
    mem = {w: INIT[w] for w in range(len(INIT))}
    # Latches / output registers per EBR and path (0 = A, 1 = B). After the
    # priming pass every value is one the program set; start from 0 and run
    # two passes, printing the second.
    l1 = [0, 0]
    l2, o2 = [0, 0], [0, 0]
    l3b = 0
    lines = []
    for p in range(2):
        for i, s in enumerate(STEPS):
            word = mem.get(s["ra"], 0)
            halves = [word & M18, word >> 18]          # bits 17:0, 35:18
            path_of = [0, 1] if low == "A" else [1, 0]  # half -> path
            if s["pb"]:
                # E1: NOREG, read when CEB=1, SYNC reset per path.
                for h in range(2):
                    path = path_of[h]
                    rst = s["rsta"] if (path == 0 and ctl == "own") else s["rstb"]
                    if s["ceb"]:
                        l1[path] = 0 if rst else halves[h]
                    elif rst and ce0 == "reset":
                        l1[path] = 0
                # E2: OUTREG; the register takes the old latch when its OCE is 1.
                old = list(l2)
                for path in range(2):
                    oce = s["ocea"] if (path == 0 and ctl == "own") else s["oceb"]
                    if oce:
                        o2[path] = old[path]
                if s["ceb"]:
                    for h in range(2):
                        l2[path_of[h]] = halves[h]
                # E3: 18-bit read of the 18-bit word {ra, half}: bits 17:0
                # (half 0) or 35:18 (half 1) of the 36-bit word, on DOB.
                if s["ceb"]:
                    l3b = halves[s["half"]]
            if s["pa"] and s["cea"]:
                old = mem.get(s["wa"], 0)
                new = old
                for g in range(4):
                    if s["be"] >> g & 1:
                        m = 0x1FF << (9 * g)
                        new = (new & ~m) | (DATA[s["wsel"]] & m)
                mem[s["wa"]] = new
            if p == 1:
                lines.append(f"S{i:02X} A{l1[0]:05X} B{l1[1]:05X} C{o2[0]:05X} D{o2[1]:05X} E{l3b:05X}")
    return lines


def main():
    fixed = dict(a.split("=") for a in sys.argv[1:])
    combos = [dict(low=l, ctl=c, ce0=e) for l, c, e in
              itertools.product("AB", ["B", "own"], ["reset", "hold"])]
    combos = [c for c in combos if all(c[k] == v for k, v in fixed.items())]
    passes = {}
    for c in combos:
        name = " ".join(f"{k}={v}" for k, v in c.items())
        passes[name] = run(**c)
        print(f"# {name}")
        print("\n".join(passes[name]))
    names = list(passes)
    for a, b in itertools.combinations(names, 2):
        if passes[a] == passes[b]:
            print(f"# identical: {a} | {b}")


if __name__ == "__main__":
    main()
