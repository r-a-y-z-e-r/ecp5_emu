"""Extract the ULX3S board geometry from its KiCad 5 ``.kicad_pcb`` into JSON.

Reads ``assets/ulx3s/ulx3s_v3.0.8.kicad_pcb`` and writes ``out/ulx3s_3d/board.json``:
the Edge.Cuts outline, every footprint (reference, value, side, position,
rotation, pads, Fab/courtyard extents, KiCad 3D model reference), the drilled
holes and the vias. Coordinates stay in KiCad's board frame: millimetres,
x to the right, y down, angles counter-clockwise as seen from the top.

Footprints on B.Cu are stored by KiCad 5 already flipped, so local coordinates
map to the board by the same rotate-then-translate as on the top side.

Usage: python3 tools/ulx3s_3d/kicad_extract.py
"""

import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths  # noqa: E402


# --- S-expression reader -----------------------------------------------------

def tokenize(text):
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c in " \t\r\n":
            i += 1
        elif c == "(" or c == ")":
            yield c
            i += 1
        elif c == '"':
            j = i + 1
            buf = []
            while text[j] != '"':
                if text[j] == "\\":
                    j += 1
                    buf.append({"n": "\n"}.get(text[j], text[j]))
                else:
                    buf.append(text[j])
                j += 1
            yield ("str", "".join(buf))
            i = j + 1
        else:
            # A quote inside a bare atom belongs to it (the OLED footprint has `0.96"`).
            j = i
            while j < n and text[j] not in " \t\r\n()":
                j += 1
            yield text[i:j]
            i = j


def parse(text):
    stack = [[]]
    for tok in tokenize(text):
        if tok == "(":
            stack.append([])
        elif tok == ")":
            node = stack.pop()
            stack[-1].append(node)
        else:
            stack[-1].append(tok[1] if isinstance(tok, tuple) else tok)
    return stack[0][0]


def children(node, key):
    return [c for c in node if isinstance(c, list) and c and c[0] == key]


def child(node, key):
    for c in node:
        if isinstance(c, list) and c and c[0] == key:
            return c
    return None


def nums(node, start=1):
    out = []
    for v in node[start:]:
        if isinstance(v, str):
            try:
                out.append(float(v))
            except ValueError:
                pass
    return out


def layer_of(node):
    lay = child(node, "layer")
    return lay[1] if lay else None


# --- geometry helpers --------------------------------------------------------

def arc_points(cx, cy, sx, sy, angle_deg, steps=16):
    """KiCad 5 arc: centre, start point, sweep angle (degrees, clockwise on screen)."""
    r = math.hypot(sx - cx, sy - cy)
    a0 = math.atan2(sy - cy, sx - cx)
    pts = []
    for k in range(steps + 1):
        a = a0 + math.radians(angle_deg) * k / steps
        pts.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return pts


def graphic_points(item):
    """Points that bound a fp_/gr_ line, arc, circle, rect or polygon."""
    kind = item[0]
    if kind in ("fp_line", "gr_line"):
        return [tuple(nums(child(item, "start"))), tuple(nums(child(item, "end")))]
    if kind in ("fp_arc", "gr_arc"):
        c = nums(child(item, "start"))
        s = nums(child(item, "end"))
        a = nums(child(item, "angle"))[0]
        return arc_points(c[0], c[1], s[0], s[1], a)
    if kind in ("fp_circle", "gr_circle"):
        c = nums(child(item, "center"))
        e = nums(child(item, "end"))
        r = math.hypot(e[0] - c[0], e[1] - c[1])
        return [(c[0] - r, c[1] - r), (c[0] + r, c[1] + r)]
    if kind in ("fp_poly", "gr_poly"):
        return [tuple(nums(xy)) for xy in children(child(item, "pts"), "xy")]
    return []


def bbox(points):
    if not points:
        return None
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return [min(xs), min(ys), max(xs), max(ys)]


def to_board(x0, y0, rot_deg, lx, ly):
    a = math.radians(rot_deg)
    return (x0 + lx * math.cos(a) + ly * math.sin(a),
            y0 - lx * math.sin(a) + ly * math.cos(a))


def chain_outline(segments):
    """Join Edge.Cuts segments (lists of points) into one closed polygon."""
    segs = [list(s) for s in segments]
    poly = segs.pop(0)

    def close(a, b):
        return math.hypot(a[0] - b[0], a[1] - b[1]) < 1e-3

    while segs:
        for i, s in enumerate(segs):
            if close(poly[-1], s[0]):
                poly += s[1:]
                break
            if close(poly[-1], s[-1]):
                poly += list(reversed(s))[1:]
                break
        else:
            raise ValueError("Edge.Cuts outline is not one closed chain")
        segs.pop(i)
    if close(poly[0], poly[-1]):
        poly.pop()
    return poly


# --- extraction --------------------------------------------------------------

