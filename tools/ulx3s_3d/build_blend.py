"""Build the ULX3S v3.0.8 3D model in Blender from the extracted KiCad data.

Inputs (run these first): ``kicad_extract.py`` -> ``out/ulx3s_3d/board.json``,
``layers.py`` -> ``out/ulx3s_3d/textures/{top,bottom}_{color,data}.png``.
Output: ``out/ulx3s_3d/ulx3s_v3.0.8.blend`` (textures packed).

Run headless in Blender::

    blender -b -P tools/ulx3s_3d/build_blend.py

It also runs from a live session (exec the file with ``__file__`` set), but
export the glTF headless: exporting through the Blender MCP add-on crashed
Blender 5.1.2 once. The script builds into a scene named
``ULX3S`` (replacing it if present) and leaves other scenes alone.

Conventions
-----------
* 1 Blender unit = 1 mm (scene length unit mm, unit scale 0.001).
* Board centre at the origin, top copper at z = +0.8, bottom at z = -0.8.
  Blender X = KiCad x - centre, Blender Y = centre - KiCad y (KiCad's y is down).
* One object per populated footprint, named by its reference (U1, D0, B1, J1,
  GPDI1, ...), origin at the footprint origin on the board surface, z pointing
  away from the board (bottom-side parts are turned 180 degrees about X).
  Custom properties: ref, value, footprint, side, kicad_x/y/rot, and for the
  board I/O: led_index / led_color (D0..D7), btn_index / btn_name (B0..B6).
* Movable parts are child objects: ``B<n>_plunger``, ``SW1_slider<k>``.
* Each LED has its own lens material ``LED_<ref>`` with an emission input set
  to strength 0: the GUI lights it by raising the strength.
* Unpopulated footprints (KiCad attr ``virtual``, or every 3D model the designer
  marked ``_disabled``/``_defunct``/``_hidden``) get no body; through-hole ones
  (J1, J2, J3, J4, J5, OLED1) and the mounting holes get their plated rings.

Part bodies are drawn from datasheet dimensions (comments give the source
size); positions, rotations, pad geometry and Fab outlines come from KiCad.
"""

import json
import math
import sys
from pathlib import Path

import bmesh
import bpy
from mathutils import Matrix, Vector
from mathutils.geometry import tessellate_polygon

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib  # noqa: E402

import paths  # noqa: E402

importlib.reload(paths)  # a live Blender session keeps modules between runs

SCENE = "ULX3S"
TAG = "ulx3s_3d"  # ID property marking every datablock this script creates
T_BOARD = 1.6
Z_TOP = T_BOARD / 2

UNPOPULATED_SUFFIXES = ("_disabled", "_defunct", "_hidden")
# How far the microSD card's outer end sits past the SD1 socket mouth (mm). 0 = pushed
# all the way in, flush with the socket front (0.3 mm inside the board edge). The
# radiona.org photo shows a half-inserted card, 6.4 here.
SD_CARD_OUT = 0.0

LED_RGB = {
    "RED": (1.0, 0.05, 0.02),
    "ORANGE": (1.0, 0.35, 0.0),
    "GREEN": (0.1, 1.0, 0.1),
    "BLUE": (0.05, 0.2, 1.0),
}
# board.rs: btn[0]..btn[6] = PWR, FIRE1, FIRE2, UP, DOWN, LEFT, RIGHT; silkscreen B0..B6.
BUTTONS = {"B0": "PWR", "B1": "FIRE1", "B2": "FIRE2", "B3": "UP", "B4": "DOWN",
           "B5": "LEFT", "B6": "RIGHT"}


# --- materials ---------------------------------------------------------------

def own(idb):
    idb[TAG] = 1
    return idb


def srgb(c):
    """sRGB 0..255 or 0..1 triple -> linear RGBA."""
    out = []
    for v in c:
        v = v / 255 if v > 1 else v
        out.append(v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4)
    return (*out, 1.0)


MATERIALS = {
    # name: (base sRGB, metallic, roughness, extra)
    "ENIG": ((250, 232, 185), 1.0, 0.32, {}),
    "Tin": ((214, 214, 210), 1.0, 0.3, {}),
    "Steel": ((232, 232, 230), 1.0, 0.16, {}),
    "Steel_brushed": ((190, 190, 188), 1.0, 0.35, {}),
    "Nickel": ((196, 192, 182), 1.0, 0.25, {}),
    "Mold_black": ((22, 22, 24), 0.0, 0.45, {}),
    "Plastic_black": ((16, 16, 17), 0.0, 0.5, {}),
    "Plastic_white": ((235, 235, 230), 0.0, 0.4, {}),
    "Resistor_black": ((20, 20, 20), 0.0, 0.55, {}),
    "Ceramic_cap": ((166, 126, 82), 0.0, 0.55, {}),
    "Ferrite": ((58, 58, 58), 0.0, 0.6, {}),
    "Ferrite_grey": ((112, 112, 110), 0.2, 0.5, {}),
    "Ceramic_white": ((228, 222, 205), 0.0, 0.5, {}),
    "BGA_substrate": ((60, 52, 30), 0.0, 0.5, {}),
    "Marking": ((150, 150, 148), 0.0, 0.7, {}),
    "Marking_dark": ((60, 60, 60), 0.0, 0.7, {}),
    "Module_pcb": ((24, 24, 26), 0.0, 0.35, {}),
    "Kapton": ((255, 196, 70), 0.0, 0.1, {"transmission": 0.9}),
    "SD_card": ((20, 20, 22), 0.0, 0.4, {}),
    "PCB_edge": ((88, 108, 58), 0.0, 0.75, {}),  # FR-4 edge under green mask, as on the photos
    "OLED_pcb": ((25, 70, 160), 0.0, 0.35, {}),
    "OLED_glass": ((3, 3, 5), 0.0, 0.12, {}),
    "OLED_screen": ((0, 0, 0), 0.0, 0.1, {}),
    "OLED_flex": ((230, 140, 30), 0.0, 0.35, {}),
}


def make_material(name, base, metallic, roughness, extra):
    mat = own(bpy.data.materials.new(name))
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = srgb(base)
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["Roughness"].default_value = roughness
    if "transmission" in extra:
        bsdf.inputs["Transmission Weight"].default_value = extra["transmission"]
        bsdf.inputs["IOR"].default_value = 1.7
    if "alpha" in extra:
        bsdf.inputs["Alpha"].default_value = extra["alpha"]
        if hasattr(mat, "surface_render_method"):
            mat.surface_render_method = "BLENDED"
    mat.diffuse_color = srgb(base)
    return mat


def led_material(ref, color_name):
    mat = own(bpy.data.materials.new(f"LED_{ref}"))
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    rgb = LED_RGB.get(color_name, (1.0, 1.0, 1.0))
    # Off: a faintly tinted clear epoxy. On: raise Emission Strength (GUI).
    tint = tuple(0.75 + 0.25 * c for c in rgb)
    bsdf.inputs["Base Color"].default_value = (*tint, 1.0)
    bsdf.inputs["Roughness"].default_value = 0.08
    bsdf.inputs["Transmission Weight"].default_value = 0.85
    bsdf.inputs["IOR"].default_value = 1.5
    bsdf.inputs["Emission Color"].default_value = (*rgb, 1.0)
    bsdf.inputs["Emission Strength"].default_value = 0.0
    mat.diffuse_color = (*tint, 1.0)
    return mat


def pcb_material(side):
    mat = own(bpy.data.materials.new(f"PCB_{side}"))
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes.get("Principled BSDF")
    col = nt.nodes.new("ShaderNodeTexImage")
    col.image = own(bpy.data.images.load(str(paths.TEXTURES / f"{side}_color.png")))
    col.image.name = f"ulx3s_{side}_color"
    col.interpolation = "Cubic"
    dat = nt.nodes.new("ShaderNodeTexImage")
    dat.image = own(bpy.data.images.load(str(paths.TEXTURES / f"{side}_data.png")))
    dat.image.name = f"ulx3s_{side}_data"
    dat.image.colorspace_settings.name = "Non-Color"
    sep = nt.nodes.new("ShaderNodeSeparateColor")
    bump = nt.nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.6
    bump.inputs["Distance"].default_value = 0.06
    nt.links.new(col.outputs["Color"], bsdf.inputs["Base Color"])
    nt.links.new(dat.outputs["Color"], sep.inputs["Color"])
    nt.links.new(sep.outputs["Red"], bump.inputs["Height"])
    nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
    nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
    nt.links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
    # Solder mask is a thin glossy lacquer over the copper/laminate.
    bsdf.inputs["Coat Weight"].default_value = 0.1
    bsdf.inputs["Coat Roughness"].default_value = 0.12
    mat.diffuse_color = srgb((20, 110, 60))
    return mat


MATS = {}


def marking_font():
    font = bpy.data.fonts.load(str(paths.FONT), check_existing=True)
    return own(font)


def mat(name):
    if name not in MATS:
        base, metallic, rough, extra = MATERIALS[name]
        MATS[name] = make_material(name, base, metallic, rough, extra)
    return MATS[name]


# --- mesh building -----------------------------------------------------------

