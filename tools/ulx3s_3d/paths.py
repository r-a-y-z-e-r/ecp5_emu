"""Repo-relative locations used by the ULX3S 3D model tools.

Everything is derived from this file's own location, so the tools work from
any checkout and from inside Blender (which runs the build script with
``__file__`` set to its path in the repo).
"""

from pathlib import Path

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parents[1]

# Third-party inputs (git LFS): the ULX3S v3.0.8 KiCad board and its licence.
ASSETS = REPO / "assets" / "ulx3s"
PCB = ASSETS / "ulx3s_v3.0.8.kicad_pcb"
# Chip markings are set in Inter (SIL OFL 1.1), copied from Blender 5.1.2's own fonts.
FONT = REPO / "assets" / "fonts" / "Inter.woff2"

# Generated, not in git: extracted geometry, layer textures, the .blend.
OUT = REPO / "out" / "ulx3s_3d"
BOARD_JSON = OUT / "board.json"
LAYERS = OUT / "layers"
TEXTURES = OUT / "textures"
BLEND = OUT / "ulx3s_v3.0.8.blend"
RENDERS = OUT / "renders"

# Finished QA renders (in git).
QA = REPO / "docs" / "qa" / "ulx3s_3d"
