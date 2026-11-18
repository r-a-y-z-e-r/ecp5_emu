#!/usr/bin/env python3
"""Rebuild checked-in application icons on macOS; no Python packages required.

Requires rsvg-convert on PATH and /usr/bin/iconutil. Packaging consumes the
generated files directly and never needs either tool. Run from any directory.
"""

from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib


ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "assets/icon"
THEME = ROOT / "site/theme"
LINUX_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
MAC_SIZES = (16, 32, 128, 256, 512)
ICONUTIL = Path("/usr/bin/iconutil")


def png_rgba(png: bytes) -> bytes:
    """Decode rsvg-convert's non-interlaced, 8-bit RGBA PNG for legacy ICNS slots."""
    width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", png[16:29])
    if (depth, color, interlace) != (8, 6, 0):
        raise ValueError("expected a non-interlaced 8-bit RGBA PNG from rsvg-convert")
    compressed = []
    offset = 8
    while offset < len(png):
        length = struct.unpack_from(">I", png, offset)[0]
        if png[offset + 4:offset + 8] == b"IDAT":
            compressed.append(png[offset + 8:offset + 8 + length])
        offset += length + 12
    filtered = zlib.decompress(b"".join(compressed))
    stride = width * 4
    previous = bytearray(stride)
    pixels = bytearray()
    for y in range(height):
        start = y * (stride + 1)
        method = filtered[start]
        row = bytearray(filtered[start + 1:start + 1 + stride])
        for x in range(stride):
            left = row[x - 4] if x >= 4 else 0
            above = previous[x]
            upper_left = previous[x - 4] if x >= 4 else 0
            if method == 0:
                predictor = 0
            elif method == 1:
                predictor = left
            elif method == 2:
                predictor = above
            elif method == 3:
                predictor = (left + above) // 2
            elif method == 4:
                estimate = left + above - upper_left
                predictor = min((left, above, upper_left), key=lambda value: abs(estimate - value))
            else:
                raise ValueError(f"unknown PNG row filter: {method}")
            row[x] = (row[x] + predictor) & 255
        pixels.extend(row)
        previous = row
    return bytes(pixels)


def pack_iconset(iconset: Path) -> bytes:
    """Pack lossless representations when the system encoder is unavailable.

    Use legacy RGB/alpha chunks for 16/32 at 1x (macOS does not reliably read
    PNGs in those slots), and PNG chunks for Retina and larger representations.
    """
    slots = (
        (b"ic11", 16, 2), (b"ic12", 32, 2),
        (b"ic07", 128, 1), (b"ic13", 128, 2),
        (b"ic08", 256, 1), (b"ic14", 256, 2),
        (b"ic09", 512, 1), (b"ic10", 512, 2),
    )
    chunks = []

    def chunk(kind: bytes, payload: bytes) -> None:
        chunks.append(kind + struct.pack(">I", len(payload) + 8) + payload)

    for size, rgb_kind, alpha_kind in ((16, b"is32", b"s8mk"), (32, b"il32", b"l8mk")):
        rgba = png_rgba((iconset / f"icon_{size}x{size}.png").read_bytes())
        # ICNS RLE literal blocks: a length-minus-one byte, then up to 128 bytes.
        rgb = bytearray()
        for channel in range(3):
            band = rgba[channel::4]
            for start in range(0, len(band), 128):
                literal = band[start:start + 128]
                rgb.append(len(literal) - 1)
                rgb.extend(literal)
        chunk(rgb_kind, bytes(rgb))
        chunk(alpha_kind, rgba[3::4])
    for kind, size, scale in slots:
        suffix = "@2x" if scale == 2 else ""
        png = (iconset / f"icon_{size}x{size}{suffix}.png").read_bytes()
        chunk(kind, png)
    body = b"".join(chunks)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