class Part:
    """A bmesh with named material slots, turned into one object at the end."""

    def __init__(self):
        self.bm = bmesh.new()
        self.mats = []

    def mi(self, material):
        if isinstance(material, str):
            material = mat(material)
        if material not in self.mats:
            self.mats.append(material)
        return self.mats.index(material)

    def _finish(self, verts, material):
        idx = self.mi(material)
        faces = {f for v in verts for f in v.link_faces}
        for f in faces:
            f.material_index = idx
        return faces

    def box(self, cx, cy, sx, sy, z0, z1, material, bevel=0.0, rot=0.0, segments=2):
        """Axis box of size sx*sy centred on (cx, cy) in the part frame, turned by rot degrees."""
        a = math.radians(rot)
        ca, sa = math.cos(a), math.sin(a)
        corners = [(-sx / 2, -sy / 2), (sx / 2, -sy / 2), (sx / 2, sy / 2), (-sx / 2, sy / 2)]
        vs = []
        for z in (z0, z1):
            for x, y in corners:
                vs.append(self.bm.verts.new((cx + x * ca - y * sa, cy + x * sa + y * ca, z)))
        f = self.bm.faces.new
        faces = [f((vs[0], vs[3], vs[2], vs[1])), f((vs[4], vs[5], vs[6], vs[7])),
                 f((vs[0], vs[1], vs[5], vs[4])), f((vs[1], vs[2], vs[6], vs[5])),
                 f((vs[2], vs[3], vs[7], vs[6])), f((vs[3], vs[0], vs[4], vs[7]))]
        if bevel > 0:
            b = min(bevel, 0.45 * min(sx, sy, z1 - z0))
            edges = list({e for fc in faces for e in fc.edges})
            res = bmesh.ops.bevel(self.bm, geom=edges, offset=b, segments=segments,
                                  profile=0.5, affect="EDGES", clamp_overlap=True)
            vs = vs + list(res["verts"])
            vs = [v for v in vs if v.is_valid]
        return self._finish(vs, material)

    def seg(self, p0, p1, w, t, material):
        """A w-wide, t-thick strip from p0 to p1 (lead segments)."""
        p0, p1 = Vector(p0), Vector(p1)
        d = (p1 - p0).normalized()
        side = d.cross(Vector((0, 0, 1)))
        if side.length < 1e-6:
            side = Vector((1, 0, 0))
        side.normalize()
        up = side.cross(d).normalized()
        vs = []
        for p in (p0, p1):
            for sx, sz in ((-1, -1), (1, -1), (1, 1), (-1, 1)):
                vs.append(self.bm.verts.new(p + side * (sx * w / 2) + up * (sz * t / 2)))
        f = self.bm.faces.new
        f((vs[0], vs[1], vs[2], vs[3]))
        f((vs[4], vs[7], vs[6], vs[5]))
        for i in range(4):
            j = (i + 1) % 4
            f((vs[i], vs[4 + i], vs[4 + j], vs[j]))
        return self._finish(vs, material)

    def cyl(self, cx, cy, r, z0, z1, material, segs=24, r2=None):
        m = Matrix.Translation((cx, cy, (z0 + z1) / 2))
        res = bmesh.ops.create_cone(self.bm, cap_ends=True, cap_tris=False, segments=segs,
                                    radius1=r, radius2=r if r2 is None else r2,
                                    depth=z1 - z0, matrix=m)
        return self._finish(res["verts"], material)

    def sphere(self, cx, cy, cz, r, material, segs=8, rings=5):
        m = Matrix.Translation((cx, cy, cz))
        res = bmesh.ops.create_uvsphere(self.bm, u_segments=segs, v_segments=rings,
                                        radius=r, matrix=m)
        return self._finish(res["verts"], material)

    def prism(self, outer, holes, z0, z1, material, matrix=None):
        """Extrude a 2D polygon with holes from z0 to z1; optional matrix applied after."""
        loops = [oriented(outer, True)] + [oriented(h, False) for h in holes]
        flat = [Vector((x, y, 0.0)) for loop in loops for (x, y) in loop]
        tris = tessellate_polygon([[Vector((x, y, 0.0)) for (x, y) in loop] for loop in loops])
        bot = [self.bm.verts.new((p.x, p.y, z0)) for p in flat]
        top = [self.bm.verts.new((p.x, p.y, z1)) for p in flat]
        new_faces = []
        for a, b, c in tris:
            pa, pb, pc = flat[a], flat[b], flat[c]
            if (pb - pa).cross(pc - pa).z < 0:
                b, c = c, b
            for verts in ((top[a], top[b], top[c]), (bot[a], bot[c], bot[b])):
                try:
                    new_faces.append(self.bm.faces.new(verts))
                except ValueError:
                    pass
        start = 0
        for loop in loops:
            n = len(loop)
            for i in range(n):
                a, b = start + i, start + (i + 1) % n
                try:
                    new_faces.append(self.bm.faces.new((bot[a], bot[b], top[b], top[a])))
                except ValueError:
                    pass
            start += n
        verts = bot + top
        if matrix is not None:
            bmesh.ops.transform(self.bm, matrix=matrix, verts=verts)
            if matrix.to_3x3().determinant() < 0:
                bmesh.ops.reverse_faces(self.bm, faces=new_faces)
        return self._finish(verts, material)

    def text(self, body, size, cx, cy, z, rot, material, bold=False, spacing=1.0):
        """Flat text (laser marking) lying on z, centred on (cx, cy), reading along rot degrees."""
        cu = bpy.data.curves.new("_marking", "FONT")
        cu.font = marking_font()
        cu.body = body
        cu.size = size
        cu.align_x = "CENTER"
        cu.align_y = "CENTER"
        cu.space_character = spacing
        ob = bpy.data.objects.new("_marking", cu)
        bpy.context.scene.collection.objects.link(ob)
        dg = bpy.context.evaluated_depsgraph_get()
        me = bpy.data.meshes.new_from_object(ob.evaluated_get(dg))
        bpy.data.objects.remove(ob)
        bpy.data.curves.remove(cu)
        src = bmesh.new()
        src.from_mesh(me)
        bpy.data.meshes.remove(me)
        m = Matrix.Translation((cx, cy, z)) @ Matrix.Rotation(math.radians(rot), 4, "Z")
        vmap = {}
        for v in src.verts:
            vmap[v] = self.bm.verts.new(m @ v.co)
        idx = self.mi(material)
        for f in src.faces:
            try:
                nf = self.bm.faces.new([vmap[v] for v in f.verts])
                nf.material_index = idx
            except ValueError:
                pass
        src.free()

    def transform_new(self, since_count, matrix):
        """Apply a matrix to vertices created after `since_count`."""
        self.bm.verts.ensure_lookup_table()
        verts = self.bm.verts[since_count:]
        bmesh.ops.transform(self.bm, matrix=matrix, verts=verts)

    def to_object(self, name, collection, smooth_angle=35.0):
        me = own(bpy.data.meshes.new(name))
        self.bm.normal_update()
        self.bm.to_mesh(me)
        self.bm.free()
        for m in self.mats:
            me.materials.append(m)
        for p in me.polygons:
            p.use_smooth = True
        me.set_sharp_from_angle(angle=math.radians(smooth_angle))
        ob = own(bpy.data.objects.new(name, me))
        collection.objects.link(ob)
        return ob


# --- footprint helpers -------------------------------------------------------

class FP:
    """A footprint in its canonical frame: as on the top side, x right, y up, z up."""

    def __init__(self, rec):
        self.rec = rec
        self.ref = rec["ref"]
        self.value = rec["value"]
        self.bottom = rec["side"] == "bottom"
        # KiCad 5 stores bottom footprints flipped (local y negated); undo it.
        self.ys = 1.0 if self.bottom else -1.0

    def pt(self, x, y):
        return (x, self.ys * y)

    def pads(self):
        out = []
        for p in self.rec["pads"]:
            x, y = self.pt(p["x"], p["y"])
            rot = -p["rot"] if self.bottom else p["rot"]
            a = math.radians(rot)
            ex = abs(p["w"] / 2 * math.cos(a)) + abs(p["h"] / 2 * math.sin(a))
            ey = abs(p["w"] / 2 * math.sin(a)) + abs(p["h"] / 2 * math.cos(a))
            out.append({**p, "cx": x, "cy": y, "crot": rot, "ex": ex, "ey": ey})
        return out

    def bbox(self, key):
        b = self.rec.get(key)
        if not b:
            return None
        x0, y0, x1, y1 = b
        ya, yb = sorted((self.ys * y0, self.ys * y1))
        return (x0, ya, x1, yb)

    def edge_dir(self, board):
        """Unit vector (canonical frame) toward the nearest board edge."""
        x, y = self.rec["x"], self.rec["y"]
        cands = [(x - board["x0"], (-1, 0)), (board["x1"] - x, (1, 0)),
                 (y - board["y0"], (0, -1)), (board["y1"] - y, (0, 1))]
        _, (bx, by) = min(cands)
        a = math.radians(self.rec["rot"])
        lx = bx * math.cos(a) - by * math.sin(a)
        ly = bx * math.sin(a) + by * math.cos(a)
        return self.pt(lx, ly)


def pad_polygon(p, segs=20):
    """Outline of a pad (canonical frame) as a list of (x, y)."""
    w, h = p["w"], p["h"]
    a = math.radians(p["crot"])
    pts = []
    if p["shape"] in ("circle", "oval"):
        r = min(w, h) / 2
        dx, dy = (w / 2 - r, 0) if w >= h else (0, h / 2 - r)
        for i in range(segs):
            t = 2 * math.pi * i / segs
            c = (dx, dy) if math.cos(t) * dx + math.sin(t) * dy >= 0 else (-dx, -dy)
            pts.append((c[0] + r * math.cos(t), c[1] + r * math.sin(t)))
    else:
        pts = [(-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)]
    ca, sa = math.cos(a), math.sin(a)
    return [(p["cx"] + x * ca - y * sa, p["cy"] + x * sa + y * ca) for x, y in pts]


def oriented(loop, ccw):
    """The loop counter-clockwise (ccw=True) or clockwise, seen from +z."""
    pts = list(loop)
    area = sum(pts[i][0] * pts[(i + 1) % len(pts)][1] - pts[(i + 1) % len(pts)][0] * pts[i][1]
               for i in range(len(pts)))
    return pts if (area > 0) == ccw else pts[::-1]


def circle(cx, cy, r, segs=24, reverse=False):
    pts = [(cx + r * math.cos(2 * math.pi * i / segs), cy + r * math.sin(2 * math.pi * i / segs))
           for i in range(segs)]
    return pts[::-1] if reverse else pts


def leads_to_pads(part, fp, body, z_exit, w_scale=0.7, t=0.15, max_w=0.5, material="Tin",
                  gull=True, drop=0.3):
    """Gull-wing (or flat) leads from the body rectangle (x0, y0, x1, y1) to every pad outside it."""
    x0, y0, x1, y1 = body
    for p in fp.pads():
        px, py = p["cx"], p["cy"]
        out_x = max(x0 - px, px - x1, 0)
        out_y = max(y0 - py, py - y1, 0)
        if out_x <= 0 and out_y <= 0:
            continue
        if out_x >= out_y:
            u = Vector((1 if px > x1 else -1, 0, 0))
            edge = Vector((x1 if px > x1 else x0, py, 0))
            ext, lat = p["ex"], p["ey"]
        else:
            u = Vector((0, 1 if py > y1 else -1, 0))
            edge = Vector((px, y1 if py > y1 else y0, 0))
            ext, lat = p["ey"], p["ex"]
        w = min(max_w, 2 * lat * w_scale)
        foot = Vector((px, py, 0)) + u * (ext - 0.12)
        span = (foot - edge).dot(u)
        if span <= 0.05:
            continue
        zt = t / 2 + 0.01
        if gull and span > 0.5:
            bend = min(drop, span * 0.35)
            p0 = edge - u * 0.1 + Vector((0, 0, z_exit))
            p1 = edge + u * bend * 0.5 + Vector((0, 0, z_exit))
            p2 = edge + u * bend * 1.4 + Vector((0, 0, zt))
            part.seg(p0, p1, w, t, material)
            part.seg(p1 - u * t * 0.3, p2, w, t, material)
            part.seg(p2 - u * t * 0.3, foot + Vector((0, 0, zt)), w, t, material)
        else:
            part.seg(edge - u * 0.1 + Vector((0, 0, zt)), foot + Vector((0, 0, zt)), w, t, material)


def body_rect(cx, cy, sx, sy):
    return (cx - sx / 2, cy - sy / 2, cx + sx / 2, cy + sy / 2)


