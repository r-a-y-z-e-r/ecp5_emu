#!/usr/bin/env python3
"""Release-package media kit. --check/--dry-run are strictly offline."""
import argparse
import datetime
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import encode

KIT = Path(__file__).resolve().parent
ROOT = KIT.parents[1]
WINDOW_POSITION = (30, 80)
DISPLAY_SIZE = (1728, 1117)  # Known media display in logical points; no live query in --check.
spec = importlib.util.spec_from_file_location('gui_record', ROOT / 'tools/gui_record.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def positive(value):
    return math.isfinite(float(value)) and float(value) > 0


def commands(path, project=False):
    """Strict allowlist for commands used by this kit, mirrored from Rust syntax.

    trace::script only parses time/text; app::script applies chip commands.
    Project delays count from completion, chip times from design readiness.
    Keep this subset small: reject unsupported new commands instead of accepting
    a typo which the window would otherwise silently ignore.
    """
    parsed = []
    previous = 0
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        # Remove comments only outside JSON strings, matching new_project::uncomment.
        quoted = escaped = False
        end = len(raw)
        for i, char in enumerate(raw):
            if escaped:
                escaped = False
            elif char == '\\' and quoted:
                escaped = True
            elif char == '"':
                quoted = not quoted
            elif char == '#' and not quoted:
                end = i
                break
        line = raw[:end].strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if len(parts) != 2:
            raise ValueError(f'{path.name}:{number}: expected seconds and command')
        stamp, command = float(parts[0]), parts[1]
        if not math.isfinite(stamp) or stamp < 0 or (not project and stamp < previous):
            raise ValueError(f'{path.name}:{number}: invalid/nonmonotonic time')
        previous = stamp
        if project:
            patterns = [r'(?:build|emulate|save)', r'view (?:project|chip)',
                        r'open [\w./-]+(?: [1-9]\d*)?', r'source [\w./-]+ [1-9]\d* [1-9]\d*']
        else:
            patterns = [r'(?:run|pause|fit)', r'theme (?:dark|light|system)', r'dev (?:on|off)',
                        r'board (?:3d|top|bottom)', r'values (?:on|off)',
                        r'rate (?:max|real|[1-9]\d*)', r'zoom (?:[1-9]\d*(?:\.\d+)?)',
                        r'waves (?:open|close)', r'watch (?:[\w.]+(?:\[\d+\])?)',
                        r'critical (?:on|off)', r'panel (?:oled|monitor) (?:open|close)',
                        r'type (?:[^\\]|\\(?:r|n|\\|x[0-9A-Fa-f]{2}))+']
        if not project and command.startswith('new-project '):
            data = json.loads(command.partition(' ')[2])
            if (set(data) - {'dir', 'name', 'resources'} or
                    not isinstance(data.get('name'), str) or not data['name'] or
                    not isinstance(data.get('dir'), str) or Path(data['dir']).is_absolute() or
                    '..' in Path(data['dir']).parts or
                    not isinstance(data.get('resources'), list) or
                    set(data['resources']) - {'clock', 'leds', 'buttons', 'switches', 'uart'}):
                raise ValueError(f'{path.name}:{number}: invalid dialog request')
        elif not any(re.fullmatch(pattern, command) for pattern in patterns):
            raise ValueError(f'{path.name}:{number}: unsupported {"project" if project else "window"} command: {command}')
        parsed.append((stamp, command))
    if not parsed:
        raise ValueError(f'empty script: {path}')
    return parsed


def oled_budget(readme):
    text = readme.read_text()
    # README: normal picture interval 493,387 clocks; first scored F000F;
    # reference still F004B; reset >=25,000 clocks plus 37 init bytes *35.
    interval = int(re.search(r'\*\*([\d,]+) clocks\*\*', text)[1].replace(',', ''))
    first = int(re.search(r'F([0-9A-F]{4}) C[0-9A-F]+ S001', text)[1], 16)
    frame = int(re.search(r'Frame F([0-9A-F]{4}) is saved', text)[1], 16)
    if frame < first or '25,000 clocks' not in text or '37 common init bytes' not in text:
        raise ValueError('OLED timing documentation changed; review warmup calculation')
    cycles = 25000 + 37 * 35 + (frame + 1) * interval
    return dict(first_scored_frame=first, target_frame=frame, interval=interval,
                min_cycles=cycles, simulated_seconds=cycles / 25000000)


def check_geometry(entry):
    def size(value):
        return isinstance(value, list) and len(value) == 2 and all(type(n) is int and n > 0 for n in value)
    window = entry['window_size']
    if not size(window) or any(window[i] + WINDOW_POSITION[i] > DISPLAY_SIZE[i] for i in (0, 1)):
        raise ValueError(f'{entry["label"]}: window_size does not fit the media display at {WINDOW_POSITION}')
    crop = entry.get('crop')
    if crop is not None:
        if (not isinstance(crop, dict) or set(crop) != {'layout', 'rect'} or
                crop['layout'] not in ('board', 'die map') or not isinstance(crop['rect'], list) or
                len(crop['rect']) != 4 or any(type(n) not in (int, float) or not math.isfinite(n) for n in crop['rect'])):
            raise ValueError('crop requires a layout and an explicit normalised rectangle [x,y,w,h]')
        x, y, w, h = crop['rect']
        if min(x, y) < 0 or min(w, h) <= 0 or x + w > 1 or y + h > 1:
            raise ValueError('crop rectangle outside declared layout')
    if entry.get('still'):
        if not size(entry['still_size']):
            raise ValueError('invalid still_size')
        if crop is None and not encode.aspect_matches(window, entry['still_size']):
            raise ValueError('still aspect differs from requested window; declare an explicit crop rectangle')


def load(args):
    entries = runner.batch_entries(argparse.Namespace(file=KIT / 'batch.json', only=[]))
    build_timeouts()  # Validate build policy even in the strictly offline check.
    known = {e['label'] for e in entries}
    if args.shot != 'all' and args.shot not in known:
        raise ValueError(f'unknown shot {args.shot}; choose {", ".join(sorted(known))}')
    referenced = set()
    for entry in entries:
        check_geometry(entry)
        for key in ('script', 'project_script'):
            if key in entry:
                path = ROOT / entry[key]
                if not path.resolve().is_relative_to(KIT / 'shots'):
                    raise ValueError('script must live inside kit/shots')
                commands(path, project=key == 'project_script')
                referenced.add(path.resolve())
        if entry['gate']['kind'] not in ('script', 'project', 'delay', 'oled'):
            raise ValueError('unknown capture gate')
        if 'pattern' in entry['gate']:
            re.compile(entry['gate']['pattern'])
        if entry['clip'] and (not positive(entry['clip_seconds']) or
                              entry['still_at'] >= entry['clip_seconds']):
            raise ValueError('invalid clip/still timing')
        if entry['clip']:
            encode.clip_profile(**entry.get('clip_encoding', {}))
        for name in (entry['still'], entry['clip']):
            if name and Path(name).name != name:
                raise ValueError('output must be a basename')
        if entry.get('project') and (not entry['project'].startswith('examples/') or
                                     '..' in Path(entry['project']).parts):
            raise ValueError('invalid packaged project path')
    if referenced != set((KIT / 'shots').glob('*.script')):
        raise ValueError('unreferenced shot script')
    budget = oled_budget(ROOT / 'examples/gallery/oled_bricks/README.md')
    return [e for e in entries if args.shot == 'all' or e['label'] == args.shot], budget, len(referenced)


def text(path):
    return path.read_text(errors='replace') if path.exists() else ''


def uart(trace):
    chunks = re.findall(r'^# uart\t[^\t\n]+\t\d+\t([^\n]*)\n', trace, re.M)
    return bytes(''.join(chunks), 'ascii').decode('unicode_escape')


def gate_ready(entry, trace, project, budget):
    gate = entry['gate']
    if gate['kind'] == 'delay':
        return True
    if gate['kind'] != 'oled':
        return bool(re.search(gate['pattern'], project if gate['kind'] == 'project' else trace))
    data = [line.split('\t') for line in trace.splitlines() if line and not line.startswith('#')]
    if len(data) < 2 or 'cycle' not in data[0]:
        return False
    complete = [row for row in data[1:] if len(row) == len(data[0])]
    if not complete:
        return False
    cycles = int(complete[-1][data[0].index('cycle')])
    reports = re.findall(r'F[0-9A-F]{4} C[0-9A-F]+ S(\d{3}) L\d B(\d+)\r?\n', uart(trace))
    scored = bool(reports and int(reports[-1][0]) > 0 and int(reports[-1][1]) < 62)
    painted = re.findall(r'# oled picture .*? on frames (\d+)', trace)
    boards = re.findall(r'^# board[^\n]*OLED module[^\n]*', trace, re.M)
    board = bool(boards and re.search(r'OLED module asked for .*?: module drawn, screen drawn, lit pixels drawn', boards[-1]))
    return bool(cycles >= budget['min_cycles'] and scored and painted and
                int(painted[-1]) >= budget['target_frame'] + 1 and board)


def package():
    app = Path(os.environ.get('ECP5_MEDIA_APP', ROOT / 'target/dist/ECP5 Emu.app')).resolve()
    launcher = app / 'Contents/MacOS/ECP5 Emu'
    resources = app / 'Contents/Resources'
    for p in (app / 'Contents/Info.plist', launcher, resources / 'toolchain/environment.sh',
              resources / 'ulx3s_v3.0.8.glb', app / 'Contents/MacOS/trellis_image_reader',
              resources / 'examples/gallery/oled_bricks/README.md',
              resources / 'bin/ecp5_emu'):
        if not p.is_file():
            raise ValueError(f'missing final package input: {p}')
    if not os.access(launcher, os.X_OK):
        raise ValueError('package launcher is not executable')
    return app, launcher, resources


def window_info(helper, pid):
    return json.loads(subprocess.check_output([str(helper), str(pid)], timeout=5))


def safe_window(info, expected=None):
    if info.get('asleep') or not info.get('rect') or not info.get('onscreen') or info.get('covered'):
        raise ValueError(f'window absent, minimised, offscreen, asleep or covered: {info}')
    if expected is not None and info['rect'] != expected:
        raise ValueError('window moved/resized during capture')
    return info['rect']


def requested_window(info, size):
    rect = info.get('rect')
    if rect and rect[2:] != size:
        raise ValueError(f'window resized by the OS: asked {size[0]}x{size[1]}, got {rect[2]}x{rect[3]}')
    return safe_window(info)


def position(pid, size):
    # Only numeric values enter AppleScript, never paths or project names.
    code = f'''tell application "System Events" to tell (first process whose unix id is {pid})
set frontmost to true
set value of attribute "AXMinimized" of window 1 to false
set position of window 1 to {{{WINDOW_POSITION[0]}, {WINDOW_POSITION[1]}}}
set size of window 1 to {{{size[0]}, {size[1]}}}
end tell'''
    subprocess.run(['osascript', '-e', code], check=True, capture_output=True, timeout=10)


def check_logs(log, trace, project):
    errors = re.search(r'(?im)^.*(?:panicked at|^error:|script:|# script error|project-script failed|chip error|load failed|build failed|open failed|project error).*$', log + '\n' + trace + '\n' + project)
    if errors:
        raise ValueError(f'window/script failure: {errors[0]}')


def app_environment(home):
    env = {k: v for k, v in os.environ.items() if not k.startswith('ECP5_')}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(home / '.config'))
    home.mkdir(parents=True, exist_ok=True)
    prefs = home / 'Library/Application Support/ecp5_emu/recent.txt'
    prefs.parent.mkdir(parents=True, exist_ok=True)
    prefs.write_text('theme=dark\ndev_mode=false\n')
    return env