def without_metadata(png: bytes) -> bytes:
    """Keep PNG rendering chunks, dropping text, timestamps and EXIF metadata."""
    signature = b"\x89PNG\r\n\x1a\n"
    if not png.startswith(signature):
        raise ValueError("rsvg-convert did not produce a PNG")
    chunks = [signature]
    offset = len(signature)
    while offset < len(png):
        length = struct.unpack_from(">I", png, offset)[0]
        end = offset + 12 + length
        if end > len(png):
            raise ValueError("rsvg-convert produced a truncated PNG")
        kind = png[offset + 4:offset + 8]
        if kind not in (b"tEXt", b"zTXt", b"iTXt", b"tIME", b"eXIf"):
            chunks.append(png[offset:end])
        offset = end
    return b"".join(chunks)


def main() -> None:
    rsvg = shutil.which("rsvg-convert")
    if rsvg is None:
        raise ValueError("missing rsvg-convert on PATH; install librsvg before rebuilding icons")
    if sys.platform != "darwin" or not ICONUTIL.is_file():
        raise ValueError("rebuilding ICNS requires macOS with /usr/bin/iconutil; use the checked-in icons for packaging")
    source = ICONS / "ecp5_emu.svg"
    if not source.is_file():
        raise ValueError("missing assets/icon/ecp5_emu.svg")
    small = ICONS / "ecp5_emu-small.svg"
    if not small.is_file():
        small = source
    cache = {}

    def render(svg: Path, size: int) -> bytes:
        key = (svg, size)
        if key not in cache:
            result = subprocess.run(
                [rsvg, "--format=png", f"--width={size}", f"--height={size}", str(svg)],
                check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            cache[key] = without_metadata(result.stdout)
        return cache[key]

    # Keep temporary iconset files within the icon directory, then remove them.
    with tempfile.TemporaryDirectory(prefix=".build-", dir=ICONS) as temporary:
        scratch = Path(temporary)
        iconset = scratch / "ecp5_emu.iconset"
        iconset.mkdir()
        for size in MAC_SIZES:
            # The 16/32-point slots share the small design at both Retina scales.
            svg = small if size <= 32 else source
            for scale in (1, 2):
                suffix = "@2x" if scale == 2 else ""
                (iconset / f"icon_{size}x{size}{suffix}.png").write_bytes(render(svg, size * scale))
        icns = scratch / "ecp5_emu.icns"
        encoded = subprocess.run(
            [str(ICONUTIL), "-c", "icns", str(iconset), "-o", str(icns)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        if encoded.returncode:
            if b"Invalid Iconset" not in encoded.stderr:
                encoded.check_returncode()
            print("iconutil rejected the iconset; using lossless ICNS packing and verifying with iconutil")
            icns.write_bytes(pack_iconset(iconset))
        decoded = scratch / "verified.iconset"
        subprocess.run(
            [str(ICONUTIL), "-c", "iconset", str(icns), "-o", str(decoded)],
            check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        for original in iconset.glob("*.png"):
            recovered = decoded / original.name
            if not recovered.is_file():
                raise ValueError(f"ICNS round-trip lost {original.name}")
            # Read IHDR dimensions directly; verification needs no PNG library.
            if recovered.read_bytes()[16:24] != original.read_bytes()[16:24]:
                raise ValueError(f"ICNS round-trip changed the dimensions of {original.name}")
            if encoded.returncode and original.name in ("icon_16x16.png", "icon_32x32.png"):
                if png_rgba(recovered.read_bytes()) != png_rgba(original.read_bytes()):
                    raise ValueError(f"ICNS round-trip changed the pixels of {original.name}")
        shutil.copyfile(icns, ICONS / icns.name)

    for size in LINUX_SIZES:
        destination = ICONS / f"hicolor/{size}x{size}/apps/ecp5_emu.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(render(small if size <= 32 else source, size))
    THEME.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, THEME / "favicon.svg")
    (THEME / "favicon.png").write_bytes(render(small, 32))
    print("Built assets/icon/ecp5_emu.icns, 8 hicolor PNGs, and site/theme/favicon.{svg,png}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        detail = error.stderr.decode("utf-8", errors="replace").strip() if error.stderr else str(error)
        sys.exit(f"icon build failed: {detail}")
    except (OSError, ValueError) as error:
        sys.exit(f"icon build failed: {error}")