def pads_center(fp):
    ps = fp.pads()
    return (sum(p["cx"] for p in ps) / len(ps), sum(p["cy"] for p in ps) / len(ps))


def pad_axis(fp):
    ps = fp.pads()
    xs = [p["cx"] for p in ps]
    ys = [p["cy"] for p in ps]
    return "x" if (max(xs) - min(xs)) >= (max(ys) - min(ys)) else "y"


def pins_on_x(fp):
    """True when the pads form two columns (pins leave the body along +-x)."""
    xs = {round(p["cx"], 2) for p in fp.pads()}
    ys = {round(p["cy"], 2) for p in fp.pads()}
    if len(xs) == 2 and len(ys) != 2:
        return True
    if len(ys) == 2 and len(xs) != 2:
        return False
    return (max(xs) - min(xs)) >= (max(ys) - min(ys))


def text_angle_world(fp, world_deg=0.0):
    """Canonical text angle that reads along world_deg in the top view (top parts)
    or in a bottom view mirrored left-right (bottom parts)."""
    rot = fp.rec["rot"]
    if fp.bottom:
        return rot - 180 - world_deg
    return world_deg - rot


# --- part builders -----------------------------------------------------------
# Each builder fills `part` in the canonical frame of `fp` (board surface z = 0).

def solder_fillets(part, fp, cx, cy, length, height, flat_leads=False):
    """One eight-triangle Tin wedge per termination, only for two-terminal chips."""
    along_x = pad_axis(fp) == "x"
    for p in fp.pads():
        coordinate = p["cx"] if along_x else p["cy"]
        center = cx if along_x else cy
        sign = 1 if coordinate > center else -1
        extent = p["ex"] if along_x else p["ey"]
        pad_end = coordinate + sign * extent
        # Flat SOD-323 leads end 0.12 mm before the outer pad edge (leads_to_pads).
        face = pad_end - sign * 0.12 if flat_leads else center + sign * length / 2
        root = face - sign * 0.015  # overlap the metal face, including its edge bevel
        reach = min(0.6, sign * (pad_end - root) - 0.03)
        if reach <= 0:
            continue
        width = 0.6 * 2 * (p["ey"] if along_x else p["ex"])
        # Triangle coordinates are outward distance / height; extrude across the pad.
        if along_x:
            matrix = Matrix(((sign, 0, 0, root), (0, 0, 1, p["cy"] - width / 2),
                             (0, 1, 0, 0.02), (0, 0, 0, 1)))
        else:
            matrix = Matrix(((0, 0, 1, p["cx"] - width / 2), (sign, 0, 0, root),
                             (0, 1, 0, 0.02), (0, 0, 0, 1)))
        part.prism([(0, 0), (reach, 0), (0, height / 3 - 0.02)], [], 0, width,
                   "Tin", matrix=matrix)


def build_chip(part, fp, height, body_mat, term_len, term_mat="Tin"):
    """Two-terminal chip (resistor, capacitor, inductor)."""
    fab = fp.bbox("fab_bbox")
    cx, cy = (fab[0] + fab[2]) / 2, (fab[1] + fab[3]) / 2
    L, W = fab[2] - fab[0], fab[3] - fab[1]
    along_x = pad_axis(fp) == "x"
    if not along_x:
        L, W = W, L
    rot = 0 if along_x else 90
    bev = min(0.06, height * 0.15)
    part.box(cx, cy, L - 2 * term_len + 0.02, W * 0.98, 0.02, height, body_mat, bevel=bev, rot=rot)
    for s in (-1, 1):
        off = s * (L / 2 - term_len / 2)
        tx, ty = (cx + off, cy) if along_x else (cx, cy + off)
        part.box(tx, ty, term_len, W, 0.0, height + 0.01, term_mat, bevel=bev, rot=rot)
    solder_fillets(part, fp, cx, cy, L, height)


def build_led(part, fp):
    fab = fp.bbox("fab_bbox")
    cx, cy = (fab[0] + fab[2]) / 2, (fab[1] + fab[3]) / 2
    along_x = pad_axis(fp) == "x"
    L, W = 2.0, 1.25  # LED 0805: 2.0 x 1.25 x 0.8 mm
    rot = 0 if along_x else 90
    part.box(cx, cy, L, W, 0.0, 0.32, "Plastic_white", bevel=0.04, rot=rot)
    for s in (-1, 1):
        off = s * (L / 2 - 0.2)
        tx, ty = (cx + off, cy) if along_x else (cx, cy + off)
        part.box(tx, ty, 0.42, W + 0.02, -0.005, 0.2, "Tin", rot=rot)
    part.box(cx, cy, L - 0.16, W - 0.12, 0.3, 0.8, led_material(fp.ref, fp.value), bevel=0.12, rot=rot)
    solder_fillets(part, fp, cx, cy, L + 0.02, 0.8)


def build_sod323(part, fp):
    # SOD-323: body 1.7 x 1.25 x 0.9 mm, flat leads; cathode = pad 1.
    along_x = pad_axis(fp) == "x"
    L, W, H = 1.7, 1.25, 0.9
    cx, cy = pads_center(fp)
    sx, sy = (L, W) if along_x else (W, L)
    part.box(cx, cy, sx, sy, 0.08, H, "Mold_black", bevel=0.08)
    k = next(p for p in fp.pads() if p["num"] == "1")
    s = 1 if (k["cx"] - cx if along_x else k["cy"] - cy) > 0 else -1
    band = 0.3
    off = s * (L / 2 - 0.35)
    bx, by = (cx + off, cy) if along_x else (cx, cy + off)
    part.box(bx, by, band if along_x else W * 0.95, W * 0.95 if along_x else band, H - 0.01, H + 0.004, "Marking")
    leads_to_pads(part, fp, body_rect(cx, cy, sx, sy), 0.1, w_scale=0.5, t=0.12, gull=False)
    solder_fillets(part, fp, cx, cy, L, H, flat_leads=True)


def build_gullwing(part, fp, sx, sy, h, standoff=0.08, t=0.15, max_w=0.45, mark=True, w_scale=0.75):
    cx, cy = pads_center(fp)
    part.box(cx, cy, sx, sy, standoff, h, "Mold_black", bevel=0.08)
    leads_to_pads(part, fp, body_rect(cx, cy, sx, sy), standoff + (h - standoff) * 0.4,
                  t=t, max_w=max_w, w_scale=w_scale)
    if mark:
        p1 = next((p for p in fp.pads() if p["num"] == "1"), None)
        if p1:
            dx = max(-sx / 2 + 0.5, min(sx / 2 - 0.5, p1["cx"] - cx))
            dy = max(-sy / 2 + 0.5, min(sy / 2 - 0.5, p1["cy"] - cy))
            part.cyl(cx + dx * 0.8, cy + dy * 0.8, min(0.3, min(sx, sy) * 0.08), h - 0.02, h + 0.003, "Marking_dark", segs=16)
    return cx, cy


def build_qfn(part, fp, size, h):
    cx, cy = pads_center(fp)
    part.box(cx, cy, size, size, 0.02, h, "Mold_black", bevel=0.04)
    half = size / 2
    for p in fp.pads():
        if p["ex"] > 1.0 and p["ey"] > 1.0:
            continue  # exposed pad under the body
        dx, dy = p["cx"] - cx, p["cy"] - cy
        if abs(dx) >= abs(dy):
            part.box(cx + math.copysign(half, dx), p["cy"], 0.06, 0.24, 0.0, 0.2, "Tin")
        else:
            part.box(p["cx"], cy + math.copysign(half, dy), 0.24, 0.06, 0.0, 0.2, "Tin")
    p1 = next((p for p in fp.pads() if p["num"] == "1"), None)
    if p1:
        part.cyl(cx + (p1["cx"] - cx) * 0.75, cy + (p1["cy"] - cy) * 0.75, 0.25, h - 0.02, h + 0.003, "Marking_dark", segs=16)


def build_bga(part, fp):
    # Lattice caBGA381: 17 x 17 mm, 0.8 mm ball pitch, ~1.5 mm seated height.
    cx, cy = pads_center(fp)
    part.box(cx, cy, 17.0, 17.0, 0.3, 0.66, "BGA_substrate", bevel=0.05)
    part.box(cx, cy, 17.0, 17.0, 0.66, 1.5, "Mold_black", bevel=0.12)
    for p in fp.pads():
        part.sphere(p["cx"], p["cy"], 0.18, 0.22, "Tin", segs=8, rings=4)
    a1 = next((p for p in fp.pads() if p["num"] == "A1"), None)
    if a1:
        part.cyl(cx + (a1["cx"] - cx) * 0.97, cy + (a1["cy"] - cy) * 0.97, 0.45, 1.48, 1.503, "Marking_dark", segs=20)
    ang = text_angle_world(fp)
    a = math.radians(ang)
    lines = [("LATTICE", 2.1, 3.6, True), ("LFE5U-85F", 1.45, 0.9, False),
             ("6BG381I", 1.45, -1.1, False), ("A5411R45", 1.2, -3.1, False)]
    for body, size, off, bold in lines:
        # Offset perpendicular to the reading direction (text "up").
        ox, oy = -math.sin(a) * off, math.cos(a) * off
        part.text(body, size, cx + ox, cy + oy, 1.503, ang, "Marking", bold=bold)


def build_tsop54(part, fp):
    # TSOP-II 54 (400 mil): body 10.16 x 22.22 x 1.2 mm max, 0.8 mm pitch.
    cx, cy = build_gullwing(part, fp, 10.16, 22.22, 1.1, standoff=0.08, t=0.15, max_w=0.35)
    ang = text_angle_world(fp)
    a = math.radians(ang)
    for body, size, off in (("1420", 1.0, 2.2), ("48LC16M16A2", 1.35, 0.0), ("TG   6A  G", 1.0, -2.2)):
        ox, oy = -math.sin(a) * off, math.cos(a) * off
        part.text(body, size, cx + ox, cy + oy, 1.103, ang, "Marking")


def build_button(part, fp, collection):
    # C&K PTS645 6 x 6 mm tact switch (SM43: 4.3 mm; the board's look ~4.8 mm).
    cx, cy = pads_center(fp)
    part.box(cx, cy, 6.0, 6.0, 0.0, 2.6, "Plastic_black", bevel=0.15)
    part.box(cx, cy, 6.0, 6.0, 2.6, 3.1, "Steel", bevel=0.08)
    part.cyl(cx, cy, 2.1, 3.1, 3.3, "Plastic_black", segs=32)
    leads_to_pads(part, fp, body_rect(cx, cy, 6.0, 6.0), 0.5, w_scale=0.6, t=0.2, max_w=0.8)
    for sx in (-1, 1):
        for sy in (-1, 1):
            part.cyl(cx + sx * 2.3, cy + sy * 2.3, 0.35, 3.1, 3.18, "Plastic_black", segs=12)
    plunger = Part()
    plunger.cyl(0, 0, 1.75, 3.3, 4.8, "Plastic_black", segs=40)
    plunger.cyl(0, 0, 1.6, 4.8, 4.85, "Plastic_black", segs=40)
    return plunger, (cx, cy)


