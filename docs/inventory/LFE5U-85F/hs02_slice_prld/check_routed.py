"""Check all 32 HS02 probe placements, native parameters and routed PRLD enums.
Usage: python3 -B check_routed.py DESIGN.routed.json DESIGN.config
"""
import argparse
import json
from pathlib import Path
import re
import sys
sys.dont_write_bytecode = True
from hs02_gen import SETTINGS


def check(routed, config):
    cells = json.loads(Path(routed).read_text())["modules"]["top"]["cells"]
    cfg = Path(config).read_text()
    tiles = {}
    current = None
    for line in cfg.splitlines():
        if line.startswith(".tile "):
            current = line.split()[1]
            tiles[current] = {}
        elif line.startswith(".tile_group "):
            current = None
        elif current and line.startswith("enum: "):
            _, key, value = line.split()
            tiles[current][key] = value
    for i, setting in enumerate(SETTINGS):
        name = f"ff{i:02}"
        cell, lut = cells[name], cells[f"di_lut{i:02}"]
        assert cell["type"] == "TRELLIS_FF" and lut["type"] == "TRELLIS_COMB"
        for key, value in zip(("LSRMODE", "SRMODE", "REGSET", "CEMUX"), setting):
            assert cell["parameters"][key].rstrip() == value, (name, key)
        assert cell["parameters"]["SD"].rstrip() == "1", name
        assert not lut["connections"].get("M"), f"{name}: identity LUT must leave M unconnected"
        assert cell["connections"]["M"] != cell["connections"]["DI"], name
        assert cell["connections"]["DI"] == lut["connections"]["F"], name
        x, y, sl, reg = re.fullmatch(r'X(\d+)/Y(\d+)/(SLICE[A-D])\.FF([01])', cell["attributes"]["NEXTPNR_BEL"]).groups()
        assert lut["attributes"]["NEXTPNR_BEL"] == f"X{x}/Y{y}/{sl}.K{reg}", name
        enums = tiles[f"R{y}C{x}:PLC2"]
        assert enums[f"{sl}.REG{reg}.LSRMODE"] == setting[0], name
        assert enums[f"{sl}.REG{reg}.REGSET"] == setting[2], name
        assert enums[f"{sl}.REG{reg}.SD"] == "1", name
        assert enums[f"{sl}.CEMUX"] == setting[3], name
        assert setting[1] in [enums.get("LSR0.SRMODE"), enums.get("LSR1.SRMODE")], name
    count = len(re.findall(r'^enum: SLICE[A-D]\.REG[01]\.LSRMODE PRLD$', cfg, re.M))
    assert count == 24, count
    print(f"{Path(config).name}: REGx.LSRMODE PRLD={count}; 32 probe parameter/placement/config checks; no identity-LUT M tie: PASS")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("routed_json")
    parser.add_argument("config")
    args = parser.parse_args()
    check(args.routed_json, args.config)
