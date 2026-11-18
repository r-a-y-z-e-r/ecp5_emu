#!/usr/bin/env python3
"""Check board.rs peripheral constants against U1 in the ULX3S v3.0.8 PCB."""
import pathlib
import re

root = pathlib.Path(__file__).resolve().parents[1]
pcb = (root / "assets/ulx3s/ulx3s_v3.0.8.kicad_pcb").read_text()
# Balanced S-expressions, with quoted strings left intact.
def expressions(text, prefix):
    for match in re.finditer(r"\(" + prefix + r"(?=\s)", text):
        depth, quoted, escaped = 0, False, False
        for end in range(match.start(), len(text)):
            c = text[end]
            if escaped:
                escaped = False
            elif c == "\\" and quoted:
                escaped = True
            elif c == '"':
                quoted = not quoted
            elif not quoted:
                depth += (c == "(") - (c == ")")
                if depth == 0:
                    yield text[match.start():end + 1]
                    break

u1, = (f for f in expressions(pcb, "module") if re.search(r"\(fp_text reference U1\s", f))
pads = {}
for pad in expressions(u1, "pad"):
    ball = re.match(r"\(pad (\S+)", pad)[1]
    net = re.search(r"\(net \d+ ([^)]+)\)", pad)
    if net:
        pads[ball] = net[1].strip('"')
source = (root / "src/board.rs").read_text()
expected = {
    "GPDI_DP": ["FPDI_D0+", "FPDI_D1+", "FPDI_D2+", "FPDI_CLK+"],
    "GPDI_DN": ["FPDI_D0-", "FPDI_D1-", "FPDI_D2-", "FPDI_CLK-"],
    **{f"OLED_{p}": [f"OLED_{net}"] for p, net in [("CLK", "CLK"), ("MOSI", "MOSI"), ("DC", "DC"), ("RESN", "RES"), ("CSN", "CS")]},
    **{f"SD_{p}": [f"SD_{net}"] for p, net in [("CLK", "CLK"), ("CMD", "CMD"), ("CDN", "CD"), ("WP", "WP")]},
    "SD_D": [f"SD_D{i}" for i in range(4)],
    **{f"SDRAM_{p}": [f"SDRAM_{net}"] for p, net in [("CLK", "CLK"), ("CKE", "CKE"), ("CSN", "nCS"), ("WEN", "nWE"), ("RASN", "nRAS"), ("CASN", "nCAS")]},
    **{f"SDRAM_{p}": [f"SDRAM_{p}{i}" for i in range(n)] for p, n in [("BA", 2), ("DQM", 2), ("A", 13), ("D", 16)]},
}
count = 0
for name, nets in expected.items():
    decl = re.search(r"pub const " + name + r":.*?=\s*(.*?);", source, re.S)[1]
    balls = re.findall(r'"([A-Z]+\d+)"', decl)
    assert len(balls) == len(nets), name
    for ball, net in zip(balls, nets):
        assert pads.get(ball) == net, (name, ball, net, pads.get(ball))
        count += 1
print(f"Peripheral ball check: {count}/{count} balls match PCB U1 (ULX3S v3.0.8)")