def rounded_outline(points, radius, steps=3):
    """Round a polygon's corners in its own plane, including reentrant corners."""
    out = []
    for i, point in enumerate(points):
        p = Vector(point)
        u = (Vector(points[i - 1]) - p).normalized()
        v = (Vector(points[(i + 1) % len(points)]) - p).normalized()
        angle = u.angle(v)
        distance = min(radius / math.tan(angle / 2),
                       (Vector(points[i - 1]) - p).length * 0.49,
                       (Vector(points[(i + 1) % len(points)]) - p).length * 0.49)
        start, end = p + u * distance, p + v * distance
        center = p + (u + v).normalized() * distance / math.cos(angle / 2)
        va, vb = start - center, end - center
        turn = math.atan2(va.x * vb.y - va.y * vb.x, va.dot(vb))
        for k in range(steps + 1):
            t = turn * k / steps
            out.append(tuple(center + Vector((va.x * math.cos(t) - va.y * math.sin(t),
                                               va.x * math.sin(t) + va.y * math.cos(t)))))
    return out


def offset_outline(points, distance):
    """Parallel offset of a convex CCW cross-section (sheet thickness)."""
    points = oriented(points, True)
    out = []
    for i, point in enumerate(points):
        p = Vector(point)
        u = (p - Vector(points[i - 1])).normalized()
        v = (Vector(points[(i + 1) % len(points)]) - p).normalized()
        n0, n1 = Vector((u.y, -u.x)), Vector((v.y, -v.x))
        out.append(tuple(p + (n0 + n1) * distance / (1 + n0.dot(n1))))
    return out


def connector_pads(fp, alpha):
    """Pad centres in the mouth frame; retain KiCad's pad identities and dimensions."""
    inverse = Matrix.Rotation(-alpha, 4, "Z")
    return [{**p, "point": inverse @ Vector((p["cx"], p["cy"], 0))} for p in fp.pads()]


def sheet_tube(part, inner, thickness, cx, front, depth, windows=()):
    """Open x/z tube. The roof is a separate sheet so its stamped holes are real."""
    inner = oriented(inner, True)
    outer = offset_outline(inner, thickness)
    to_y = Matrix(((1, 0, 0, cx), (0, 0, 1, front), (0, 1, 0, 0), (0, 0, 0, 1)))
    for i, (a, b) in enumerate(zip(inner, inner[1:] + inner[:1])):
        oa, ob = outer[i], outer[(i + 1) % len(outer)]
        if abs(a[1] - b[1]) < 1e-6 and a[1] == max(z for _, z in inner):
            part.prism([(cx + oa[0], front + 0.3), (cx + ob[0], front + 0.3),
                        (cx + ob[0], front + depth), (cx + oa[0], front + depth)],
                       windows, a[1], oa[1], "Steel")
        else:
            part.prism([oa, ob, b, a], [], 0.3, depth, "Steel", matrix=to_y)
    # A softly rounded, 0.3 mm deep rolled entry lip around the same opening.
    part.prism(rounded_outline(outer, 0.06, 2), [rounded_outline(inner, 0.05, 2)],
               0, 0.3, "Steel", matrix=to_y)


def build_usb(part, fp, board):
    # FCI drawing 10118192, Rev B; shell 7.5 x 5.0 x 2.6 mm.
    alpha, fab = conn_frame(fp, board)
    start = len(part.bm.verts)
    yf, D, H = fab[1], 5.0, 2.6
    cx = (fab[0] + fab[2]) / 2
    # Exact micro-B aperture: 6.9 x 1.85, 0.75 mm lower 45-degree chamfers.
    inner = [(-3.45, 1.25), (-3.45, 2.35), (3.45, 2.35),
             (3.45, 1.25), (2.7, 0.5), (-2.7, 0.5)]
    slots = [[(cx + x + dx, yf + y) for dx, y in
              ((-0.5, 0.7), (0.5, 0.7), (0.5, 1.5), (-0.5, 1.5))] for x in (-2.1, 2.1)]
    sheet_tube(part, inner, 0.25, cx, yf, D, slots)
    # Folded side seams bring the overall shell to 7.5 mm wide.
    for sign in (-1, 1):
        part.box(cx + sign * 3.65, yf + D / 2, 0.2, D, 1.3, H - 0.1, "Steel", bevel=0.05)
        part.box(cx + sign * 2.1, yf + 1.1, 1.0, 0.8, H - 0.39, H - 0.26,
                 "Steel_brushed", bevel=0.05)
    part.box(cx, yf + (D + 0.4) / 2, 5.4, D - 0.4, 1.5, 2.1,
             "Plastic_black", bevel=0.05)
    for k in range(5):
        part.box(cx + (k - 2) * 0.65, yf + 1.7, 0.25, 2.3, 1.45, 1.51,
                 "ENIG", bevel=0.025, segments=1)
    part.box(cx, yf + D - 0.2, 6.3, 0.4, 0.15, 2.3, "Plastic_white", bevel=0.05)
    for p in connector_pads(fp, alpha):
        x, y, _ = p["point"]
        if p["num"] != "6":
            part.seg((x, yf + D - 0.2, 0.55), (x, y - 0.22, 0.07), 0.24, 0.1, "Tin")
            part.seg((x, y - 0.22, 0.07), (x, y + 0.48, 0.07), 0.24, 0.1, "Tin")
        elif abs(x - cx) > 2.5:
            # Front/rear shell flanges land at the four outer mounting pads.
            edge = cx + math.copysign(3.6, x - cx)
            part.seg((edge, y, 1.1), (x, y, 0.16), 0.8, 0.25, "Steel")
            part.box(x, y, 0.9, 1.0, 0.02, 0.27, "Steel", bevel=0.06)
    part.transform_new(start, Matrix.Rotation(alpha, 4, "Z"))


def build_hdmi(part, fp, board):
    # FCI 10029449-111RLF; retain 15 x 9.6 x 5.8 mm and photo-derived overhang.
    alpha, fab = conn_frame(fp, board)
    start = len(part.bm.verts)
    yf, D, H = fab[1] - 1.6, 9.6, 5.8
    cx = (fab[0] + fab[2]) / 2
    # 14.0 x 4.55 aperture, 10.6 mm bottom, 45-degree lower corners.
    # Centre at z=2.9, matching the existing HDMI_plug's inserted shell.
    inner = [(-7.0, 2.325), (-7.0, 5.175), (7.0, 5.175),
             (7.0, 2.325), (5.3, 0.625), (-5.3, 0.625)]
    windows = [[(cx + x + dx, yf + y) for dx, y in
                ((-1.7, 3.1), (1.7, 3.1), (1.7, 6.1), (-1.7, 6.1))] for x in (-3.6, 3.6)]
    sheet_tube(part, inner, 0.3, cx, yf, D, windows)
    # Raised rear stamping retains the original 5.8 mm overall shell height.
    part.box(cx, yf + 8.0, 14.6, 3.2, 5.45, H, "Steel", bevel=0.08)
    for sign in (-1, 1):
        part.box(cx + sign * 7.35, yf + D - 1.3, 0.3, 2.6, 0.35, H - 0.15,
                 "Steel", bevel=0.08)
        x = cx + sign * 3.6
        for a, b in (((x, yf + 6.1, 5.4), (x, yf + 4.5, 5.0)),
                     ((x, yf + 4.5, 5.0), (x, yf + 3.4, 5.3))):
            part.seg(a, b, 2.2, 0.15, "Steel_brushed")
    middle = (0.625 + 5.175) / 2
    part.box(cx, yf + (D + 0.8) / 2, 10.0, D - 0.8, middle - 0.8, middle + 0.8,
             "Plastic_black", bevel=0.08)
    for count, z, offset in ((10, middle + 0.8, -4.5), (9, middle - 0.85, -4.0)):
        for k in range(count):
            part.box(cx + offset + k, yf + 3.0, 0.3, 3.8, z, z + 0.05,
                     "ENIG", bevel=0.025, segments=1)
    part.box(cx, yf + D - 0.25, 12.5, 0.5, 0.35, 5.4, "Plastic_black", bevel=0.1)
    for p in connector_pads(fp, alpha):
        if p["type"] != "smd":
            continue
        x, y, _ = p["point"]
        # Alternating high/low origins make the two staggered contact rows visible.
        high = int(p["num"]) % 2
        part.seg((x, yf + D - 0.3, 0.8 + high * 0.35),
                 (x, y - 0.2 + high * 0.18, 0.08), 0.21, 0.12, "Tin")
        part.seg((x, y - 0.2, 0.08), (x, y + 0.75, 0.08), 0.21, 0.12, "Tin")
    for p in connector_pads(fp, alpha):
        if p["type"] == "thru_hole":
            x, y, _ = p["point"]
            sign = 1 if x > cx else -1
            leg = abs(x - cx)
            thickness, radius, z_bend = 0.3, 0.2, 3.0
            width = min(1.0, math.sqrt(p["drill"]["h"] ** 2 - thickness ** 2) - 0.1)
            # A continuous L profile: horizontal root inside the side wall,
            # concentric quarter-circle bends, then a flat tab through the drill.
            # The root crosses x=7.0..7.3 at z=3, so there is no gap to the shell.
            outer_r, inner_r = radius + thickness / 2, radius - thickness / 2
            bend_x, bend_z = leg - radius, z_bend - radius
            root = 6.8
            profile = [(root, z_bend + thickness / 2)]
            for i in range(4):
                angle = math.pi / 2 * (1 - i / 3)
                profile.append((bend_x + outer_r * math.cos(angle),
                                bend_z + outer_r * math.sin(angle)))
            profile.extend([(leg + thickness / 2, -T_BOARD - 0.8),
                            (leg - thickness / 2, -T_BOARD - 0.8)])
            for i in range(4):
                angle = math.pi / 2 * i / 3
                profile.append((bend_x + inner_r * math.cos(angle),
                                bend_z + inner_r * math.sin(angle)))
            profile.append((root, z_bend - thickness / 2))
            to_y = Matrix(((sign, 0, 0, cx), (0, 0, 1, y - width / 2),
                           (0, 1, 0, 0), (0, 0, 0, 1)))
            part.prism(profile, [], 0, width, "Steel", matrix=to_y)
    part.transform_new(start, Matrix.Rotation(alpha, 4, "Z"))


