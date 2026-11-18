#!/usr/bin/env python3
"""Offline encoding helpers; never opens a window or captures pixels."""
import json
from pathlib import Path
import re
import subprocess
import tempfile

LIMIT = 250 * 1024


def run(argv):
    subprocess.run([str(a) for a in argv], check=True)


def probe(path):
    return json.loads(subprocess.check_output([
        'ffprobe', '-v', 'error', '-show_streams', '-show_format', '-of', 'json', str(path)]))


def aspect_matches(size, other):
    return size[0] * other[1] == size[1] * other[0]


def crop_layout(trace, crop, image, rect):
    """Map logical client coordinates to backing pixels, including titlebar offset."""
    pattern = r'# layout\s+window x ([-\d.]+)\.\.([-\d.]+) y ([-\d.]+)\.\.([-\d.]+)\s+board x ([-\d.]+)\.\.([-\d.]+) y ([-\d.]+)\.\.([-\d.]+)\s+die map x ([-\d.]+)\.\.([-\d.]+) y ([-\d.]+)\.\.([-\d.]+)'
    matches = list(re.finditer(pattern, trace))
    if not matches:
        raise ValueError('missing layout note for crop')
    nums = list(map(float, matches[-1].groups()))
    wx0, wx1, wy0, wy1 = nums[:4]
    x0, x1, y0, y1 = nums[4:8] if crop['layout'] == 'board' else nums[8:12]
    stream = probe(image)['streams'][0]
    sx, sy = stream['width'] / rect[2], stream['height'] / rect[3]
    title = rect[3] - (wy1 - wy0)
    # The batch explicitly declares a normalised rectangle inside this layout.
    left, top, width, height = crop['rect']
    x = round((x0 - wx0 + left * (x1 - x0)) * sx)
    y = round((y0 - wy0 + title + top * (y1 - y0)) * sy)
    w, h = round(width * (x1 - x0) * sx), round(height * (y1 - y0) * sy)
    if min(x, y) < 0 or min(w, h) <= 0 or x + w > stream['width'] or y + h > stream['height']:
        raise ValueError('layout crop outside captured window')
    return [x, y, w, h]


def check_source(stream, expected_size):
    if expected_size and not aspect_matches([stream['width'], stream['height']], expected_size):
        raise ValueError('captured media aspect differs from captured window')


def still(source, dest, size, crop=None, expected_size=None):
    w, h = size
    stream = probe(source)['streams'][0]
    check_source(stream, expected_size)
    if crop is None:
        if not aspect_matches(size, [stream['width'], stream['height']]):
            raise ValueError('still aspect differs from captured window; declare an explicit crop rectangle')
        filters = f'scale={w}:{h}:flags=lanczos,setsar=1'
    else:
        x, y, cw, ch = crop
        if (any(not isinstance(n, int) for n in crop) or min(x, y) < 0 or min(cw, ch) <= 0
                or x + cw > stream['width'] or y + ch > stream['height']):
            raise ValueError('explicit crop rectangle outside captured image')
        # Apply only the declared crop. Fit/pad instead of cropping again to fill.
        filters = (f'crop={cw}:{ch}:{x}:{y},scale={w}:{h}:force_original_aspect_ratio=decrease:'
                   f'flags=lanczos,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1')
    # Scale/crop once, then encode with cwebp (macOS FFmpeg often lacks libwebp).
    with tempfile.TemporaryDirectory(prefix='webp-', dir=Path(dest).parent) as temp:
        scaled = Path(temp) / 'scaled.png'
        run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-i', source,
             '-vf', filters, '-frames:v', '1', scaled])
        # Always enforce byte budget, including the formerly 2.2 MB zoom detail.
        # A low-quality result remains a candidate requiring visual review.
        for quality in (82, 76, 70, 64, 58, 50, 42, 34, 26, 18, 10):
            run(['cwebp', '-quiet', '-m', '6', '-q', str(quality), scaled, '-o', dest])
            if Path(dest).stat().st_size <= LIMIT:
                return quality
    Path(dest).unlink(missing_ok=True)
    raise ValueError(f'cannot meet WebP budget: {dest}')


def clip_profile(crf=28, preset='veryslow', fps=12):
    if (type(crf) is not int or not 0 <= crf <= 51 or type(fps) is not int or not 1 <= fps <= 60
            or preset not in ('ultrafast', 'superfast', 'veryfast', 'faster', 'fast', 'medium', 'slow', 'slower', 'veryslow')):
        raise ValueError('invalid clip encoding profile')
    return dict(crf=crf, preset=preset, fps=fps)


def clip(source, dest, seconds, trim=0.25, expected_size=None, crf=28, preset='veryslow', fps=12):
    # screencapture -V ends itself; never rely on SIGINT to finalise a MOV.
    clip_profile(crf, preset, fps)
    info = probe(source)
    check_source(info['streams'][0], expected_size)
    duration = float(info['format']['duration'])
    if duration < seconds + trim:
        raise ValueError(f'short raw clip: {duration:.3f}s, need {seconds + trim:.3f}s')
    run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-ss', str(trim), '-i', source,
         '-t', str(seconds), '-vf', f'fps={fps},scale=1100:-2:flags=lanczos,setsar=1', '-an',
         '-c:v', 'libx264', '-threads', '2', '-preset', preset, '-crf', str(crf), '-pix_fmt', 'yuv420p',
         '-movflags', '+faststart', dest])
    stream = probe(dest)['streams'][0]
    if stream['codec_name'] != 'h264' or stream['width'] != 1100:
        raise ValueError('unexpected MP4 encoding')
