#!/usr/bin/env python3
"""Bind raw ECP5 site references to architecture objects, outside the simulator.

Rules are scoped to the inventoried LFE5U-85F. Unresolved references are retained;
vendor positions are never used as general fabric coordinates. This is a partial
structural catalogue, not a behavioural model or a complete native routing graph.
"""

import argparse
from collections import Counter
import csv
import io as textio
import json
from pathlib import Path
from toolchain_paths import portable_path
import re
import subprocess

from inventory_trellis import revision, sha256, site_family, write_csv
from resource_audit import audit_config, audit_unmodelled, compile_decode_catalog


DEVICE = "LFE5U-85F"
DSP_GUIDE = "https://www.latticesemi.com/view_document?document_id=50469"


def coordinates(tile):
    return tuple(map(int, re.search(r"R(\d+)C(\d+):", tile).groups()))


def resource_id(row, col, name):
    return f"{DEVICE}/R{row}C{col}/{name}"


def build(grid, io):
    resources, ports, bindings = {}, [], []
    probe_requests, config_targets = [], {}
    by_type = {}
    for tile, data in grid.items():
        by_type.setdefault(data["type"], []).append(tile)

    def only(kind):
        names = by_type.get(kind, [])
        if len(names) != 1:
            raise ValueError(f"expected one {kind}, found {len(names)}")
        return names[0]

    def add(row, col, name, family, owner, rule, native=True, parent=""):
        key = resource_id(row, col, name)
        if key in resources:
            raise ValueError(f"duplicate structural object: {key}")
        resources[key] = dict(resource_id=key, family=family, fabric_row=row,
                              fabric_col=col, name=name, anchor_tile=owner,
                              parent_id=parent, native_bel=native,
                              native_name=name.replace("SLICE_", "SLICE") if native else "",
                              rule=rule)
        return key

    pio_anchors, ebr_anchors, dsp_groups = {}, {}, {}
    for tile, data in sorted(grid.items()):
        row, col = coordinates(tile)
        kind = data["type"]
        if kind == "PLC2":
            parent = add(row, col, "PLC2", "PLC2", tile, "logic", False)
            for letter in "ABCD":
                add(row, col, f"SLICE_{letter}", "SLICE", tile, "logic", parent=parent)
        pio_count = 0
        if "PICL0" in kind or "PICR0" in kind:
            pio_count = 4
        elif "PIOT0" in kind or ("PICB0" in kind and kind != "SPICB0"):
            pio_count = 2
        elif kind == "SPICB0":
            pio_count = 1
        if pio_count:
            pio_anchors[row, col] = pio_count
            for letter in "ABCD"[:pio_count]:
                add(row, col, f"PIO{letter}", "PIO", tile, "io_anchor")
                simple = row in (0, 95)
                add(row, col, f"{'S' if simple else ''}IOLOGIC{letter}",
                    "SIOLOGIC" if simple else "IOLOGIC", tile, "io_anchor")
        ebr_z = {"MIB_EBR0": 0, "EBR_CMUX_UR": 0, "EBR_CMUX_LR": 0,
                 "MIB_EBR2": 1, "MIB_EBR4": 2, "MIB_EBR6": 3}.get(kind)
        if ebr_z is not None:
            key = add(row, col, f"EBR{ebr_z}", "EBR", tile, "ebr_anchor")
            ebr_anchors[row, col, f"EBR{ebr_z}"] = key
        if kind == "MIB_DSP0":
            dsp_groups[row, col] = add(row, col, "DSP_BLOCK", "DSP_BLOCK", tile,
                                       "dsp_group", False)

    # A DSP group spans nine columns, each with MIB_DSP and MIB2_DSP fragments.
    for (row, base), parent in sorted(dsp_groups.items()):
        slices = {}
        for side, offset in (("LEFT", 0), ("RIGHT", 4)):
            owner = f"MIB_R{row}C{base + offset}:MIB_DSP{offset}"
            slices[side] = add(row, base + offset, f"DSP_SLICE_{side}", "DSP_SLICE", owner,
                               "dsp_slice_group", False, parent)
        for z in (0, 1, 4, 5):
            owner = f"MIB_R{row}C{base + z}:MIB_DSP{z}"
            add(row, base + z, f"MULT18_{z}", "MULT18", owner, "dsp_anchor", parent=slices["LEFT" if z < 4 else "RIGHT"])
        for z in (3, 7):
            owner = f"MIB_R{row}C{base + z}:MIB_DSP{z}"
            add(row, base + z, f"ALU54_{z}", "ALU54", owner, "dsp_anchor", parent=slices["LEFT" if z < 4 else "RIGHT"])
        for offset in range(9):
            for layer in ("MIB", "MIB2"):
                tile = f"MIB_R{row}C{base + offset}:{layer}_DSP{offset}"
                if tile not in grid:
                    raise ValueError(f"missing DSP configuration fragment: {tile}")
                for z in (0, 1, 4, 5):
                    config_targets[tile, f"MULT18_{z}"] = resource_id(row, base + z, f"MULT18_{z}")
                for z in (3, 7):
                    config_targets[tile, f"ALU54_{z}"] = resource_id(row, base + z, f"ALU54_{z}")
                for side in ("LEFT", "RIGHT"):
                    config_targets[tile, f"DSP_{side}"] = slices[side]

    # Bind EBR settings using their site identity, including SPINE/CMUX fragments.
    # One tile may configure two neighbouring EBRs; never assign by tile alone.
    for tile, data in sorted(grid.items()):
        row, _ = coordinates(tile)
        for site in data["sites"]:
            if re.fullmatch(r"EBR[0-3]", site["name"]):
                target = ebr_anchors.get((row, int(site["pos_col"]), site["name"]))
                if not target:
                    raise ValueError(f"EBR configuration site without anchor: {tile}:{site['name']}")
                config_targets[tile, site["name"]] = target

    # Public primitive interfaces from Bels.cpp. These do not imply independent
    # internal register storage; that ownership is still a separate task.
    for key, r in list(resources.items()):
        family = r["family"]
        if family not in ("EBR", "MULT18", "ALU54"):
            continue
        row, col = r["fabric_row"], r["fabric_col"]
        index = int(r["name"].removeprefix("EBR")) if family == "EBR" else int(r["name"].split("_")[1])
        probe_requests.append(["hard", row, col, family, index, -1, key, r["native_name"]])
        postfix = {"EBR": "EBR", "MULT18": "MULT18", "ALU54": "ALU54"}[family]

        def hard_port(pin, direction="in"):
            constructor = {"EBR": "add_bram", "MULT18": "add_mult18", "ALU54": "add_alu54b"}[family]
            ports.append([key, pin, direction, row, col, f"J{pin}_{postfix}", f"Bels.cpp:{constructor}"])

        if family == "EBR":
            for p in ("ADA", "ADB"):
                for i in range(14): hard_port(f"{p}{i}")
            for p in ("DIA", "DIB"):
                for i in range(18): hard_port(f"{p}{i}")
            for p in ("DOA", "DOB"):
                for i in range(18): hard_port(f"{p}{i}", "out")
            for p in ("CEA", "CEB", "CLKA", "CLKB", "OCEA", "OCEB", "RSTA", "RSTB", "WEA", "WEB"):
                hard_port(p)
            for p in ("CSA", "CSB"):
                for i in range(3): hard_port(f"{p}{i}")
        else:
            for p in ("CLK", "CE", "RST"):
                for i in range(4): hard_port(f"{p}{i}")
            if family == "MULT18":
                for p in ("SIGNEDA", "SIGNEDB", "SOURCEA", "SOURCEB"): hard_port(p)
                for p in ("A", "B", "C", "SRIA", "SRIB"):
                    for i in range(18): hard_port(f"{p}{i}")
                for p in ("ROA", "ROB", "ROC", "SROA", "SROB"):
                    for i in range(18): hard_port(f"{p}{i}", "out")
                for i in range(36): hard_port(f"P{i}", "out")
                hard_port("SIGNEDP", "out")
            else:
                for p in ("SIGNEDIA", "SIGNEDIB", "SIGNEDCIN"): hard_port(p)
                for p in ("A", "B", "MA", "MB"):
                    for i in range(36): hard_port(f"{p}{i}")
                for p in ("C", "CFB", "CIN"):
                    for i in range(54): hard_port(f"{p}{i}")
                for i in range(11): hard_port(f"OP{i}")
                for p in ("R", "CO"):
                    for i in range(54): hard_port(f"{p}{i}", "out")
                for p in ("EQZ", "EQZM", "EQOM", "EQPAT", "EQPATB", "OVER", "UNDER", "OVERUNDER", "SIGNEDR"):
                    hard_port(p, "out")

    for quad, dr, dc in (("UL", 0, 1), ("UR", 0, -1), ("LL", -1, 0), ("LR", -1, 0)):
        owner = only(f"PLL0_{quad}")
        row, col = coordinates(owner)
        row, col = row + dr, col + dc
        key = add(row, col, f"EHXPLL_{quad}", "PLL", owner, "pll_anchor")
        probe_requests.append(["pll", row, col, quad, 0, -1, key, f"EHXPLL_{quad}"])
        # Bels.cpp:add_pll; CLKI/CLKFB/CLKINTFB use wires without the J prefix.
        for pin in ("REFCLK", "RST", "STDBY", "PHASEDIR", "PHASELOADREG", "PHASESEL0", "PHASESEL1",
                    "PHASESTEP", "PLLWAKESYNC", "ENCLKOP", "ENCLKOS2", "ENCLKOS3", "ENCLKOS"):
            ports.append([key, pin, "in", row, col, f"J{pin}_PLL", "Bels.cpp:add_pll"])
        for pin, direction in (("CLKI", "in"), ("CLKFB", "in"), ("CLKINTFB", "out")):
            ports.append([key, pin, direction, row, col, f"{pin}_PLL", "Bels.cpp:add_pll"])
        for pin in ("LOCK", "INTLOCK", "CLKOP", "CLKOS", "CLKOS2", "CLKOS3"):
            ports.append([key, pin, "out", row, col, f"J{pin}_PLL", "Bels.cpp:add_pll"])

    for kind, side, count in (("LMID_0", "L", 14), ("RMID_0", "R", 14),
                               ("TMID_0", "T", 12), ("BMID_0V", "B", 16)):
        owner = only(kind)
        row, col = coordinates(owner)
        for z in range(count):
            key = add(row, col, f"{side}DCC{z}", "DCC_MID", owner, "dcc_mid")
            for pin, direction, wire in (("CLKI", "in", f"G_CLKI_{side}DCC{z}"),
                                          ("CE", "in", f"G_JCE_{side}DCC{z}"),
                                          ("CLKO", "out", f"G_CLKO_{side}DCC{z}")):
                ports.append([key, pin, direction, 0, 0, wire, "Bels.cpp:add_dcc"])

    # Missing from get_routing_graph_ecp5's BEL calls; retained from the CMUX
    # sites and the 85K CMUX fuzzer, with independently named input wires.
    for quad, suffix in (("LL", "BL"), ("LR", "BR"), ("UL", "TL"), ("UR", "TR")):
        owner = only(f"CMUX_{quad}_0")
        key = add(46, 67, f"DCC{suffix}", "DCC_CENTRE", owner, "dcc_centre", False)
        for pin, direction, wire in (("CLKI", "in", f"G_JCLKI_DCC{suffix}"),
                                      ("CE", "in", f"G_JCE_DCC{suffix}"),
                                      ("CLKO", "out", f"G_CLKO_DCC{suffix}")):
            ports.append([key, pin, direction, 0, 0, wire, "023-cmux; EBR_CMUX_UL/bits.db"])

    for z, quad in ((0, "UL"), (1, "LL")):
        owner = only(f"EBR_CMUX_{quad}")
        row, col = coordinates(owner)
        key = add(row, col, f"DCS{z}", "DCS", owner, "dcs")
        for pin in ("CLK0", "CLK1", "DCSOUT", "MODESEL", "SEL0", "SEL1"):
            prefix = "J" if pin in ("MODESEL", "SEL0", "SEL1") else ""
            ports.append([key, pin, "out" if pin == "DCSOUT" else "in", 0, 0,
                          f"G_{prefix}{pin}_DCS{z}", "Bels.cpp:add_dcs"])

    def add_ioclk(row, col, kind, owner, index=0, bank=-1, native=True):
        names = {"CLKDIVF": f"CLKDIV{index}", "ECLKSYNCB": f"ECLKSYNC{index}_BK{bank}",
                 "TRELLIS_ECLKBUF": f"ECLKBUF{index}", "ECLKBRIDGECS": f"ECLKBRIDGECS{index}",
                 "BRGECLKSYNC": f"BRGECLKSYNC{index}", "DLLDELD": "DLLDEL",
                 "DDRDLL": "DDRDLL", "DQSBUFM": "DQSBUF"}
        name = names[kind]
        key = add(row, col, name, kind, owner, "ioclk_constructor" if native else "top_dlldel", native)
        probe_requests.append(["ioclk", row, col, kind, index, bank, key, name])

        def port(pin, direction="in", j=True, wire=None, global_wire=False):
            ports.append([key, pin, direction, 0 if global_wire else row, 0 if global_wire else col,
                          wire or f"{'J' if j else ''}{pin}_{postfix}", "Bels.cpp:add_ioclk_bel"])

        if kind == "CLKDIVF":
            postfix = f"CLKDIV{index}"
            port("CLKI", j=False); port("RST"); port("ALIGNWD"); port("CDIVX", "out")
        elif kind in ("ECLKSYNCB", "BRGECLKSYNC"):
            postfix = f"ECLKSYNC{index}" if kind == "ECLKSYNCB" else name
            port("ECLKI", j=False); port("STOP"); port("ECLKO", "out")
        elif kind == "TRELLIS_ECLKBUF":
            port("ECLKI", wire=f"JECLK{index}")
            port("ECLKO", "out", wire=f"G_BANK{bank}ECLK{index}", global_wire=True)
        elif kind == "ECLKBRIDGECS":
            postfix = name
            port("CLK0"); port("CLK1"); port("SEL"); port("ECSOUT", "out", j=False)
        elif kind == "DLLDELD":
            postfix = "DLLDEL"
            port("A"); port("DDRDEL", j=False); port("LOADN"); port("MOVE"); port("DIRECTION")
            port("Z", "out", j=False); port("CFLAG", "out")
        elif kind == "DDRDLL":
            postfix = "DDRDLL"
            for pin in ("CLK", "RST", "UDDCNTLN", "FREEZE"): port(pin)
            port("DDRDEL", "out", j=False); port("LOCK", "out"); port("DIVOSC", "out")
            for i in range(8): port(f"DCNTL{i}", "out")
        elif kind == "DQSBUFM":
            postfix = "DQS"
            for pin in ("DQSI", "READ1", "READ0", "READCLKSEL2", "READCLKSEL1", "READCLKSEL0"): port(pin)
            port("DDRDEL", j=False); port("ECLK", j=False); port("SCLK"); port("RST")
            for i in range(8): port(f"DYNDELAY{i}")
            for pin in ("PAUSE", "RDLOADN", "RDMOVE", "RDDIRECTION", "WRLOADN", "WRMOVE", "WRDIRECTION"): port(pin)
            for pin in ("DQSR90", "DQSW", "DQSW270"): port(pin, "out")
            for i in range(3):
                port(f"RDPNTR{i}", "out", j=False); port(f"WRPNTR{i}", "out", j=False)
            for pin in ("DATAVALID", "BURSTDET", "RDCFLAG", "WRCFLAG"): port(pin, "out")
        return key

    ioclk_sites = {}
    for side, col_delta, banks, bridge in (("L", -2, (7, 6), 1), ("R", 2, (2, 3), 0)):
        owner = only(f"ECLK_{side}")
        row, col = coordinates(owner)
        col += col_delta
        for index in range(2):
            key = add_ioclk(row, col, "CLKDIVF", owner, index)
            ioclk_sites[owner, f"CLKDIV{index}"] = key
            config_targets[owner, f"CLKDIV_{side}{index}"] = key
        for offset, bank in enumerate(banks):
            for index in range(2):
                key = add_ioclk(row + offset, col, "ECLKSYNCB", owner, index, bank)
                ioclk_sites[owner, f"ECLKSYNC{offset * 2 + index}"] = key
                config_targets[owner, f"ECLKSYNC{index}_BK{bank}"] = key
                add_ioclk(row + offset, col, "TRELLIS_ECLKBUF", owner, index, bank)
        for offset, suffix in ((-1, "70" if side == "L" else "20"),
                               (0, "71" if side == "L" else "21"),
                               (1, "60" if side == "L" else "30"),
                               (2, "61" if side == "L" else "31")):
            key = add_ioclk(row + offset, col, "DLLDELD", owner)
            ioclk_sites[owner, f"DLLDEL{suffix}"] = key
            config_targets[owner, f"DLLDEL_{suffix}"] = key
        for kind in ("ECLKBRIDGECS", "BRGECLKSYNC"):
            key = add_ioclk(row, col, kind, owner, bridge)
            ioclk_sites[owner, f"{kind}{bridge}"] = key
            config_targets[owner, f"{kind}{bridge}"] = key

    # TMID site columns match the relative routing endpoints in the 85K data.
    # These four instances are omitted from Chip.cpp's construction calls.
    for kind, suffixes in (("TMID_0", ("00", "01")), ("TMID_1", ("10", "11"))):
        owner = only(kind)
        for site in grid[owner]["sites"]:
            if site["name"] in {f"DLLDEL{s}" for s in suffixes}:
                key = add_ioclk(0, int(site["pos_col"]), "DLLDELD", owner, native=False)
                ioclk_sites[owner, site["name"]] = key
                config_targets[owner, f"DLLDEL_{site['name'][6:]}"] = key

    for quad in ("UL", "UR", "LL", "LR"):
        owner = only(f"DDRDLL_{quad}")
        row, col = coordinates(owner)
        key = add_ioclk(row + (-10 if quad[0] == "U" else 13), col + (-2 if quad[1] == "L" else 2), "DDRDLL", owner)
        ioclk_sites[owner, "DDRDLL"] = key
        config_targets[owner, "DDRDLL"] = key

    for owner, data in sorted(grid.items()):
        if data["type"] in ("PICL0_DQS2", "PICR0_DQS2"):
            row, col = coordinates(owner)
            key = add_ioclk(row, col, "DQSBUFM", owner)
            side = data["type"][3]
            for offset, fragment in ((-2, "1_DQS0"), (-1, "2_DQS1"), (0, "0_DQS2"), (1, "1_DQS3")):
                tile = f"MIB_R{row + offset}C{col}:PIC{side}{fragment}"
                if tile not in grid:
                    raise ValueError(f"missing DQS fragment: {tile}")
                ioclk_sites[tile, "DQS"] = key
                config_targets[tile, "DQS"] = key

    # The bridge appears once more in RMID with the exact same raw site tuple.
    right_eclk, rmid = only("ECLK_R"), only("RMID_0")
    bridge_site = next(s for s in grid[right_eclk]["sites"] if s["name"] == "ECLKBRIDGECS0")
    if bridge_site not in grid[rmid]["sites"]:
        raise ValueError("RMID bridge reference no longer matches ECLK_R")
    ioclk_sites[rmid, "ECLKBRIDGECS0"] = ioclk_sites[right_eclk, "ECLKBRIDGECS0"]

    system_sites, system_keys = {}, {}
    for kind, owner_kind, dr, dc in (("GSR", "EFB0_PICB0", -1, 0), ("JTAGG", "EFB0_PICB0", -1, 0),
                                    ("OSCG", "EFB0_PICB0", -1, 0), ("SEDGA", "EFB0_PICB0", -1, 0),
                                    ("DTR", "DTR", -1, 0), ("USRMCLK", "EFB1_PICB1", 0, -5)):
        owner = only(owner_kind)
        row, col = coordinates(owner)
        row, col = row + dr, col + dc
        key = add(row, col, kind, kind, owner, "system_constructor")
        system_keys[kind] = key
        probe_requests.append(["misc", row, col, kind, 0, -1, key, kind])
        postfix = {"JTAGG": "JTAG", "OSCG": "OSC", "SEDGA": "SED", "USRMCLK": "CCLK"}.get(kind, kind)
        for tile, data in grid.items():
            if data["type"].startswith("EFB") or data["type"] in ("OSC", "DTR", "VIQ_BUF"):
                config_targets[tile, postfix] = key
                if any(s["name"] == postfix for s in data["sites"]): system_sites[tile, postfix] = key
        spec = {
            "GSR": (("GSR", "CLK"), (), ()),
            "JTAGG": (("TCK", "TMS", "TDI", "JTDO2", "JTDO1"), ("TDO", "JTDI", "JTCK", "JRTI2", "JRTI1", "JSHIFT", "JUPDATE", "JRSTN", "JCE2", "JCE1"), ()),
            "OSCG": ((), (), ("SEDSTDBY",)),
            "SEDGA": (("SEDENABLE", "SEDSTART", "SEDFRCERR"), ("SEDDONE", "SEDINPROG", "SEDERR"), ()),
            "DTR": (("STARTPULSE",), tuple(f"DTROUT{i}" for i in range(8)), ()),
            "USRMCLK": (("PADDO", "PADDT"), ("PADDI",), ())}[kind]
        for direction, pins in (("in", spec[0]), ("out", spec[1])):
            for pin in pins: ports.append([key, pin, direction, row, col, f"J{pin}_{postfix}", "Bels.cpp:add_misc"])
        for pin in spec[2]: ports.append([key, pin, "out", row, col, f"{pin}_{postfix}", "Bels.cpp:add_misc"])
        if kind == "OSCG": ports.append([key, "OSC", "out", 0, 0, "G_JOSC_OSC", "Bels.cpp:add_misc"])
        if kind == "SEDGA": ports.append([key, "SEDSTDBY", "in", row, col, "SEDSTDBY_SED", "Bels.cpp:add_misc"])

    # Every package entry must resolve to the exact same set as the native PIO
    # construction rules, not merely to a matching total.
    physical_pio = {(r["fabric_row"], r["fabric_col"], r["name"][-1])
                    for r in resources.values() if r["family"] == "PIO"}
    metadata = {(p["row"], p["col"], p["pio"]) for p in io["pio_metadata"]}
    if physical_pio != metadata or len(metadata) != len(io["pio_metadata"]):
        raise ValueError("PIO instances do not exactly match unique iodb metadata")
    for package, pins in io["packages"].items():
        for pin, p in pins.items():
            if (p["row"], p["col"], p["pio"]) not in physical_pio:
                raise ValueError(f"unbound package pin {package}:{pin}")

    side_offsets = {"PICL0": 0, "PICR0": 0, "PICL0_DQS2": 0, "PICR0_DQS2": 0,
                    "PICL1": 1, "PICR1": 1, "PICL1_DQS0": 1, "PICR1_DQS0": 1,
                    "PICL1_DQS3": 1, "PICR1_DQS3": 1, "PICL2": 2, "PICR2": 2,
                    "PICL2_DQS1": 2, "PICR2_DQS1": 2, "MIB_CIB_LR": 2,
                    "MIB_CIB_LR_A": 2, "MIB_CIB_LRC": 2, "MIB_CIB_LRC_A": 2,
                    "MIB_CIB_LX": 3, "MIB_CIB_RX": 3}
    for tile, data in sorted(grid.items()):
        row, col = coordinates(tile)
        kind = data["type"]
        for index, site in enumerate(data["sites"]):
            name, key, relation, rule = site["name"], "", "unresolved", "unresolved"
            family = site_family(name)
            if family == "SLICE" and kind == "PLC2":
                key, relation, rule = resource_id(row, col, name), "instance", "logic"
            elif family in ("PIO", "IOLOGIC"):
                if kind in side_offsets:
                    ar, ac = row - side_offsets[kind], col
                    rule = "io_side_fragment"
                elif kind in ("PIOT0", "PIOT1", "PICT0", "PICT1"):
                    ar, ac = 0, col - int(kind.endswith("1"))
                    rule = "io_top_fragment"
                elif kind == "SPICB0" or "PICB0" in kind or "PICB1" in kind:
                    ar, ac = 95, col - int("PICB1" in kind)
                    rule = "io_bottom_fragment"
                else:
                    raise ValueError(f"unrecognised IO reference owner: {tile}")
                if (ar, ac) not in pio_anchors:
                    raise ValueError(f"IO fragment without anchor: {tile}")
                key = resource_id(ar, ac, ("S" if family == "IOLOGIC" and ar in (0, 95) else "") + name)
                relation = "instance"
                if kind in ("MIB_CIB_LRC", "MIB_CIB_LRC_A", "MIB_CIB_LX", "MIB_CIB_RX"):
                    # These empty peripheral fragments have site lists but no
                    # decoded settings. Adjacency suggests the same IO group;
                    # retain that inference separately from a confirmed alias.
                    relation, rule = "instance_candidate", "io_empty_fragment_candidate"
            elif family == "EBR":
                key = ebr_anchors.get((row, int(site["pos_col"]), name), "")
                relation, rule = "instance", "ebr_fragment_column"
            elif family == "PLL":
                quad = kind[-2:] if kind.startswith("PLL") else {"BANKREF8": "LL", "BANKREF4": "LR"}.get(kind)
                if quad:
                    owner = only(f"PLL0_{quad}")
                    r, c = coordinates(owner)
                    key = resource_id(r - int(quad in ("LL", "LR")), c + {"UL": 1, "UR": -1}.get(quad, 0), f"EHXPLL_{quad}")
                    relation, rule = "instance", "pll_fragment"
            elif family == "DCC":
                if kind.startswith("CMUX_"):
                    suffix = {"DCC0": "BL", "DCC1": "BR", "DCC2": "TL", "DCC3": "TR"}[name]
                    key, rule = resource_id(46, 67, f"DCC{suffix}"), "dcc_centre"
                else:
                    side = kind[0]
                    owner_kind = {"L": "LMID_0", "R": "RMID_0", "T": "TMID_0", "B": "BMID_0V"}[side]
                    r, c = coordinates(only(owner_kind))
                    key, rule = resource_id(r, c, side + name), "dcc_mid"
                relation = "instance"
            elif family == "DCS":
                owner = only("EBR_CMUX_UL" if name == "DCS0" else "EBR_CMUX_LL")
                r, c = coordinates(owner)
                key, relation, rule = resource_id(r, c, name), "instance", "dcs"
            elif (tile, name) in ioclk_sites:
                key, relation, rule = ioclk_sites[tile, name], "instance", "ioclk_site"
                if family == "ECLKSYNC":
                    # Raw tilegrid indices 0..3 and bank/index BEL names use
                    # different namespaces; the index correspondence needs a
                    # vendor-site crosswalk beyond the constructor topology.
                    relation, rule = "instance_candidate", "eclksync_site_order_candidate"
            elif (tile, name) in system_sites:
                key, relation, rule = system_sites[tile, name], "instance", "system_fragment"
            elif family in ("MULT18", "ALU54", "PRADD18", "PRADD9", "MULT9", "ALU24"):
                match = re.fullmatch(r"MIB2?_DSP(\d)", kind)
                if not match:
                    raise ValueError(f"unrecognised DSP reference owner: {tile}")
                base = col - int(match[1])
                parent = dsp_groups[row, base]
                if family in ("MULT18", "ALU54"):
                    key = resource_id(row, base + int(name.split("_")[1]), name)
                    relation, rule = "instance", "dsp_anchor"
                else:
                    # Membership is established; lane/register sharing is not.
                    key, relation, rule = parent, "group_member", "dsp_group_member"
            if key and key not in resources:
                raise ValueError(f"reference targets absent object: {tile}:{name} -> {key}")
            if not key:
                if relation != "unresolved":
                    raise ValueError(f"rule {rule} could not bind {tile}:{name}")
            bindings.append([tile, index, name, family, site["pos_row"], site["pos_col"],
                             key, relation, rule])

    for resource in resources.values():
        if resource["anchor_tile"] not in grid:
            raise ValueError(f"missing anchor tile: {resource}")
        if resource["parent_id"] and resource["parent_id"] not in resources:
            raise ValueError(f"missing parent object: {resource}")
    return resources, bindings, ports, probe_requests, config_targets


