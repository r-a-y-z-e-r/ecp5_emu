"""Render the QA views of the ULX3S model built by ``build_blend.py``.

Runs in the Blender session that holds the ``ULX3S`` scene (or headless on the
saved .blend). Writes PNGs to ``out/ulx3s_3d/renders/``; copy the ones worth
keeping to ``docs/qa/ulx3s_3d/``.

Views: top (orthographic), photo (perspective, like the radiona.org photo),
bottom (orthographic, mirrored left-right like the board's bottom photo),
angle_bottom (perspective from below), options (photo with OLED and HDMI fitted).

Globals honoured when exec'd: ``VIEWS`` (list of names), ``SAMPLES``, ``SCALE``
(resolution percentage).
"""

import sys
from pathlib import Path

import bpy
import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib  # noqa: E402

import paths  # noqa: E402

importlib.reload(paths)  # a live Blender session keeps modules between runs

CAMS = {
    "top": ("Cam_Top", (2000, 1100), True),
    "photo": ("Cam_Photo", (2000, 838), True),  # same frame as the photo area of the reference
    "options": ("Cam_Photo", (2000, 838), True),
    "bottom": ("Cam_Bottom", (2000, 1100), False),
    "angle_bottom": ("Cam_Angle_Bottom", (1800, 1100), False),
}


def on_white(path):
    """Composite the transparent render (board + shadow-catcher shadows) on white."""
    img = bpy.data.images.load(str(path), check_existing=False)
    px = np.empty(len(img.pixels), dtype=np.float32)
    img.pixels.foreach_get(px)
    px = px.reshape(-1, 4)
    a = px[:, 3:4]
    px[:, :3] = px[:, :3] * a + (1.0 - a)
    px[:, 3] = 1.0
    img.pixels.foreach_set(px.ravel())
    img.filepath_raw = str(path)
    img.file_format = "PNG"
    img.save()
    bpy.data.images.remove(img)


def render(views=None, samples=128, scale=100):
    scene = bpy.data.scenes["ULX3S"]
    if bpy.context.window:
        bpy.context.window.scene = scene
    ground = scene.objects.get("Ground")
    paths.RENDERS.mkdir(parents=True, exist_ok=True)
    out = []
    for name in views or CAMS:
        cam, (w, h), show_ground = CAMS[name]
        scene.camera = scene.objects[cam]
        scene.render.resolution_x = w
        scene.render.resolution_y = h
        scene.render.resolution_percentage = scale
        scene.cycles.samples = samples
        if ground is not None:
            ground.hide_render = not show_ground
        path = paths.RENDERS / f"{name}.png"
        scene.render.filepath = str(path)
        options = [(ob, ob.hide_render) for ob in scene.objects if ob.get("option")]
        try:
            for ob, _ in options:
                ob.hide_render = name != "options"
            bpy.ops.render.render(write_still=True, scene=scene.name)
        finally:
            for ob, hidden in options:
                ob.hide_render = hidden
        on_white(path)
        out.append(str(path.relative_to(paths.REPO)))
    if ground is not None:
        ground.hide_render = False
    scene.camera = scene.objects["Cam_Photo"]
    return out


if __name__ == "__main__":
    g = globals()
    RESULT = render(g.get("VIEWS"), g.get("SAMPLES", 128), g.get("SCALE", 100))
    print(RESULT)
