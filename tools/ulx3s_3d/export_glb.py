"""Export the ULX3S model to glTF binary for the GUI.

Run headless on the .blend that ``build_blend.py`` wrote::

    blender -b out/ulx3s_3d/ulx3s_v3.0.8.blend -P tools/ulx3s_3d/export_glb.py

Writes ``out/ulx3s_3d/ulx3s_v3.0.8.glb``: the ``ULX3S`` collection and its children
(board, parts, the microSD card, hidden OLED/HDMI options; not the studio), with
the objects' custom properties as glTF ``extras`` (ref, value, led_index, btn_index,
...). glTF is Y-up: a Blender point (x, y, z) is (x, z, -y) in the file, in mm.
"""

import sys
from pathlib import Path

import bpy

sys.path.insert(0, str(Path(__file__).resolve().parent))
import importlib  # noqa: E402

import paths  # noqa: E402

importlib.reload(paths)

GLB = paths.OUT / "ulx3s_v3.0.8.glb"


def main():
    scene = bpy.data.scenes["ULX3S"]
    if bpy.context.window:
        bpy.context.window.scene = scene
    view_layer = scene.view_layers[0]
    view_layer.active_layer_collection = view_layer.layer_collection.children["ULX3S"]
    with bpy.context.temp_override(scene=scene, view_layer=view_layer):
        bpy.ops.export_scene.gltf(
            filepath=str(GLB),
            export_format="GLB",
            use_active_collection=True,
            use_active_collection_with_nested=True,
            export_extras=True,
            use_renderable=False,  # options default to hide_render, but the GUI needs them
            use_visible=False,
            export_texcoords=True,
            export_cameras=False,
            export_lights=False,
            export_apply=True,
        )
    return {"glb": str(GLB.relative_to(paths.REPO)), "bytes": GLB.stat().st_size}


if __name__ == "__main__":
    RESULT = main()
    print(RESULT)