def database_symbol(name):
    if name.startswith(("25K_", "45K_")):
        return None
    if name.startswith("85K_"):
        name = name[4:]
    match = re.fullmatch(r"(?:[NS]\d+)?(?:[EW]\d+)?_(.+)", name)
    return match[1] if match else name


def bind_config_fields(grid, inventory, targets):
    fields, bit_rows = [], []
    for tile, data in sorted(grid.items()):
        descriptions = inventory["tile_types"][data["type"]]
        for kind, category in (("enum", "enums"), ("word", "words")):
            for name, description in sorted(descriptions[category].items()):
                target = targets.get((tile, name.split(".")[0]))
                if not target:
                    continue
                field_id = f"{tile}/{kind}/{name}"
                groups = (o["bits"] for o in description["options"]) if kind == "enum" else description["bit_groups"]
                tokens = {t.lstrip("!") for group in groups for t in group if t != "-"}
                for token in sorted(tokens):
                    match = re.fullmatch(r"F(\d+)B(\d+)", token)
                    if not match:
                        raise ValueError(f"unrecognised config bit: {field_id}: {token}")
                    frame, bit = map(int, match.groups())
                    if frame >= data["cols"] or bit >= data["rows"]:
                        raise ValueError(f"config bit outside owner tile: {field_id}: {token}")
                    bit_rows.append([field_id, target, tile, frame, bit,
                                     data["start_frame"] + frame, data["start_bit"] + bit])
                fields.append([field_id, target, tile, kind, name, description["default"], len(tokens),
                               f"{descriptions['source']}:{description['line']}"])
    return fields, bit_rows


