"""Verify the active DSP divider in routed JSON and text configuration.
Usage: python3 -B check_routed.py DESIGN.routed.json DESIGN.config

Checking only synthesis parameters or the total number of DISABLED enums is
insufficient: the old broken builds had DISABLED on unused CLK1 instead of CLK0.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re


def check(routed, config):
    top = json.loads(Path(routed).read_text())["modules"]["top"]
    cells = {n: c for n, c in top["cells"].items() if c["type"] in ("ALU54B", "MULT18X18D")}
    assert len(cells) in (16, 33), f"unexpected DSP cell count: {len(cells)}"
    active_clocks = set()
    active_registers = 0
    for name, cell in cells.items():
        params, ports = cell["parameters"], cell["connections"]
        assert params.get("CLK0_DIV") == "DISABLED", (name, params)
        assert len(ports["CLK0"]) == 1 and isinstance(ports["CLK0"][0], int), name
        active_clocks.add(ports["CLK0"][0])
        for key, value in params.items():
            if key.startswith("REG_") and key.endswith("_CLK"):
                assert value in ("NONE", "CLK0"), (name, key, value)
                active_registers += value == "CLK0"
    assert len(active_clocks) == 1, "DSP cells must share one active 25 MHz clock"
    assert active_registers > 0
    aliases = [n for n, data in top["netnames"].items() if data["bits"] == list(active_clocks)]
    assert any("clk_25mhz" in name for name in aliases), aliases
    if "ma" in cells and cells["ma"]["parameters"].get("REG_INPUTA_CLK") == "CLK0":
        probes = {"ma": ("INPUTA", 1, 2), "mb": ("INPUTB", 2, 3),
                  "mc": ("PIPELINE", 3, 1), "md": ("OUTPUT", 0, 0)}
        for name, (reg, ce, rst) in probes.items():
            params = cells[name]["parameters"]
            for stage in ("INPUTA", "INPUTB", "PIPELINE", "OUTPUT"):
                assert params[f"REG_{stage}_CLK"] == ("CLK0" if stage == reg else "NONE"), name
            if reg != "OUTPUT":
                assert params[f"REG_{reg}_CE"] == f"CE{ce}", name
                assert params[f"REG_{reg}_RST"] == f"RST{rst}", name
        print("  isolated multiplier stages and CE/RST parameters in routed JSON: PASS")
    enums = Counter(re.findall(r'^enum: (?:ALU54_[0-9]+|MULT18_[0-9]+)\.(CLK[0-3]_DIV) (ENABLED|DISABLED)$', Path(config).read_text(), re.M))
    assert enums[("CLK0_DIV", "DISABLED")] == len(cells), enums
    assert enums[("CLK0_DIV", "ENABLED")] == 0, enums
    assert sum(enums.values()) == 4 * len(cells), enums
    # Every non-NONE selector must use the divider proved disabled above.
    selectors = Counter(re.findall(r'^enum: (?:ALU54_[0-9]+|MULT18_[0-9]+)\.(REG_[A-Z0-9_]+_CLK) (CLK[0-3])$', Path(config).read_text(), re.M))
    assert selectors and all(clock == "CLK0" for _, clock in selectors), selectors
    print(f"{Path(config).name}: {len(cells)} DSP cells; all {sum(selectors.values())} active register selectors use CLK0")
    for clock in range(4):
        print(f"  CLK{clock}_DIV DISABLED={enums[(f'CLK{clock}_DIV', 'DISABLED')]} ENABLED={enums[(f'CLK{clock}_DIV', 'ENABLED')]}")
    print("  active physical clock divider: PASS")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("routed_json")
    parser.add_argument("config")
    args = parser.parse_args()
    check(args.routed_json, args.config)