def build_jack(part, fp, board):
    # CUI SJ-43516-SMT, the older housing visible in the v3.0.3 photos.
    alpha, fab = conn_frame(fp, board)
    start = len(part.bm.verts)
    yf, W, H, nozzle = fab[1], 6.0, 5.0, 2.4
    cx, front, back = 0.0, yf + nozzle, yf + nozzle + 14.5
    to_y = Matrix(((1, 0, 0, cx), (0, 0, 1, yf), (0, 1, 0, 0), (0, 0, 0, 1)))
    bore = circle(0, H / 2, 1.8, 40)
    part.prism(circle(0, H / 2, 2.5, 40), [bore], 0, nozzle,
               "Plastic_black", matrix=to_y)
    part.prism(circle(0, H / 2, 2.65, 40), [bore], nozzle - 0.4, nozzle,
               "Plastic_black", matrix=to_y)
    # The bore continues 0.6 mm into the body, rather than ending at the nozzle.
    face = rounded_outline([(-3, 0.3), (-2.7, 0), (2.7, 0), (3, 0.3),
                            (3, 4.9), (-3, 4.9)], 0.08)
    part.prism(face, [bore], nozzle, 3.05, "Plastic_black", matrix=to_y)
    part.prism(bore, [], 3.0, 3.05, "Mold_black", matrix=to_y)
    # 0.3 mm bottom chamfer, 0.2 mm recessed top with its raised perimeter.
    section = [(-2.7, 0), (2.7, 0), (3, 0.3), (3, 4.8), (-3, 4.8), (-3, 0.3)]
    part.prism(section, [], 3.05, back - yf, "Plastic_black", matrix=to_y)
    outline = rounded_outline([(-3, front), (3, front), (3, back), (-3, back)], 0.12)
    recess = rounded_outline([(-2.65, front + 0.4), (2.65, front + 0.4),
                              (2.65, back - 0.4), (-2.65, back - 0.4)], 0.15)
    part.prism(outline, [recess], 4.8, H, "Plastic_black")
    # Large silver rear contact, folded up the back and over the top near D0.
    part.box(0, back - 0.08, 4.5, 0.2, 0.25, 4.9, "Steel", bevel=0.06)
    part.box(0, back - 0.7, 4.5, 1.35, 4.9, 5.05, "Steel", bevel=0.06)
    for p in connector_pads(fp, alpha):
        if p["type"] != "smd":
            continue
        x, y, _ = p["point"]
        if abs(x) > W / 2:
            edge = math.copysign(W / 2, x)
            part.seg((edge, y, 1.0), (x, y, 0.13), 1.15, 0.2, "Steel")
            part.box(x, y, 1.1, 1.7, 0.02, 0.22, "Steel", bevel=0.05)
        else:
            part.seg((x, back, 1.1), (x, y, 0.13), 1.3, 0.2, "Steel")
    part.transform_new(start, Matrix.Rotation(alpha, 4, "Z"))
    for p in fp.pads():
        if p["type"] == "np_thru_hole":
            part.cyl(p["cx"], p["cy"], 0.75, -T_BOARD, 0.05, "Plastic_black", segs=16)


def build_sd(part, fp, board, accessories):
    # Alps SCHD3A0100 footprint, open-top construction from the board photos.
    alpha, fab = conn_frame(fp, board)
    start = len(part.bm.verts)
    x0, yf, x1, yb = fab
    W, D, cx = x1 - x0, yb - yf, (x0 + x1) / 2
    part.box(cx, (yf + yb) / 2, W - 0.6, D, 0, 0.35, "Plastic_black")
    for sign in (-1, 1):
        part.box(cx + sign * (W / 2 - 0.55), (yf + yb) / 2, 0.5, D, 0, 1.75,
                 "Plastic_black", bevel=0.05)
    part.box(cx, yb - 0.6, W - 0.6, 1.2, 0, 1.75, "Plastic_black", bevel=0.05)
    # One continuous 0.2 mm stainless U along both long sides and the back.
    u = [(x0, yf), (x0 + 0.2, yf), (x0 + 0.2, yb - 0.2),
         (x1 - 0.2, yb - 0.2), (x1 - 0.2, yf), (x1, yf), (x1, yb), (x0, yb)]
    part.prism(u, [], 0, 1.8, "Steel")
    cover_back = yf + D * 0.3
    cover = rounded_outline([(x0 + 0.2, yf), (x1 - 0.2, yf),
                             (x1 - 0.2, cover_back), (x0 + 0.2, cover_back)], 0.15)
    slot = rounded_outline([(cx - 0.8, yf + 2.1), (cx + 2.8, yf + 2.1),
                            (cx + 2.8, yf + 3.1), (cx - 0.8, yf + 3.1)], 0.15)
    part.prism(cover, [slot], 1.6, 1.8, "Steel")
    for p in connector_pads(fp, alpha):
        x, y, _ = p["point"]
        if p["num"] in tuple(str(i) for i in range(1, 9)):
            # 0.6 mm wide, 1.1 mm pitch; rise, arch, and curl down at the tip.
            points = [(x, y, 0.12), (x, y + 0.7, 0.4), (x, yb - 3.1, 0.8),
                      (x, yb - 2.1, 1.12), (x, yb - 1.7, 1.08), (x, yb - 1.4, 0.82)]
            for a, b in zip(points, points[1:]):
                part.seg(a, b, 0.6, 0.08, "ENIG")
            # Low tails return from the rear frame to KiCad's internal pad row.
            part.seg((x, yb + 0.2, 0.15), (x, y + 0.3, 0.08), 0.45, 0.08, "ENIG")
            part.seg((x, y + 0.3, 0.08), (x, y - 0.3, 0.06), 0.45, 0.08, "ENIG")
        else:
            edge = x0 + 0.1 if x < cx else x1 - 0.1
            part.seg((edge, y, 0.8), (x, y, 0.15), 0.8, 0.2, "Steel")
            part.box(x, y, 0.8, 1.0, 0.02, 0.22, "Steel", bevel=0.05)
    part.transform_new(start, Matrix.Rotation(alpha, 4, "Z"))
    # microSD: 11 x 15 x 0.8, narrowed to 9.7 mm over the rear 6.4 mm.
    card = Part()
    front = yf - SD_CARD_OUT
    outline = [(cx - 5.5, front), (cx + 5.5, front), (cx + 5.5, front + 8.6),
               (cx + 4.2, front + 8.6), (cx + 4.2, front + 15), (cx - 5.5, front + 15)]
    card.prism(rounded_outline(outline, 0.6, 5), [], 0.58, 1.38, "SD_card")
    card.text("micro SD  HC", 1.0, cx, front + 10, 1.385, 0, "Marking")
    card.transform_new(0, Matrix.Rotation(alpha, 4, "Z"))
    return card


def build_dip(part, fp, collection):
    # CTS 219-4LPST-style 4-way low-profile slide DIP switch under amber Kapton tape.
    fab = fp.bbox("fab_bbox")
    cx, cy = (fab[0] + fab[2]) / 2, (fab[1] + fab[3]) / 2
    sx, sy = fab[2] - fab[0], fab[3] - fab[1]
    H = 1.9
    part.box(cx, cy, sx, sy, 0.05, H - 0.02, "Plastic_black", bevel=0.12)
    # Seen through the tape the top reads light amber with dark slots (board photos).
    part.box(cx, cy, sx - 0.3, sy - 0.3, H - 0.05, H, "Plastic_white")
    leads_to_pads(part, fp, body_rect(cx, cy, sx, sy), 0.4, t=0.15, w_scale=0.4, max_w=0.5)
    along_y = sy > sx
    pads = fp.pads()
    rows = sorted({round(p["cy"] if along_y else p["cx"], 2) for p in pads})
    rotation = Matrix.Rotation(math.radians(fp.rec["rot"]), 3, "Z")
    sliders = []
    for k, r in enumerate(rows):
        wx, wy = (2.6, 1.1) if along_y else (1.1, 2.6)
        px, py = (cx, r) if along_y else (r, cy)
        part.box(px, py, wx, wy, H - 0.25, H + 0.002, "Mold_black")
        s = Part()
        sxw, syw = (0.9, 0.9)
        s.box(0, 0, sxw, syw, H - 0.25, H + 0.15, "Plastic_white", bevel=0.05)
        row_pads = [p for p in pads if abs((p["cy"] if along_y else p["cx"]) - r) < 0.01]
        sw_pad = next(p for p in row_pads if p["net"] in ("SW1", "SW2", "SW3", "SW4"))
        index = int(sw_pad["net"][2:]) - 1
        # The SW<n> pad is at the ON end; the photo places ON toward smaller
        # KiCad y (positive board Blender Y). Row order alone reverses SW1..4.
        axis = Vector((1, 0, 0) if along_y else (0, 1, 0))
        toward_on = Vector((sw_pad["cx"] - px, sw_pad["cy"] - py, 0))
        axis *= 1 if toward_on.dot(axis) > 0 else -1
        travel = axis * (2.6 - 0.9)
        world_travel = rotation @ travel
        assert world_travel.y > 1.6, "SW1 ON must face the board's top edge"
        # Extras are not converted by glTF export: rotate the local vector into
        # board Blender coordinates, then (x,y,z) -> glTF (x,z,-y), in mm.
        # sw_travel is already a BOARD-frame vector, not a node-local vector.
        exported = [round(v, 10) for v in (world_travel.x, world_travel.z, -world_travel.y)]
        off = Vector((px, py, 0)) - travel / 2
        sliders.append((s, (off.x, off.y), k + 1, index, exported))
    part.box(cx, cy, sx - 0.1, sy - 0.1, H + 0.16, H + 0.21, "Kapton")
    return sliders


def build_osc7050(part, fp):
    # 7.0 x 5.0 mm SMD crystal oscillator: ceramic base, seam-welded metal lid, 1.4 mm.
    fab = fp.bbox("fab_bbox")
    cx, cy = (fab[0] + fab[2]) / 2, (fab[1] + fab[3]) / 2
    part.box(cx, cy, 7.0, 5.0, 0.03, 0.55, "Ceramic_white", bevel=0.1)
    part.box(cx, cy, 6.4, 4.4, 0.55, 1.4, "Steel_brushed", bevel=0.25)
    for p in fp.pads():
        sx = 1 if p["cx"] > cx else -1
        sy = 1 if p["cy"] > cy else -1
        part.box(cx + sx * 2.54, cy + sy * 2.5, 1.4, 0.06, 0.0, 0.4, "ENIG")
    part.text("FNETHE025", 0.55, cx, cy + 0.5, 1.403, text_angle_world(fp), "Marking_dark")
    part.text("25.000", 0.55, cx, cy - 0.5, 1.403, text_angle_world(fp), "Marking_dark")


def build_abs25(part, fp):
    # Abracon ABS25 32.768 kHz: 8.0 x 3.8 x 2.5 mm plastic SMD.
    cx, cy = pads_center(fp)
    along_x = pad_axis(fp) == "x"
    sx, sy = (8.0, 3.8) if along_x else (3.8, 8.0)
    part.box(cx, cy, sx, sy, 0.1, 2.5, "Mold_black", bevel=0.2)
    for p in fp.pads():
        part.box(p["cx"], p["cy"], 1.2, 0.9, 0.0, 0.25, "Tin")
    ang = text_angle_world(fp)
    part.text("32.768K", 0.9, cx, cy + 0.6, 2.503, ang, "Marking")
    part.text("1811TH6", 0.7, cx, cy - 0.7, 2.503, ang, "Marking")