def footprint_record(mod):
    lib_name = mod[1]
    lib, _, name = lib_name.partition(":")
    side = "bottom" if layer_of(mod) == "B.Cu" else "top"
    at = nums(child(mod, "at"))
    x, y = at[0], at[1]
    rot = at[2] if len(at) > 2 else 0.0
    attr = child(mod, "attr")
    ref = value = None
    for t in children(mod, "fp_text"):
        if t[1] == "reference":
            ref = t[2]
        elif t[1] == "value":
            value = t[2]

    fab, crtyd, silk, fab_circles = [], [], [], []
    for item in mod:
        if not (isinstance(item, list) and item and item[0].startswith("fp_")):
            continue
        if item[0] == "fp_text":
            continue
        lay = layer_of(item) or ""
        pts = graphic_points(item)
        if lay.endswith(".Fab"):
            fab += pts
            if item[0] == "fp_circle":
                c = nums(child(item, "center"))
                e = nums(child(item, "end"))
                fab_circles.append([c[0], c[1], math.hypot(e[0] - c[0], e[1] - c[1])])
        elif lay.endswith(".CrtYd"):
            crtyd += pts
        elif lay.endswith(".SilkS"):
            silk += pts

    pads = []
    for p in children(mod, "pad"):
        pat = nums(child(p, "at"))
        prot = (pat[2] if len(pat) > 2 else 0.0) - rot
        size = nums(child(p, "size"))
        drill_node = child(p, "drill")
        drill = None
        if drill_node is not None:
            if "oval" in drill_node:
                d = nums(drill_node, 2)
                drill = {"w": d[0], "h": d[1] if len(d) > 1 else d[0]}
            else:
                d = nums(drill_node)
                drill = {"w": d[0], "h": d[0]} if d else None
            off = child(drill_node, "offset")
            if drill is not None and off is not None:
                drill["offset"] = nums(off)
        layers = child(p, "layers")
        net = child(p, "net")
        pads.append({
            "num": p[1],
            "net": net[2] if net and len(net) > 2 else None,
            "type": p[2],
            "shape": p[3],
            "x": pat[0],
            "y": pat[1],
            "rot": prot,
            "w": size[0],
            "h": size[1] if len(size) > 1 else size[0],
            "drill": drill,
            "layers": layers[1:] if layers else [],
        })

    models = []
    for m in children(mod, "model"):
        rec = {"path": m[1]}
        for key in ("at", "offset", "scale", "rotate"):
            node = child(m, key)
            if node is not None:
                rec[key] = nums(child(node, "xyz"))
        models.append(rec)

    pad_pts = []
    for p in pads:
        a = math.radians(p["rot"])
        hw, hh = p["w"] / 2, p["h"] / 2
        for sx, sy in ((-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)):
            pad_pts.append((p["x"] + sx * math.cos(a) + sy * math.sin(a),
                            p["y"] - sx * math.sin(a) + sy * math.cos(a)))

    return {
        "ref": ref,
        "value": value,
        "lib": lib,
        "name": name,
        "side": side,
        "x": x,
        "y": y,
        "rot": rot,
        "attr": attr[1] if attr else None,
        "fab_bbox": bbox(fab),
        "fab_circles": fab_circles,
        "crtyd_bbox": bbox(crtyd),
        "silk_bbox": bbox(silk),
        "pad_bbox": bbox(pad_pts),
        "pads": pads,
        "models": models,
    }


def main():
    text = paths.PCB.read_text(encoding="utf-8")
    root = parse(text)
    general = child(root, "general")
    thickness = nums(child(general, "thickness"))[0]

    edge = []
    for item in root:
        if isinstance(item, list) and item and item[0].startswith("gr_") and layer_of(item) == "Edge.Cuts":
            edge.append(graphic_points(item))
    outline = chain_outline(edge)
    bb = bbox(outline)

    footprints = [footprint_record(m) for m in children(root, "module")]

    holes = []
    for fp in footprints:
        for p in fp["pads"]:
            if not p["drill"] or p["type"] not in ("thru_hole", "np_thru_hole"):
                continue
            off = p["drill"].get("offset", [0.0, 0.0])
            a = math.radians(p["rot"])
            lx = p["x"] + off[0] * math.cos(a) + off[1] * math.sin(a)
            ly = p["y"] - off[0] * math.sin(a) + off[1] * math.cos(a)
            bx, by = to_board(fp["x"], fp["y"], fp["rot"], lx, ly)
            holes.append({
                "ref": fp["ref"],
                "pad": p["num"],
                "x": bx,
                "y": by,
                "w": p["drill"]["w"],
                "h": p["drill"]["h"],
                "rot": p["rot"] + fp["rot"],
                "plated": p["type"] == "thru_hole",
            })

    vias = []
    for v in children(root, "via"):
        at = nums(child(v, "at"))
        vias.append({
            "x": at[0],
            "y": at[1],
            "size": nums(child(v, "size"))[0],
            "drill": nums(child(v, "drill"))[0],
        })

    data = {
        "source": {
            "file": str(paths.PCB.relative_to(paths.REPO)),
            "design": "ULX3S v3.0.8 (github.com/emard/ulx3s, tag v3.0.8)",
            "frame": "KiCad board frame: mm, x right, y down, angles CCW seen from top",
        },
        "board": {
            "x0": bb[0], "y0": bb[1], "x1": bb[2], "y1": bb[3],
            "width": bb[2] - bb[0], "height": bb[3] - bb[1],
            "thickness": thickness,
        },
        "outline": outline,
        "footprints": footprints,
        "holes": holes,
        "vias": vias,
    }
    paths.OUT.mkdir(parents=True, exist_ok=True)
    paths.BOARD_JSON.write_text(json.dumps(data, indent=1), encoding="utf-8")
    print(f"{paths.BOARD_JSON.relative_to(paths.REPO)}: board {data['board']['width']:.2f} x "
          f"{data['board']['height']:.2f} x {thickness} mm, {len(footprints)} footprints, "
          f"{len(holes)} drilled pad holes, {len(vias)} vias")


if __name__ == "__main__":
    main()