def find_routing_issues(grid, inventory, resources, ports, fields):
    """Identify raw fixed bypasses that cannot implement an enabled STOP input."""
    def endpoint(row, col, name):
        if name.startswith(("25K_", "45K_")):
            return None
        if name.startswith("85K_"):
            name = name[4:]
        match = re.fullmatch(r"([NS]\d+)?([EW]\d+)?_(.+)", name)
        if match:
            vertical, horizontal, name = match.groups()
            if vertical: row += int(vertical[1:]) * (1 if vertical[0] == "S" else -1)
            if horizontal: col += int(horizontal[1:]) * (1 if horizontal[0] == "E" else -1)
        return row, col, name

    by_resource = {}
    for p in ports: by_resource.setdefault(p[0], {})[p[1]] = (p[3], p[4], p[5])
    mode_fields = {f[1]: f[0] for f in fields if f[4].endswith(".MODE")}
    issues = []
    for key, r in resources.items():
        if r["family"] != "ECLKSYNCB":
            continue
        tile = r["anchor_tile"]
        row, col = coordinates(tile)
        description = inventory["tile_types"][grid[tile]["type"]]
        for connection in description["fixed_connections"]:
            if (endpoint(row, col, connection["source"]) == by_resource[key]["ECLKI"] and
                    endpoint(row, col, connection["sink"]) == by_resource[key]["ECLKO"]):
                issues.append([key, mode_fields[key], tile, connection["source"], connection["sink"],
                               f"{description['source']}:{connection['line']}",
                               "fixed_input_output_bypass_requires_mode_review"])
    return issues