def build_sma(part, fp):
    # SMA Flat (STPS2L30AF): body 4.3 x 2.6 x 1.0 mm.
    cx, cy = pads_center(fp)
    along_x = pad_axis(fp) == "x"
    sx, sy = (4.3, 2.6) if along_x else (2.6, 4.3)
    part.box(cx, cy, sx, sy, 0.05, 1.0, "Mold_black", bevel=0.12)
    k = next(p for p in fp.pads() if p["num"] == "1")
    s = 1 if (k["cx"] - cx if along_x else k["cy"] - cy) > 0 else -1
    bx, by = (cx + s * 1.5, cy) if along_x else (cx, cy + s * 1.5)
    part.box(bx, by, 0.4 if along_x else sy * 0.9, sy * 0.9 if along_x else 0.4, 0.99, 1.004, "Marking")
    leads_to_pads(part, fp, body_rect(cx, cy, sx, sy), 0.1, t=0.15, w_scale=0.6, max_w=1.6, gull=False)


def build_battery_holder(part, fp):
    # Keystone 3000 CR1225 holder: nickel-plated clip, ~3.2 mm high.
    fab = fp.bbox("fab_bbox")
    circ = fp.rec["fab_circles"][0]
    ccx, ccy = circ[0], fp.ys * circ[1]
    r = 6.3
    tabs = [p for p in fp.pads() if p["shape"] == "rect"]
    # The open (insertion) side is where the Fab outline reaches past the circle.
    ext = fab[2] if abs(fab[2] - ccx) > abs(fab[0] - ccx) else fab[0]
    sgn = 1 if ext > ccx else -1
    outer = [(ccx + r * math.cos(math.pi / 2 + sgn * math.pi * i / 24), ccy + r * math.sin(math.pi / 2 + sgn * math.pi * i / 24))
             for i in range(25)]
    outer += [(ext, ccy - r), (ext, ccy + r)]
    slots = []
    for oy in (-2.6, 2.6):
        slot = []
        for i in range(10):
            t = -math.pi / 2 + math.pi * i / 9
            slot.append((ccx + 1.6 + 0.9 * math.cos(t), ccy + oy + 0.9 * math.sin(t)))
        for i in range(10):
            t = math.pi / 2 + math.pi * i / 9
            slot.append((ccx - 2.6 + 0.9 * math.cos(t), ccy + oy + 0.9 * math.sin(t)))
        slots.append(slot)
    part.prism(outer, slots, 2.95, 3.2, "Nickel")
    for p in tabs:
        s = 1 if p["cy"] > ccy else -1
        part.box(ccx, ccy + s * r, 3.7, 0.25, 0.0, 3.2, "Nickel")
        part.box(p["cx"], (ccy + s * r + p["cy"] + s * p["ey"]) / 2, 3.7,
                 abs(p["cy"] + s * p["ey"] - (ccy + s * r)), 0.0, 0.25, "Nickel")


def build_esp32(part, fp):
    # Espressif ESP32-WROOM-32: 18 x 25.5 x 3.1 mm, 6 mm PCB antenna end.
    silk = fp.bbox("silk_bbox")
    x0, x1 = -9.0, 9.0
    y0, y1 = silk[1], silk[3]
    ant_lo = min(y0, y1) if any(p["cy"] > 6 for p in fp.pads()) else max(y0, y1)
    ant_sign = -1 if ant_lo == y0 else 1
    cy = (y0 + y1) / 2
    part.box(0, cy, 18.0, 25.5, 0.05, 0.85, "Module_pcb", bevel=0.03)
    ant_edge = cy + ant_sign * 25.5 / 2
    can_y0 = ant_edge - ant_sign * 6.4
    can_y1 = cy - ant_sign * (25.5 / 2 - 0.35)
    part.box(0, (can_y0 + can_y1) / 2, 17.4, abs(can_y1 - can_y0), 0.85, 3.1, "Steel_brushed", bevel=0.25)
    # Meander antenna trace on the module's PCB.
    ya = ant_edge - ant_sign * 1.0
    xs = [-7.5 + 1.5 * k for k in range(10)]
    for k, x in enumerate(xs):
        part.box(x, ya - ant_sign * 1.8, 0.35, 3.6, 0.85, 0.87, "ENIG")
        if k < len(xs) - 1:
            yy = ya if k % 2 == 0 else ya - ant_sign * 3.6
            part.box(x + 0.75, yy, 1.85, 0.35, 0.85, 0.87, "ENIG")
    for p in fp.pads():
        if p["ex"] > 2 and p["ey"] > 2:
            continue
        if abs(p["cx"]) > 7.5:
            part.box(math.copysign(9.0, p["cx"]), p["cy"], 0.5, 0.9, 0.05, 0.85, "ENIG")
        else:
            part.box(p["cx"], cy - ant_sign * 12.75, 0.9, 0.5, 0.05, 0.85, "ENIG")
    # Can marking: text "up" points at the antenna, as on the real module.
    ang = 0 if ant_sign > 0 else 180
    a = math.radians(ang)
    mid = (can_y0 + can_y1) / 2
    for body, size, off in (("ESPRESSIF", 1.6, 5.5), ("ESP32-WROOM-32", 1.3, 3.2),
                            ("FCC ID:2AC7Z-ESPWROOM32", 0.6, 0.0), ("CE", 1.5, -4.0)):
        ox, oy = -math.sin(a) * off, math.cos(a) * off
        part.text(body, size, ox, mid + oy, 3.103, ang, "Marking_dark")


def rot_rect(r, alpha):
    x0, y0, x1, y1 = r
    ca, sa = math.cos(alpha), math.sin(alpha)
    pts = [(x * ca - y * sa, x * sa + y * ca) for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1))]
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs), max(ys))


def conn_frame(fp, board):
    """Angle that turns a connector modelled with its mouth at -y toward the board
    edge, and the Fab box in that connector frame."""
    dx, dy = fp.edge_dir(board)
    alpha = math.atan2(dy, dx) + math.pi / 2
    fab = fp.bbox("fab_bbox")
    return alpha, rot_rect(fab, -alpha)


def build_rings(part, fp):
    """Plated rings of unpopulated through-hole pads, top and bottom."""
    for p in fp.pads():
        if p["type"] != "thru_hole" or not p["drill"]:
            continue
        hole = circle(p["cx"], p["cy"], p["drill"]["w"] / 2, 20, reverse=True)
        outer = pad_polygon(p, 24)
        part.prism(outer, [hole], 0.0, 0.035, "ENIG")
        part.prism(outer, [hole], -T_BOARD - 0.035, -T_BOARD, "ENIG")


# --- board -------------------------------------------------------------------

def build_board(data, collection):
    b = data["board"]
    cx, cy = (b["x0"] + b["x1"]) / 2, (b["y0"] + b["y1"]) / 2

    def bl(x, y):
        return (x - cx, cy - y)

    outer = [bl(x, y) for x, y in data["outline"]]
    area = sum(outer[i][0] * outer[(i + 1) % len(outer)][1] - outer[(i + 1) % len(outer)][0] * outer[i][1]
               for i in range(len(outer)))
    if area < 0:
        outer = outer[::-1]
    holes = []
    for h in data["holes"]:
        x, y = bl(h["x"], h["y"])
        r = h["w"] / 2
        segs = max(16, int(r * 2 * math.pi / 0.25))
        holes.append((circle(x, y, r, segs, reverse=True), h["plated"]))

    part = Part()
    pcb_top, pcb_bot = pcb_material("top"), pcb_material("bottom")
    loops = [outer] + [h[0] for h in holes]
    flat = [Vector((x, y, 0)) for loop in loops for (x, y) in loop]
    tris = tessellate_polygon([[Vector((x, y, 0)) for (x, y) in loop] for loop in loops])
    bm = part.bm
    top = [bm.verts.new((p.x, p.y, Z_TOP)) for p in flat]
    bot = [bm.verts.new((p.x, p.y, -Z_TOP)) for p in flat]
    uv = bm.loops.layers.uv.new("UVMap")
    w, h = b["width"], b["height"]
    i_top, i_bot = part.mi(pcb_top), part.mi(pcb_bot)
    i_edge, i_plate = part.mi("PCB_edge"), part.mi("ENIG")

    def set_uv(face):
        for loop in face.loops:
            co = loop.vert.co
            loop[uv].uv = ((co.x + w / 2) / w, (co.y + h / 2) / h)

    for a, bb, c in tris:
        f = bm.faces.new((top[a], top[bb], top[c]))
        f.material_index = i_top
        set_uv(f)
        f = bm.faces.new((bot[a], bot[c], bot[bb]))
        f.material_index = i_bot
        set_uv(f)
    start = 0
    for k, loop in enumerate(loops):
        n = len(loop)
        idx = i_edge if k == 0 else (i_plate if holes[k - 1][1] else i_edge)
        for i in range(n):
            a, c = start + i, start + (i + 1) % n
            f = bm.faces.new((bot[a], bot[c], top[c], top[a]))
            f.material_index = idx
        start += n
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    ob = part.to_object("PCB", collection, smooth_angle=20)
    ob["design"] = "ULX3S v3.0.8"
    ob["size_mm"] = f"{w:.2f} x {h:.2f} x {T_BOARD}"
    return ob, (cx, cy)


# --- assembly ----------------------------------------------------------------

def populated(rec):
    """Fitted unless KiCad marks it virtual or every 3D model of it is switched off
    (SD1 and SW1 carry a disabled alternative model next to the fitted one)."""
    if rec["attr"] == "virtual":
        return False
    models = rec["models"]
    return not models or any(not m["path"].endswith(UNPOPULATED_SUFFIXES) for m in models)


def place(ob, rec, center):
    x, y = rec["x"] - center[0], center[1] - rec["y"]
    rot = math.radians(rec["rot"])
    m = Matrix.Translation((x, y, Z_TOP if rec["side"] == "top" else -Z_TOP)) @ Matrix.Rotation(rot, 4, "Z")
    if rec["side"] == "bottom":
        m = m @ Matrix.Rotation(math.pi, 4, "X")
    ob.matrix_world = m


def tag(ob, rec):
    ob["ref"] = rec["ref"]
    ob["value"] = rec["value"]
    ob["footprint"] = f'{rec["lib"]}:{rec["name"]}'
    ob["side"] = rec["side"]
    ob["kicad_x"] = rec["x"]
    ob["kicad_y"] = rec["y"]
    ob["kicad_rot"] = rec["rot"]


def child(part, name, parent, collection, offset):
    ob = part.to_object(name, collection)
    ob.parent = parent
    ob.matrix_parent_inverse = Matrix.Identity(4)
    ob.location = (offset[0], offset[1], 0)
    return ob