def design_name(design):
    if not re.fullmatch(r'[A-Za-z0-9_-]+/[A-Za-z0-9_-]+', design):
        raise ValueError(f'design must be GROUP/NAME, got {design!r}')
    return design


def build_timeouts():
    values = json.loads((KIT / 'batch.json').read_text()).get('build_timeouts', {})
    if not isinstance(values, dict):
        raise ValueError('build_timeouts must be an object keyed by GROUP/NAME')
    for design, seconds in values.items():
        design_name(design)
        if not isinstance(seconds, (int, float)) or isinstance(seconds, bool) or not positive(seconds):
            raise ValueError(f'invalid build timeout for {design}')
    return values


def neutral_folder():
    supplied = os.environ.get('ECP5_MEDIA_NEUTRAL')
    base = Path(supplied).resolve() if supplied else Path('/private/tmp/ecp5-media')
    if base == Path('/private/tmp') or not base.is_relative_to(Path('/private/tmp')):
        raise ValueError('neutral folder must resolve inside private temporary storage')
    base.mkdir(parents=True, exist_ok=True)
    return base if supplied else Path(tempfile.mkdtemp(prefix='take-', dir=base))


def refresh_examples(resources, neutral):
    """Mirror packaged sources and relative symlinks, retaining every build/ tree.

    Prune removed source files too, but retain obsolete project directories when
    they contain a build cache. Never traverse a destination symlink on refresh.
    """
    def prune(path):
        if path.name == 'build':
            return
        if path.is_symlink() or not path.is_dir():
            path.unlink(missing_ok=True)
        else:
            for child in path.iterdir():
                prune(child)
            if not any(path.iterdir()):
                path.rmdir()

    def copy(source, dest):
        if source.is_symlink() or not source.is_dir():
            prune(dest)
            if dest.exists() or dest.is_symlink():
                raise ValueError(f'source refresh would replace a directory containing build/: {dest}')
            if source.is_symlink():
                target = os.readlink(source)
                if Path(target).is_absolute() or not source.resolve().is_relative_to((resources / 'examples').resolve()):
                    raise ValueError(f'packaged example link escapes examples: {source}')
                dest.symlink_to(target)
            else:
                shutil.copy2(source, dest)
            return
        if dest.is_symlink() or (dest.exists() and not dest.is_dir()):
            dest.unlink()
        dest.mkdir(parents=True, exist_ok=True)
        names = {p.name for p in source.iterdir()}
        for child in dest.iterdir():
            if child.name not in names:
                prune(child)
        for child in source.iterdir():
            if child.name != 'build':
                copy(child, dest / child.name)

    source = resources / 'examples'
    if not source.is_dir():
        raise ValueError('packaged examples directory missing')
    copy(source, neutral / 'examples')