def verify_native(probe, root, requests, ports):
    command = [[str(c) for c in r[:6]] for r in requests]
    result = subprocess.run([str(probe.resolve()), str(root)],
                            input="".join(" ".join(r) + "\n" for r in command),
                            capture_output=True, text=True, check=True)
    by_location = {(r[1], r[2], r[7]): r[6] for r in requests}
    actual = []
    for row in csv.DictReader(textio.StringIO(result.stdout), delimiter="\t"):
        key = by_location[int(row["row"]), int(row["col"]), row["bel"]]
        actual.append((key, row["port"], row["direction"], int(row["wire_row"]), int(row["wire_col"]), row["wire"]))
    targets = {r[6] for r in requests}
    expected = [tuple(p[:6]) for p in ports if p[0] in targets]
    if Counter(actual) != Counter(expected):
        missing = Counter(expected) - Counter(actual)
        extra = Counter(actual) - Counter(expected)
        raise ValueError(f"native constructor ports differ: missing {missing}, extra {extra}")
    library_lines = [s.removeprefix("libtrellis_path=") for s in result.stderr.splitlines() if s.startswith("libtrellis_path=")]
    if len(library_lines) != 1:
        raise ValueError("native probe did not identify its loaded Trellis library")
    library = Path(library_lines[0]).resolve()
    return dict(constructors=len(requests), ports=len(actual), exact_port_match=True,
                probe_sha256=sha256(probe), helper_source_sha256=sha256(Path(__file__).with_name("trellis_resource_probe.cpp")),
                loaded_library_path=portable_path(library), loaded_library_sha256=sha256(library),
                scope="Native constructor interfaces only; placement, full routing and behaviour are not verified.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inventory", type=Path, default=Path("docs/inventory") / DEVICE)
    parser.add_argument("--database", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--native-probe", type=Path, help="Optional compiled trellis_resource_probe helper")
    args = parser.parse_args()
    inventory = json.loads((args.inventory / "device_inventory.json").read_text())
    if inventory["device"] != DEVICE:
        raise ValueError("binding rules currently cover LFE5U-85F only")
    root = (args.database or Path(inventory["database_root"]).expanduser()).resolve()
    # Refuse stale inputs: both database data and previously pinned rule sources.
    for source in inventory["source_files"]:
        if sha256(root / source["path"]) != source["sha256"]:
            raise ValueError(f"inventory is stale: {source['path']}")
    for source in inventory["reference_source_files"]:
        if sha256(root.parent / source["path"]) != source["sha256"]:
            raise ValueError(f"Trellis source changed: {source['path']}")
    grid = json.loads((root / "ECP5" / DEVICE / "tilegrid.json").read_text())
    io = json.loads((root / "ECP5" / DEVICE / "iodb.json").read_text())
    resources, bindings, ports, probe_requests, config_targets = build(grid, io)
    fields, bit_rows = bind_config_fields(grid, inventory, config_targets)
    field_groups, enum_aliases, shared_bits, option_conflicts, word_layout = audit_config(grid, inventory, fields, bit_rows)
    unmodelled = audit_unmodelled(inventory, bindings, database_symbol)
    decode_catalog = compile_decode_catalog(grid, inventory, fields, resources)
    routing_issues = find_routing_issues(grid, inventory, resources, ports, fields)
    database_wires = set()
    for tile_type in inventory["tile_types"].values():
        for sink, mux in tile_type["muxes"].items():
            database_wires.add(database_symbol(sink))
            database_wires.update(database_symbol(s["source"]) for s in mux["sources"])
        for connection in tile_type["fixed_connections"]:
            database_wires.update((database_symbol(connection["sink"]), database_symbol(connection["source"])))
    missing_ports = [p for p in ports if p[5] not in database_wires]
    native_verification = verify_native(args.native_probe, root, probe_requests, ports) if args.native_probe else None
    counts = Counter(r["family"] for r in resources.values())
    expected = {"PLC2": 10455, "SLICE": 41820, "PIO": 365, "IOLOGIC": 224,
                "SIOLOGIC": 141, "EBR": 208, "MULT18": 156, "ALU54": 78,
                "DSP_BLOCK": 39, "DSP_SLICE": 78, "PLL": 4, "DCC_MID": 56, "DCC_CENTRE": 4, "DCS": 2,
                "CLKDIVF": 4, "ECLKSYNCB": 8, "TRELLIS_ECLKBUF": 8, "ECLKBRIDGECS": 2,
                "BRGECLKSYNC": 2, "DLLDELD": 12, "DDRDLL": 4, "DQSBUFM": 14,
                "GSR": 1, "JTAGG": 1, "OSCG": 1, "SEDGA": 1, "DTR": 1, "USRMCLK": 1}
    if counts != Counter(expected):
        raise ValueError(f"unexpected structural counts: {dict(counts)}")
    if len(bindings) != inventory["statistics"]["site_reference_records"]:
        raise ValueError("site records lost during binding")
    references = ["libtrellis/src/Chip.cpp", "libtrellis/src/Bels.cpp",
                  "tools/extract_tilegrid.py", "docs/architecture/global_routing.rst",
                  "fuzzers/ECP5/023-cmux/fuzzer.py", "fuzzers/ECP5/026-dcc/fuzzer.py",
                  "fuzzers/ECP5/070-dsp_routing/fuzzer.py",
                  "fuzzers/ECP5/121-eclk/fuzzer.py", "fuzzers/ECP5/122-eclksync/fuzzer.py",
                  "fuzzers/ECP5/125-brgeclk/fuzzer.py", "fuzzers/ECP5/126-eclkbridgecs/fuzzer.py",
                  "fuzzers/ECP5/130-dqsbuf/fuzzer.py", "fuzzers/ECP5/132-dlldel/fuzzer.py",
                  "fuzzers/ECP5/133-ddrdll/fuzzer.py", "fuzzers/ECP5/101-dtr/fuzzer.py",
                  "fuzzers/ECP5/102-oscg/fuzzer.py", "fuzzers/ECP5/103-gsr/fuzzer.py",
                  "fuzzers/ECP5/104-jtagg/fuzzer.py", "fuzzers/ECP5/105-sedga/fuzzer.py"]
    references.extend(["fuzzers/ECP5/041-ebr_config/fuzzer.py", "fuzzers/ECP5/042-ebr_mux/fuzzer.py",
                       "fuzzers/ECP5/047-copy_ebr_config/fuzzer.py", "fuzzers/ECP5/073-mult18_config/fuzzer.py",
                       "fuzzers/ECP5/075-alu54b_config/fuzzer.py", "fuzzers/ECP5/077-copy_dsp_config/fuzzer.py",
                       "docs/architecture/tiles.rst", "libtrellis/src/Bitstream.cpp"])
    report = dict(schema_version=2, device=DEVICE, database_revision=revision(root),
                  trellis_revision=revision(root.parent),
                  input_inventory_sha256=sha256(args.inventory / "device_inventory.json"),
                  generator_source_files=[dict(path=f"tools/{name}", sha256=sha256(Path(__file__).with_name(name)))
                                          for name in ("bind_trellis_sites.py", "resource_audit.py", "inventory_trellis.py")],
                  resource_counts=dict(sorted(counts.items())),
                  site_binding_counts=dict(sorted(Counter(b[7] for b in bindings).items())),
                  unresolved_by_family=dict(sorted(Counter(b[3] for b in bindings if b[7] == "unresolved").items())),
                  reference_sources=[dict(path=p, sha256=sha256(root.parent / p)) for p in references],
                  additional_reference=dict(url=DSP_GUIDE, revision="FPGA-TN-02205-1.3", pages=[6, 7, 8, 9]),
                  native_constructor_verification=native_verification,
                  port_count=len(ports), port_symbols_absent_from_database=len(missing_ports),
                  config_field_count=len(fields), config_field_bit_references=len(bit_rows),
                  config_unique_cram_bits=len({(b[5], b[6]) for b in bit_rows}),
                  config_field_groups=len(field_groups),
                  config_groups_with_differing_defaults=sum(g[6] for g in field_groups),
                  fragment_enum_alias_classes=len(enum_aliases),
                  cross_resource_config_bits=len(shared_bits),
                  config_options_with_conflicting_bit_constraints=len(option_conflicts),
                  config_word_layout_rows=len(word_layout),
                  config_fields_by_family=dict(sorted(Counter(resources[f[1]]["family"] for f in fields).items())),
                  routing_interpretation_issues=len(routing_issues),
                  validation=dict(exact_pio_metadata_set=True, all_package_pins_bound=True,
                                  all_site_records_preserved=True, all_target_objects_exist=True,
                                  all_exported_port_wire_names_in_database=not missing_ports,
                                  all_config_bits_within_owning_tiles=True),
                  limitations=["Partial structural catalogue; omitted families remain unresolved.",
                               "DSP group membership does not establish internal lane or register ownership.",
                               "No full native RoutingGraph or silicon comparison was performed.",
                               "Exported ports cover EBR/MULT18/ALU54, DCC/DCS and selected IO-clock/system constructors only.",
                               "Database symbol presence does not establish correct location or a physical route.",
                               "Raw ECLKSYNC site indices have candidate bank/index bindings.",
                               "Configuration field bindings preserve bit provenance, not effective mode decoding."])
    nextpnr = root.parent.parent / "nextpnr"
    if (nextpnr / "ecp5/bitstream.cc").is_file():
        report["io_configuration_reference"] = dict(path=portable_path(nextpnr / "ecp5/bitstream.cc"),
                sha256=sha256(nextpnr / "ecp5/bitstream.cc"), revision=revision(nextpnr),
                functions=["get_pio_tile", "get_pic_tile", "write_dqsbuf", "get_bram_tiles", "get_dsp_tiles", "write_bram", "write_mult18", "write_alu54", "run"])
    output = args.output or args.inventory
    output.mkdir(parents=True, exist_ok=True)
    columns = ["resource_id", "family", "fabric_row", "fabric_col", "name", "anchor_tile", "parent_id", "native_bel", "native_name", "rule"]
    write_csv(output / "resource_instances.csv", columns,
              [[r[c] for c in columns] for _, r in sorted(resources.items())])
    write_csv(output / "site_bindings.csv", ["tile", "site_index", "site_name", "label_family",
              "vendor_pos_row", "vendor_pos_col", "resource_id", "relation", "rule"], bindings)
    port_columns = ["resource_id", "port", "direction", "wire_row", "wire_col", "wire", "source"]
    clock_families = {"DCC_MID", "DCC_CENTRE", "DCS", "CLKDIVF", "ECLKSYNCB", "TRELLIS_ECLKBUF", "ECLKBRIDGECS", "BRGECLKSYNC", "DLLDELD", "DDRDLL", "DQSBUFM"}
    write_csv(output / "clock_ports.csv", port_columns,
              sorted(p for p in ports if resources[p[0]]["family"] in clock_families))
    write_csv(output / "resource_ports.csv", port_columns + ["database_symbol_known"],
              [p + [p[5] in database_wires] for p in sorted(ports)])
    write_csv(output / "config_field_bindings.csv", ["field_id", "resource_id", "tile", "kind", "field", "default", "bit_count", "source"], fields)
    write_csv(output / "config_field_bits.csv", ["field_id", "resource_id", "tile", "local_frame", "local_bit", "cram_frame", "cram_bit"], bit_rows)
    write_csv(output / "config_field_groups.csv", ["resource_id", "kind", "field", "fragment_count", "tiles", "defaults", "defaults_differ", "combined_encoding_aliases"], field_groups)
    write_csv(output / "enum_fragment_aliases.csv", ["field_id", "resource_id", "options", "bit_count", "pattern", "source"], enum_aliases)
    write_csv(output / "cross_resource_config_bits.csv", ["cram_frame", "cram_bit", "resource_ids", "field_ids"], shared_bits)
    write_csv(output / "config_option_conflicts.csv", ["resource_id", "field", "option", "cram_frame", "cram_bit"], option_conflicts)
    write_csv(output / "config_word_layout.csv", ["field_id", "resource_id", "field", "logical_bit_lsb0", "width", "mapping_status", "cram_constraints_for_one"], word_layout)
    (output / "unmodelled_resource_evidence.json").write_text(json.dumps(unmodelled, ensure_ascii=False, indent=2) + "\n")
    (output / "decode_catalog.json").write_text(json.dumps(decode_catalog, ensure_ascii=False, separators=(",", ":")) + "\n")
    write_csv(output / "routing_interpretation_issues.csv", ["resource_id", "mode_field_id", "tile", "raw_source", "raw_sink", "source", "issue"], routing_issues)
    (output / "binding_report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({k: report[k] for k in ("resource_counts", "site_binding_counts", "unresolved_by_family", "port_count", "port_symbols_absent_from_database", "config_field_count", "config_field_bit_references", "native_constructor_verification", "validation")}, indent=2))


if __name__ == "__main__":
    main()