def build_headers(rec, collection, center):
    """Optional straight 2x20 male header; drills, rather than the angled Fab."""
    pads = [p for p in FP(rec).pads() if p["type"] == "thru_hole"]
    assert len(pads) == 40
    part = Part()
    # Individual moulded stations give the familiar shallow divisions at 2.54 pitch.
    for x in sorted({round(p["cx"], 3) for p in pads}):
        ys = [p["cy"] for p in pads if abs(p["cx"] - x) < 0.01]
        part.box(x, sum(ys) / len(ys), 2.54, 5.08, 0, 2.5,
                 "Plastic_black", bevel=0.1, segments=1)
    for p in pads:
        x, y = p["cx"], p["cy"]
        part.box(x, y, 0.64, 0.64, -T_BOARD - 3.0, 8.5,
                 "ENIG", bevel=0.05, segments=1)
        # Sloped annulus on the underside, meeting the square pin with no open centre.
        outer = circle(x, y, 0.82, 12)
        inner = []
        for i in range(12):
            angle = 2 * math.pi * i / 12
            co, si = math.cos(angle), math.sin(angle)
            radius = 0.32 / max(abs(co), abs(si))
            inner.append((x + radius * co, y + radius * si))
        vs0 = [part.bm.verts.new((a, b, -T_BOARD - 0.04)) for a, b in outer]
        vs1 = [part.bm.verts.new((a, b, -T_BOARD - 0.42)) for a, b in inner]
        for i in range(12):
            j = (i + 1) % 12
            part.bm.faces.new((vs0[i], vs1[i], vs1[j], vs0[j])).material_index = part.mi("Tin")
    ob = part.to_object(rec["ref"] + "_header", collection)
    place(ob, rec, center)
    ob["ref"], ob["value"], ob["option"] = rec["ref"], "2x20 pin header", "headers"
    ob.hide_render = True
    return ob


def build_oled(rec, collection, center):
    """Pins-up display frame: GND -> CS is +x; away from the header is -y.

    Match the actual nets, including the module's names for power and SPI,
    rather than relying on the footprint's pad numbering or rotation.
    """
    pins = FP(rec).pads()
    nets = ("GND", "+3V3", "OLED_CLK", "OLED_MOSI", "OLED_RES", "OLED_DC", "OLED_CS")
    pads = [next(p for p in pins if p["net"] == net) for net in nets]
    gnd = Vector((pads[0]["cx"], pads[0]["cy"], 0))
    cs = Vector((pads[-1]["cx"], pads[-1]["cy"], 0))
    xaxis = (cs - gnd).normalized()
    yaxis = Vector((0, 0, 1)).cross(xaxis)
    mid = (gnd + cs) / 2
    rotation = Matrix(((xaxis.x, yaxis.x, 0, 0), (xaxis.y, yaxis.y, 0, 0),
                       (0, 0, 1, 0), (0, 0, 0, 1)))
    for i, p in enumerate(pads):
        expected = mid + xaxis * ((i - 3) * 2.54)
        assert (Vector((p["cx"], p["cy"], 0)) - expected).length < 0.01, nets
    # Header edge y=0, seven pins y=-3.0. PCB underside 11 mm above board top.
    part = Part()
    sockets = [body_rect((i - 3) * 2.54, -3, 0.95, 0.95) for i in range(7)]
    holes = [[(x0, y0), (x1, y0), (x1, y1), (x0, y1)] for x0, y0, x1, y1 in sockets]
    part.prism([(-8.89, -4.27), (8.89, -4.27), (8.89, -1.73), (-8.89, -1.73)],
               holes, 0, 8.5, "Plastic_black")
    part.box(0, -3, 17.78, 2.54, 8.5, 11.0, "Plastic_black", bevel=0.12)
    for i in range(7):
        x = (i - 3) * 2.54
        part.box(x, -3, 0.64, 0.64, 5.0, 13.0, "ENIG", bevel=0.04)
        part.cyl(x, -3, 0.9, 12.60, 12.63, "ENIG", segs=20)
    pcb_holes = [circle(x, y, 1.1) for x in (-11.85, 11.85) for y in (-2, -28.1)]
    part.prism([(-13.85, -30.1), (13.85, -30.1), (13.85, 0), (-13.85, 0)],
               pcb_holes, 11.0, 12.6, "OLED_pcb")
    part.box(0, -15.3, 25.9, 21.6, 12.6, 13.6, "OLED_glass", bevel=0.08)
    # Separate active face. Blender's glTF exporter flips V: Blender v=1 at
    # the header becomes exported v=0, the image's top row.
    uv = part.bm.loops.layers.uv.new("UVMap")
    corners = [(-10.07, -6.5), (-10.07, -19.92), (10.07, -19.92), (10.07, -6.5)]
    face = part.bm.faces.new([part.bm.verts.new((x, y, 13.62)) for x, y in corners])
    face.material_index = part.mi("OLED_screen")
    for loop, coord in zip(face.loops, ((0, 1), (0, 0), (1, 0), (1, 1))):
        loop[uv].uv = coord
    # Dark bonded foot and the orange ribbon folded over the far PCB edge.
    part.box(0, -25.9, 22.0, 3.2, 12.61, 13.0, "Plastic_black")
    for a, b in (((0, -25.9, 13.02), (0, -29.8, 13.02)),
                 ((0, -29.8, 13.02), (0, -30.25, 12.5)),
                 ((0, -30.25, 12.5), (0, -30.25, 10.8)),
                 ((0, -30.25, 10.8), (0, -27.8, 10.8))):
        part.seg(a, b, 19.0, 0.16, "OLED_flex")
    ob = part.to_object("OLED_module", collection)
    place(ob, rec, center)
    ob.matrix_world = ob.matrix_world @ Matrix.Translation(mid + yaxis * 3) @ rotation
    ob["ref"], ob["value"], ob["option"] = "OLED1", "SSD1331 0.95in 96x64 OLED", "oled"
    ob["pin_nets"] = json.dumps([[p["num"], p["net"]] for p in pads])
    ob.hide_render = True
    return ob


def build_hdmi_plug(rec, board, collection, center, zmin):
    """Plug and cable in the same mouth frame used by build_hdmi (-y outward)."""
    fp = FP(rec)
    alpha, fab = conn_frame(fp, board)
    cx, mouth = (fab[0] + fab[2]) / 2, fab[1] - 1.6
    part = Part()
    # 13.9 x 4.45 mm metal shell, inserted 7 mm, with 3 mm exposed.
    outer = [(-6.95, 1.8), (-6.95, 5.125), (6.95, 5.125),
             (6.95, 1.8), (5.825, 0.675), (-5.825, 0.675)]
    to_y = Matrix(((1, 0, 0, cx), (0, 0, 1, mouth - 3), (0, 1, 0, 0), (0, 0, 0, 1)))
    part.prism(outer, [], 0, 10, "Steel", matrix=to_y)
    part.box(cx, mouth - 17, 21, 28, -2.6, 8.4, "Plastic_black", bevel=1.8, segments=4)
    part.box(cx, mouth - 34, 8.5, 8, -1.1, 6.9, "Plastic_black", bevel=0.8, segments=4)
    # A bevelled Bezier curve, converted to mesh before joining the plug.
    cu = own(bpy.data.curves.new("HDMI_cable", "CURVE"))
    cu.dimensions = "3D"
    cu.resolution_u = 20
    cu.bevel_depth = 3.25
    cu.bevel_resolution = 4
    cu.use_fill_caps = True
    spline = cu.splines.new("BEZIER")
    points = [(cx, mouth - 36, 2.9), (cx, mouth - 52, 0),
              (cx + 7, mouth - 72, zmin - Z_TOP + 3.25),
              (cx + 24, mouth - 104, zmin - Z_TOP + 3.25)]
    spline.bezier_points.add(len(points) - 1)
    for p, co in zip(spline.bezier_points, points):
        p.co = co
        p.handle_left_type = p.handle_right_type = "AUTO"
    # Keep the last span level, with no automatic-handle overshoot below ground.
    for i in (2, 3):
        end = spline.bezier_points[i]
        end.handle_left_type = end.handle_right_type = "ALIGNED"
        end.handle_left = Vector(points[i]) - Vector((5.67, -10.67, 0))
        end.handle_right = Vector(points[i]) + Vector((5.67, -10.67, 0))
    cable = own(bpy.data.objects.new("_HDMI_cable", cu))
    collection.objects.link(cable)
    dg = bpy.context.evaluated_depsgraph_get()
    mesh = bpy.data.meshes.new_from_object(cable.evaluated_get(dg))
    bm = bmesh.new()
    bm.from_mesh(mesh)
    idx = part.mi("Plastic_black")
    vm = {v: part.bm.verts.new(v.co) for v in bm.verts}
    for f in bm.faces:
        part.bm.faces.new([vm[v] for v in f.verts]).material_index = idx
    bm.free()
    bpy.data.meshes.remove(mesh)
    bpy.data.objects.remove(cable, do_unlink=True)
    bpy.data.curves.remove(cu)
    part.transform_new(0, Matrix.Rotation(alpha, 4, "Z"))
    ob = part.to_object("HDMI_plug", collection)
    place(ob, rec, center)
    ob["ref"], ob["value"], ob["option"] = "GPDI1", "HDMI cable", "hdmi"
    ob["cable_end"] = list(Matrix.Rotation(alpha, 4, "Z") @ Vector(points[-1]))
    ob.hide_render = True
    return ob


