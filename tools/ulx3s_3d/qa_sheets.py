"""Assemble the ULX3S model QA sheets from the renders of ``render_qa.py``.

Writes to ``docs/qa/ulx3s_3d/``:
- ``render_{top,photo,bottom,angle_bottom}.jpg``: the renders;
- ``compare_{top,bottom}.jpg``: the ULX3S v3.0.3 board photos shipped with the
  design (``assets/ulx3s/ULX3S_v303_*.png``) above the orthographic renders.

With ``--photo PATH`` (a screenshot of radiona.org/ulx3s, 2000 x 1293, whose photo
area is rows 184..1022) it also writes, to ``out/ulx3s_3d/renders/`` only (the
photo's licence is unclear, so it stays out of git): the photo above the
``Cam_Photo`` render (same frame) and three zoomed side-by-side crops.

Usage: python3 tools/ulx3s_3d/qa_sheets.py [--photo PATH]
"""

import argparse
import sys
from pathlib import Path

from PIL import Image, ImageDraw

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths  # noqa: E402

PHOTO_AREA = (0, 184, 2000, 1022)  # the hero photo inside the radiona.org screenshot


def label(im, text):
    d = ImageDraw.Draw(im)
    d.rectangle([0, 0, 7 * len(text) + 16, 22], fill="white")
    d.text((8, 5), text, fill="black")
    return im


def stack(a, b):
    out = Image.new("RGB", (max(a.width, b.width), a.height + b.height), "white")
    out.paste(a, (0, 0))
    out.paste(b, (0, a.height))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--photo", type=Path, help="radiona.org/ulx3s screenshot (not in git)")
    args = ap.parse_args()
    paths.QA.mkdir(parents=True, exist_ok=True)

    for n in ("top", "photo", "bottom", "angle_bottom"):
        Image.open(paths.RENDERS / f"{n}.png").convert("RGB").save(paths.QA / f"render_{n}.jpg", quality=92)

    for side in ("top", "bottom"):
        ph = Image.open(paths.ASSETS / f"ULX3S_v303_{side}.png").convert("RGB")
        rn = Image.open(paths.RENDERS / f"{side}.png").convert("RGB")
        w = 1400
        ph = ph.resize((w, round(ph.height * w / ph.width)), Image.Resampling.LANCZOS)
        rn = rn.resize((w, round(rn.height * w / rn.width)), Image.Resampling.LANCZOS)
        label(ph, f"photo: ULX3S v3.0.3 {side} (emard/ulx3s, pic/ULX3S_v303_{side}.png)")
        label(rn, f"render: Cam_{side.capitalize()} (orthographic), model of v3.0.8")
        stack(ph, rn).save(paths.QA / f"compare_{side}.jpg", quality=90)

    if args.photo:
        ref = Image.open(args.photo).convert("RGB").crop(PHOTO_AREA)
        ren = Image.open(paths.RENDERS / "photo.png").convert("RGB")
        box = (560, 0, 1880, 838)
        stack(label(ref.crop(box), "photo: radiona.org/ulx3s"),
              label(ren.crop(box), "render: Cam_Photo, camera fitted to the photo")
              ).save(paths.RENDERS / "compare_radiona_photo.png")
        for name, bx in (("dip_hdmi", (1150, 60, 1650, 330)), ("sd_leds", (700, 300, 1200, 838)),
                         ("right", (1350, 250, 1880, 700))):
            a, b = ref.crop(bx), ren.crop(bx)
            o = Image.new("RGB", (2 * a.width + 10, a.height), "white")
            o.paste(a, (0, 0))
            o.paste(b, (a.width + 10, 0))
            o.save(paths.RENDERS / f"zoom_{name}.png")
    print(f"sheets in {paths.QA.relative_to(paths.REPO)}")


if __name__ == "__main__":
    main()
