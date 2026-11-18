"""Turn the ULX3S copper, solder-mask and silkscreen layers into PCB textures.

1. ``kicad-cli pcb export svg`` plots F/B.Cu, F/B.Mask and F/B.SilkS of
   ``assets/ulx3s/ulx3s_v3.0.8.kicad_pcb`` (KiCad's SVG user units are board
   millimetres, so the plot is cropped to the Edge.Cuts rectangle by rewriting
   the SVG viewBox).
2. ``rsvg-convert`` rasterises each plot at ``PX_PER_MM``.
3. The layers are composited into, per side:
   - ``<side>_color.png``: solder-mask green (lighter over copper), ENIG gold where
     the mask opens over copper, bare FR-4 where it opens over nothing, white
     silkscreen (clipped at mask openings), darkened via drills;
   - ``<side>_data.png`` (non-colour): R = relief height (copper + silk),
     G = roughness, B = metallic.

Both images use the same board frame as seen from the top (x right, y down in
KiCad = image right, image down); the bottom-side images are not mirrored.

Needs ``kicad-cli`` (on PATH, ``$KICAD_CLI``, or found via Spotlight) and
``rsvg-convert`` (Homebrew librsvg). Run ``kicad_extract.py`` first.

Usage: python3 tools/ulx3s_3d/layers.py
"""

import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths  # noqa: E402

PX_PER_MM = 40

# sRGB albedos, calibrated by rendering the model with the camera fitted to the
# radiona.org photo and matching median colours at ~3000 projected board points
# per class (on this board the mask over copper looks darker than over bare FR-4).
MASK_BARE = (22, 165, 96)
MASK_COPPER = (4, 123, 69)
ENIG = (250, 232, 185)
FR4 = (170, 150, 95)
SILK = (240, 242, 236)
VIA_HOLE = (8, 40, 24)

SIDES = {"top": "F", "bottom": "B"}


def kicad_cli():
    found = shutil.which("kicad-cli") or os.environ.get("KICAD_CLI")
    if found:
        return found
    apps = subprocess.run(
        ["mdfind", "kMDItemCFBundleIdentifier == 'org.kicad.kicad'"],
        capture_output=True, text=True,
    ).stdout.split("\n")
    for app in apps:
        cand = Path(app) / "Contents" / "MacOS" / "kicad-cli"
        if app and cand.exists():
            return str(cand)
    sys.exit("kicad-cli not found: put it on PATH or set KICAD_CLI")


def plot_layer(cli, layer, board):
    svg = paths.LAYERS / f"{layer}.svg"
    png = paths.LAYERS / f"{layer}.png"
    subprocess.run(
        [cli, "pcb", "export", "svg", "--mode-single", "--layers", layer,
         "--black-and-white", "--exclude-drawing-sheet", "--page-size-mode", "1",
         "--drill-shape-opt", "0", "-o", str(svg), str(paths.PCB)],
        check=True, capture_output=True,
    )
    text = svg.read_text(encoding="utf-8")
    w, h = board["width"], board["height"]
    text, n = re.subn(
        r'width="[^"]*mm" height="[^"]*mm" viewBox="[^"]*"',
        f'width="{w:.4f}mm" height="{h:.4f}mm" viewBox="{board["x0"]:.4f} {board["y0"]:.4f} {w:.4f} {h:.4f}"',
        text, count=1,
    )
    if n != 1:
        sys.exit(f"{svg}: unexpected SVG header")
    svg.write_text(text, encoding="utf-8")
    wpx, hpx = round(w * PX_PER_MM), round(h * PX_PER_MM)
    subprocess.run(
        ["rsvg-convert", "-w", str(wpx), "-h", str(hpx), "-o", str(png), str(svg)],
        check=True,
    )
    alpha = np.asarray(Image.open(png).convert("RGBA"))[:, :, 3].astype(np.float32) / 255.0
    return alpha


def via_mask(data, side_shape):
    """Anti-aliased (4x supersampled) mask of via drills."""
    board = data["board"]
    ss = 4
    img = Image.new("L", (side_shape[1] * ss, side_shape[0] * ss), 0)
    draw = ImageDraw.Draw(img)
    k = PX_PER_MM * ss
    for v in data["vias"]:
        cx = (v["x"] - board["x0"]) * k
        cy = (v["y"] - board["y0"]) * k
        r = v["drill"] / 2 * k
        draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
    img = img.resize((side_shape[1], side_shape[0]), Image.Resampling.BOX)
    return np.asarray(img).astype(np.float32) / 255.0


def mix(base, color, amount):
    c = np.array(color, dtype=np.float32)[None, None, :]
    a = amount[:, :, None]
    return base * (1 - a) + c * a


def main():
    data = json.loads(paths.BOARD_JSON.read_text(encoding="utf-8"))
    board = data["board"]
    paths.LAYERS.mkdir(parents=True, exist_ok=True)
    paths.TEXTURES.mkdir(parents=True, exist_ok=True)
    cli = kicad_cli()

    for side, p in SIDES.items():
        cu = plot_layer(cli, f"{p}.Cu", board)
        mask_open = plot_layer(cli, f"{p}.Mask", board)
        silk = plot_layer(cli, f"{p}.SilkS", board)
        silk = silk * (1 - mask_open)
        vias = via_mask(data, cu.shape)

        color = np.empty(cu.shape + (3,), dtype=np.float32)
        color[:] = np.array(MASK_BARE, dtype=np.float32)
        color = mix(color, MASK_COPPER, cu)
        opening = mask_open
        color = mix(color, FR4, opening * (1 - cu))
        color = mix(color, ENIG, opening * cu)
        color = mix(color, VIA_HOLE, vias * (1 - opening))
        color = mix(color, SILK, silk)

        # Relief: copper 35 um and silk 20 um above the laminate, mask 20 um over everything.
        height = 0.35 * cu + 0.25 * silk + 0.15 * (1 - opening) - 0.4 * vias
        height = np.clip(height + 0.3, 0, 1)
        rough = 0.4 * np.ones_like(cu)
        rough = rough * (1 - opening) + 0.28 * opening * cu + 0.7 * opening * (1 - cu)
        rough = rough * (1 - silk) + 0.75 * silk
        metal = opening * cu

        Image.fromarray(np.clip(color + 0.5, 0, 255).astype(np.uint8), "RGB").save(
            paths.TEXTURES / f"{side}_color.png", optimize=True)
        dmap = np.stack([height, rough, metal], axis=-1) * 255
        Image.fromarray(np.clip(dmap + 0.5, 0, 255).astype(np.uint8), "RGB").save(
            paths.TEXTURES / f"{side}_data.png", optimize=True)
        print(f"{side}: {cu.shape[1]}x{cu.shape[0]} px")


if __name__ == "__main__":
    main()