def build_footprint(rec, board, cols, center):
    fp = FP(rec)
    name = fp.rec["name"]
    lib = fp.rec["lib"]
    col = cols["bottom"] if fp.bottom else cols["top"]
    part = Part()
    extras = []  # (Part, name, offset)
    sliders = []  # (Part, offset, name suffix, switch index, board-frame travel)
    accessory = None

    if not populated(rec):
        if any(p["type"] == "thru_hole" for p in rec["pads"]):
            build_rings(part, fp)
        else:
            return None
    elif name.startswith("MountingHole"):
        build_rings(part, fp)
    elif name.startswith(("R_0603", "R_0805")):
        build_chip(part, fp, 0.45 if "0603" in name else 0.5, "Resistor_black", 0.3 if "0603" in name else 0.4)
    elif name.startswith("C_0603"):
        build_chip(part, fp, 0.8, "Ceramic_cap", 0.3)
    elif name.startswith("C_0805"):
        big = rec["value"].endswith("uF") and float(rec["value"][:-2]) >= 10
        build_chip(part, fp, 1.25 if big else 0.95, "Ceramic_cap", 0.4)
    elif name.startswith("L_0805"):
        build_chip(part, fp, 0.9, "Ferrite", 0.4)
    elif name.startswith("L_1008"):
        build_chip(part, fp, 1.4, "Ferrite_grey", 0.5)
    elif name.startswith("LED_0805"):
        build_led(part, fp)
    elif name.startswith("D_SOD-323"):
        build_sod323(part, fp)
    elif name == "D_SMA":
        build_sma(part, fp)
    elif name == "SOT-23":
        cols_x = pins_on_x(fp)  # SOT-23: 2.9 x 1.3 x 1.0 mm
        build_gullwing(part, fp, 1.3 if cols_x else 2.9, 2.9 if cols_x else 1.3, 1.0, max_w=0.4)
    elif name == "TSOT-25":
        cols_x = pins_on_x(fp)  # TSOT-23-5: 2.9 x 1.6 x 0.87 mm
        build_gullwing(part, fp, 1.6 if cols_x else 2.9, 2.9 if cols_x else 1.6, 0.87, max_w=0.4)
    elif name == "TSOP54":
        build_tsop54(part, fp)
    elif name.startswith("BGA-381"):
        build_bga(part, fp)
    elif name.startswith("MAX1112x"):
        build_qfn(part, fp, 5.0, 0.75)
    elif name.startswith("FT231X"):
        cols_x = pins_on_x(fp)  # footprint body 4.4 x 6.5 mm, 0.65 mm pitch
        build_gullwing(part, fp, 4.4 if cols_x else 6.5, 6.5 if cols_x else 4.4, 1.1, max_w=0.3)
    elif name in ("SOT96-1", "SM8"):
        cols_x = pins_on_x(fp)  # SO-8: 3.9 x 4.9 x 1.5 mm
        build_gullwing(part, fp, 3.9 if cols_x else 4.9, 4.9 if cols_x else 3.9, 1.5, max_w=0.42)
    elif name.startswith("SOA008"):
        build_gullwing(part, fp, 5.28, 5.28, 1.9, max_w=0.45)  # SOIC-8 208 mil: 5.28 x 5.28 x 2.16 max
    elif name == "SW_SPST_PTS645":
        plunger, off = build_button(part, fp, col)
        extras.append((plunger, f"{rec['ref']}_plunger", off))
    elif name.startswith("USB-MICRO-B"):
        build_usb(part, fp, board)
    elif name.startswith("CONN-10029449"):
        build_hdmi(part, fp, board)
    elif name.startswith("CUI_SJ-43516"):
        build_jack(part, fp, board)
    elif name == "SCHD3A100":
        accessory = build_sd(part, fp, board, cols["accessories"])
    elif name.startswith("SW_DIP_x4"):
        sliders = build_dip(part, fp, col)
    elif name.startswith("Crystal_SMD_7050"):
        build_osc7050(part, fp)
    elif name == "Crystal_SMD_ABS25":
        build_abs25(part, fp)
    elif name.startswith("Keystone_3000"):
        build_battery_holder(part, fp)
    elif name == "ESP32-WROOM32":
        build_esp32(part, fp)
    else:
        part.bm.free()
        return ("skipped", f"{rec['ref']} {lib}:{name}")

    ob = part.to_object(rec["ref"], col)
    place(ob, rec, center)
    tag(ob, rec)
    ob["populated"] = populated(rec)
    if any(p["type"] == "thru_hole" for p in rec["pads"]) and not populated(rec):
        ob["pins"] = json.dumps([[p["num"], round(p["cx"], 3), round(p["cy"], 3)] for p in fp.pads()])
    if rec["ref"] in [f"D{i}" for i in range(8)]:
        ob["led_index"] = int(rec["ref"][1:])
    if name.startswith("LED_0805"):
        ob["led_color"] = rec["value"]
        ob["led_material"] = f"LED_{rec['ref']}"
    if rec["ref"] in BUTTONS:
        ob["btn_index"] = int(rec["ref"][1:])
        ob["btn_name"] = BUTTONS[rec["ref"]]
    for p, nm, off in extras:
        c = child(p, nm, ob, col, off)
        c["ref"] = rec["ref"]
    for p, off, k, index, travel in sliders:
        c = child(p, f"{rec['ref']}_slider{k}", ob, col, off)
        c["ref"] = rec["ref"]
        c["sw_index"], c["sw_travel"] = index, travel
    if accessory is not None:
        acc = accessory.to_object("microSD_card", cols["accessories"])
        acc.matrix_world = ob.matrix_world.copy()
        acc["note"] = "Not part of the board: a microSD card in SD1."
        acc["card_out_mm"] = SD_CARD_OUT
    return ob


def setup_scene():
    """A fresh ULX3S scene; removes only what an earlier run of this script made."""
    old = bpy.data.scenes.get(SCENE)
    if old is not None:
        if bpy.context.window and bpy.context.window.scene == old:
            other = next((s for s in bpy.data.scenes if s != old), None)
            if other is None:
                other = bpy.data.scenes.new("Scene")
            bpy.context.window.scene = other
        bpy.data.scenes.remove(old)
    for block in (bpy.data.objects, bpy.data.collections, bpy.data.meshes, bpy.data.materials,
                  bpy.data.images, bpy.data.cameras, bpy.data.lights, bpy.data.worlds, bpy.data.fonts):
        for item in list(block):
            if item.get(TAG) or (block == bpy.data.collections and item.name.startswith("ULX3S_")):
                block.remove(item)
    scene = own(bpy.data.scenes.new(SCENE))
    if bpy.context.window:
        bpy.context.window.scene = scene
    us = scene.unit_settings
    us.system = "METRIC"
    us.scale_length = 0.001
    us.length_unit = "MILLIMETERS"
    root = own(bpy.data.collections.new("ULX3S"))
    scene.collection.children.link(root)
    cols = {}
    for key, title in (("pcb", "ULX3S_PCB"), ("top", "ULX3S_Top"), ("bottom", "ULX3S_Bottom"),
                       ("accessories", "ULX3S_Accessories"), ("options", "ULX3S_Options"),
                       ("studio", "ULX3S_Studio")):
        c = own(bpy.data.collections.new(title))
        (scene.collection if key == "studio" else root).children.link(c)
        cols[key] = c
    return scene, cols


def setup_studio(scene, cols, zmin):
    world = own(bpy.data.worlds.new("ULX3S_World"))
    world.use_nodes = True
    bg = world.node_tree.nodes.get("Background")
    bg.inputs["Color"].default_value = (1, 1, 1, 1)
    bg.inputs["Strength"].default_value = 1.0  # a light tent, as in the photo
    scene.world = world

    ground = Part()
    ground.box(0, 0, 2000, 2000, zmin - 0.5, zmin, make_material("Ground", (250, 250, 250), 0, 0.8, {}))
    g = ground.to_object("Ground", cols["studio"])
    g.hide_select = True
    g.is_shadow_catcher = True  # renders are composited on white by render_qa.py

    def light(name, loc, rot, size, power):
        ld = own(bpy.data.lights.new(name, "AREA"))
        ld.shape = "DISK"
        ld.size = size
        ld.energy = power
        ob = own(bpy.data.objects.new(name, ld))
        ob.location = loc
        ob.rotation_euler = rot
        cols["studio"].objects.link(ob)
        return ob

    def aim(ob, target=(0, 0, 0)):
        d = Vector(target) - ob.location
        ob.rotation_euler = d.to_track_quat("-Z", "Y").to_euler()

    k = light("Key", (-60, -120, 220), (0, 0, 0), 180, 0.5e6)
    aim(k)
    f = light("Fill", (160, 60, 150), (0, 0, 0), 200, 0.2e6)
    aim(f)
    # No light straight above: the glossy mask would mirror it into the top view.

    def cam(name, loc, target, lens=None, ortho=None):
        cd = own(bpy.data.cameras.new(name))
        cd.clip_start = 1
        cd.clip_end = 5000
        if ortho:
            cd.type = "ORTHO"
            cd.ortho_scale = ortho
        else:
            cd.lens = lens
        ob = own(bpy.data.objects.new(name, cd))
        ob.location = loc
        cols["studio"].objects.link(ob)
        aim(ob, target)
        return ob

    top = cam("Cam_Top", (0, 0, 400), (0, 0, 0), ortho=104)
    top.rotation_euler = (0, 0, 0)
    bot = cam("Cam_Bottom", (0, 0, -400), (0, 0, 0), ortho=104)
    bot.rotation_euler = (0, math.pi, 0)  # seen from below, mirrored left-right
    # Fitted to the radiona.org photo (7 landmarks: mounting holes and board corners,
    # 1.8 px RMS, principal point at the frame centre) for a 2000 x 838 frame.
    photo = cam("Cam_Photo", (-132.24, -244.39, 410.51), (0, 0, 0), lens=104.05)
    photo.rotation_euler = (math.radians(34.05), math.radians(0.93), math.radians(-26.21))
    photo.data.sensor_fit = "HORIZONTAL"
    photo.data.sensor_width = 36.0
    cam("Cam_Angle_Bottom", (95, -125, -120), (0, 0, -3), lens=85)
    scene.camera = photo

    scene.render.engine = "CYCLES"
    scene.cycles.device = "GPU"
    scene.cycles.samples = 128
    scene.cycles.use_denoising = True
    scene.render.resolution_x = 1800
    scene.render.resolution_y = 1100
    scene.render.film_transparent = True
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Punchy"


def main():
    data = json.loads(paths.BOARD_JSON.read_text(encoding="utf-8"))
    MATS.clear()
    scene, cols = setup_scene()
    pcb, center = build_board(data, cols["pcb"])
    built, skipped, rings = [], [], []
    for rec in data["footprints"]:
        res = build_footprint(rec, data["board"], cols, center)
        if res is None:
            continue
        if isinstance(res, tuple):
            skipped.append(res[1])
        else:
            built.append(res.name)
            if not res.get("populated", True):
                rings.append(res.name)
    bpy.context.view_layer.update()
    zmin = min((ob.matrix_world @ Vector(c)).z for ob in scene.objects if ob.type == "MESH"
               for c in ob.bound_box)
    by_ref = {rec["ref"]: rec for rec in data["footprints"]}
    build_oled(by_ref["OLED1"], cols["options"], center)
    build_hdmi_plug(by_ref["GPDI1"], data["board"], cols["options"], center, zmin)
    for ref in ("J1", "J2"):
        build_headers(by_ref[ref], cols["options"], center)
    setup_studio(scene, cols, zmin)
    for img in bpy.data.images:
        if img.name.startswith("ulx3s_") and img.packed_file is None:
            img.pack()
    paths.OUT.mkdir(parents=True, exist_ok=True)
    # Write only the ULX3S scene and what it uses, whatever else the session holds.
    bpy.data.libraries.write(str(paths.BLEND), {scene}, path_remap="NONE", compress=True)
    return {"built": len(built), "unpopulated_th": rings, "skipped": skipped,
            "zmin": zmin, "blend": str(paths.BLEND.relative_to(paths.REPO))}


if __name__ == "__main__":
    RESULT = main()
    print(RESULT)