def build_steps(log):
    matches = re.findall(r'^\[\d+/\d+\] (.+?) … (skipped: inputs unchanged|done in [\d.]+ s|failed)\s*$', log, re.M)
    return [{'step': step, 'status': 'skipped' if result.startswith('skipped:') else
             'failed' if result == 'failed' else 'ran', 'detail': result}
            for step, result in matches]


def build_designs(designs, resources, neutral, take):
    # Always invoke the package's flow: it alone decides whether cache keys match.
    env = app_environment(neutral / 'build-home')
    timeouts = build_timeouts()
    reports = []
    for design in designs:
        design_name(design)
        project = neutral / 'examples' / design
        if not (project / 'ecp5.toml').is_file():
            raise ValueError(f'project absent from final package: {design}')
        seconds = timeouts.get(design, 600)
        log_path = take / f'{design.replace("/", "-")}.build.log'
        print(f'Preparing {design} with packaged CLI (no window; timeout {seconds:g}s)...', flush=True)
        with log_path.open('w') as log:
            result = subprocess.run([str(resources / 'bin/ecp5_emu'), 'build', str(project)],
                                    cwd=neutral, env=env, stdout=log, stderr=log, timeout=seconds)
        steps = build_steps(text(log_path))
        for step in steps:
            print(f'{design}: {step["step"]}: {step["status"]} ({step["detail"]})', flush=True)
        if result.returncode:
            raise ValueError(f'{design}: build failed (exit {result.returncode}); see {log_path}')
        if not (project / 'build/top.config').is_file():
            raise ValueError(f'package build did not produce top.config: {design}')
        if not steps:
            raise ValueError(f'{design}: no flow step statuses found; see {log_path}')
        status = 'skipped' if all(step['status'] == 'skipped' for step in steps) else 'ran'
        report = dict(design=design, status=status, steps=steps, timeout=seconds, log=log_path.name)
        reports.append(report)
        with log_path.open('a') as log:
            log.write(f'\n# site_media: {design}: {status}\n')
        (take / 'builds.json').write_text(json.dumps(reports, indent=2) + '\n')
        print(f'{design}: {status}; log: {log_path}', flush=True)
    return reports


def prepare_projects(entries, resources, neutral, take):
    designs = sorted({e['project'].removeprefix('examples/') for e in entries
                      if e.get('project') and e['label'] != 'project'})
    return build_designs(designs, resources, neutral, take)


def capture(entry, launcher, neutral, take, helper, budget):
    label = entry['label']
    work = neutral / 'sessions' / take.name / label
    work.mkdir(parents=True)
    env = app_environment(work / 'home')
    trace_path = take / f'{label}.tsv'
    project_path = take / f'{label}.project.tsv'
    log_path = take / f'{label}.out'
    env.update(ECP5_GUI_TRACE=str(trace_path), ECP5_PROJECT_TRACE=str(project_path), ECP5_LOAD_TIMES='1')
    for key, var in [('script', 'ECP5_GUI_SCRIPT'), ('project_script', 'ECP5_PROJECT_SCRIPT')]:
        if key in entry:
            script = work / Path(entry[key]).name
            shutil.copy2(ROOT / entry[key], script)
            env[var] = str(script)
    command = [str(launcher)]
    if entry.get('project'):
        command += ['--project', str(neutral / entry['project'])]
        if not entry.get('project_script'):
            command += ['--run']
    command += entry.get('args', [])
    proc = cap = None
    raw = take / f'{label}.raw.mov'
    image = take / f'{label}.raw.png'
    deadline = time.monotonic() + entry['timeout']
    with log_path.open('w') as log, (take / f'{label}.coverage.jsonl').open('w') as coverage:
        try:
            proc = subprocess.Popen(command, cwd=work, env=env, stdout=log, stderr=log)

            def poll(expected=None, require_window=True):
                if time.monotonic() >= deadline:
                    raise ValueError(f'{label}: timeout waiting for readiness/capture')
                if proc.poll() is not None:
                    raise ValueError(f'{label}: window exited early ({proc.returncode})')
                info = window_info(helper, proc.pid)
                coverage.write(json.dumps(info) + '\n')
                coverage.flush()
                if require_window:
                    safe_window(info, expected)
                trace, project = text(trace_path), text(project_path)
                check_logs(text(log_path), trace, project)
                return info, trace, project

            # Startup may have no window yet. Never capture until it is present.
            while True:
                info, _, _ = poll(require_window=False)
                if info.get('asleep'):
                    raise ValueError('display slept during startup')
                if info.get('rect'):
                    break
                time.sleep(.2)
            position(proc.pid, entry['window_size'])
            time.sleep(.5)
            info, _, _ = poll(require_window=False)
            rect = requested_window(info, entry['window_size'])
            while True:
                _, trace, project = poll(rect)
                if gate_ready(entry, trace, project, budget):
                    break
                time.sleep(.2)
            safe_window(window_info(helper, proc.pid), rect)
            region = ','.join(map(str, rect))
            started = time.monotonic()
            if entry['clip']:
                # Extra second is trimmed; the app remains alive beyond -V.
                cap = subprocess.Popen(['screencapture', '-x', '-v', f'-V{entry["clip_seconds"] + 1}',
                                        f'-R{region}', str(raw)], stdout=log, stderr=log)
            captured = not bool(entry['still'])
            while True:
                _, trace, project = poll(rect)
                elapsed = time.monotonic() - started
                if not captured and elapsed >= entry['still_at']:
                    if label == 'project' and 'build ok' not in project:
                        # A 28s clip must actually include build completion.
                        if cap is not None and cap.poll() is not None:
                            raise ValueError('build exceeded clip; increase clip_seconds in batch.json')
                    else:
                        if label == 'oled' and not gate_ready(entry, trace, project, budget):
                            raise ValueError('OLED scored/painted state missing at still')
                        runner.console_unlocked()
                        subprocess.run(['screencapture', '-x', f'-R{region}', str(image)], check=True)
                        poll(rect)
                        # Save layout at the still, rather than one changed after it.
                        (take / f'{label}.still-trace.txt').write_text(trace)
                        captured = True
                if cap is not None and cap.poll() is not None and cap.returncode:
                    raise ValueError('screencapture failed')
                if captured and (cap is None or cap.poll() is not None):
                    break
                time.sleep(.2)
            if label == 'project' and 'build ok' not in project:
                raise ValueError('build completion missing from project clip')
            # The capture has ENDED while the original unobscured window still exists.
            poll(rect)
            runner.console_unlocked()
        except BaseException:
            # SIGINT does not stop screencapture. Discard partial video on every abort.
            if cap is not None and cap.poll() is None:
                cap.kill()
                cap.wait()
            raw.unlink(missing_ok=True)
            image.unlink(missing_ok=True)
            raise
        finally:
            # Always reap capture before closing the window, including interruptions.
            if cap is not None and cap.poll() is None:
                cap.kill()
                cap.wait()
                raw.unlink(missing_ok=True)
            runner.stop(proc)
    if entry['clip']:
        encode.clip(raw, take / entry['clip'], entry['clip_seconds'], expected_size=rect[2:],
                    **entry.get('clip_encoding', {}))
    quality = None
    if entry['still']:
        crop = encode.crop_layout(text(take / f'{label}.still-trace.txt'), entry['crop'], image, rect) if entry.get('crop') else None
        quality = encode.still(image, take / entry['still'], entry['still_size'], crop, expected_size=rect[2:])
    print(f'{label}: encoded candidates (WebP quality {quality}); visual/path review required', flush=True)
    return {'shot': label, 'webp_quality': quality, 'rect': rect}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('shot', nargs='?', default='all')
    parser.add_argument('designs', nargs='*', help='GROUP/NAME designs for prebuild mode')
    parser.add_argument('--check', '--dry-run', dest='check', action='store_true',
                        help='validate every script; no app, capture, display query or lock')
    args = parser.parse_args()
    try:
        prebuild = args.shot == 'prebuild'
        if prebuild:
            if not args.designs:
                raise ValueError('prebuild needs at least one GROUP/NAME design')
            designs = list(dict.fromkeys(design_name(design) for design in args.designs))
        elif args.designs:
            raise ValueError('extra arguments are only supported by prebuild DESIGN...')
        entries, budget, count = load(argparse.Namespace(shot='all') if prebuild else args)
        if args.check:
            print(f'PASS: 12 shots, {count} scripts validated against the supported Rust command subset')
            if prebuild:
                timeouts = build_timeouts()
                for design in designs:
                    print(f'PLAN prebuild {design}: packaged CLI; timeout {timeouts.get(design, 600):g}s')
                print('No window, screen capture, display query, package build or screen lock performed.')
                return
            print(f'OLED: F{budget["first_scored_frame"]:04X} first score; F{budget["target_frame"]:04X} target, '
                  f'>={budget["min_cycles"]:,} cycles ({budget["simulated_seconds"]:.6f} simulated s), plus UART/OLED/3D evidence')
            for entry in entries:
                print(f'PLAN {entry["label"]}: packaged {entry.get("project", "start screen")} -> '
                      f'out/site_media/TAKE/{entry["still"] or entry["clip"]}')
            print('No window, screen capture, display query, package build or screen lock performed.')
            return
        if prebuild and not os.environ.get('ECP5_MEDIA_NEUTRAL'):
            raise ValueError('prebuild requires ECP5_MEDIA_NEUTRAL to name a reusable neutral folder')
        if sys.platform != 'darwin':
            raise ValueError('the release app package requires macOS')
        app, launcher, resources = package()
        neutral = neutral_folder()
        refresh_examples(resources, neutral)
        output = ROOT / 'out/site_media'
        output.mkdir(parents=True, exist_ok=True)
        stamp = datetime.datetime.now().strftime('%Y%m%d-%H%M%S')
        take = Path(tempfile.mkdtemp(prefix=f'{stamp}-{neutral.name}-', dir=output))
        print(f'Neutral sources/build cache: {neutral}; logs: {take}', flush=True)
        if prebuild:
            build_designs(designs, resources, neutral, take)
            print(f'Prebuild complete; retained neutral folder: {neutral}. No window or screen lock used.')
            return
        budget = oled_budget(resources / 'examples/gallery/oled_bricks/README.md')
        for program in ('swiftc', 'ffmpeg', 'ffprobe', 'cwebp', 'osascript', 'screencapture'):
            if not shutil.which(program):
                raise ValueError(f'missing capture dependency: {program}')
        if 'libx264' not in subprocess.check_output(['ffmpeg', '-hide_banner', '-encoders'], text=True):
            raise ValueError('FFmpeg must provide libx264')
        for entry in entries:
            if entry.get('project') and not (neutral / entry['project'] / 'ecp5.toml').is_file():
                raise ValueError(f'project absent from final package: {entry["project"]}')
        prepare_projects(entries, resources, neutral, take)
        helper = take / 'window_info'
        subprocess.run(['swiftc', str(KIT / 'window_info.swift'), '-o', str(helper)], check=True)
        metadata = {'app_version': plistlib.loads((app / 'Contents/Info.plist').read_bytes()).get('CFBundleShortVersionString'),
                    'gui_sha256': hashlib.sha256((app / 'Contents/MacOS/ecp5_gui').read_bytes()).hexdigest(),
                    'neutral_folder': str(neutral), 'oled_budget': budget, 'candidates': []}
        (take / 'provenance.json').write_text(json.dumps(metadata, indent=2) + '\n')
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, runner.interrupted)
        runner.console_unlocked()
        if window_info(helper, 0).get('asleep'):
            raise ValueError('display must be awake')
        # Use existing common-checkout lock, inode ownership and signal cleanup.
        # record_run would force development reader/model paths and has no pixel hook.
        with runner.screen_lock('site-media', os.environ.get('ECP5_SESSION', 'site-media')):
            for entry in entries:
                print(f'Capturing {entry["label"]} from final package...', flush=True)
                metadata['candidates'].append(capture(entry, launcher, neutral, take, helper, budget))
                (take / 'provenance.json').write_text(json.dumps(metadata, indent=2) + '\n')
        print(f'Candidates: {take.relative_to(ROOT)}; retain neutral sources for retakes. No files published.')
    except (OSError, ValueError, subprocess.SubprocessError, KeyError, IndexError) as error:
        parser.exit(1, f'site_media: {error}\n')


if __name__ == '__main__':
    main()
