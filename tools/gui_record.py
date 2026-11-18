#!/usr/bin/env python3
"""Runs the emulator's window with a timed script, and sums its drawing records up.

Usage: tools/gui_record.py run DESIGN SCRIPT LABEL [--who NAME] [--binary PATH] [--env K=V ...]
       tools/gui_record.py run picorv32_console tools/gui_scripts/waves_repro.script waves --start-screen
       tools/gui_record.py run --project DIR --project-script FILE SCRIPT LABEL
       tools/gui_record.py batch tools/gui_scripts/v010/batch.json [--dry-run | --check]
       tools/gui_record.py sum RECORD [RECORD ...]

DESIGN is a top.config path or a name under target/reference. Records and window
output go to out/records/LABEL.tsv and LABEL.out. The display must be awake and
unlocked, and the main checkout's out/screen.lock must be free. --timeout defaults
to 300 seconds. A stalled record gets up to three macOS process samples, once
frames arrive after each design load and no load: or fast mode: line has arrived
in the last three seconds.
The screen lock's holder is --who NAME, else ECP5_SESSION, else LABEL; its line is
<who> <date> <label>.

Summaries split at script commands and changes of running, rate or values. Two
records are compared by command occurrence and state, ignoring timestamp jitter.
Seconds are the sum of frame_ms / 1000, so a single frame has a useful duration
and a design's reset of t does not subtract time. Missed counts include the change
from the preceding phase's last frame. Lag is sum(sum) / sum(n), across all five
strips and frames; its maximum is the greatest observed maximum. Percentiles use
linear interpolation. Notes are counted by the word after #, including waves.
Older records may omit trailing diagnostic columns; their end values show n/a.
See gui/src/trace.rs for the record and script formats.
"""
import argparse
from collections import Counter
from contextlib import contextmanager
import datetime
import json
import math
import os
import pathlib
import plistlib
import re
import signal
import shlex
import statistics
import subprocess
import time
from contextlib import nullcontext

ROOT = pathlib.Path(__file__).resolve().parents[1]
RECORDS = ROOT / "out/records"


def percentile(values, share):
    values = sorted(values)
    pos = (len(values) - 1) * share
    lo = int(pos)
    return values[lo] + (values[math.ceil(pos)] - values[lo]) * (pos - lo)


def note_counts(line):
    parts = line.split()
    kind = parts[1] if len(parts) > 1 else "unknown"
    wrong = re.search(r"\bWRONG\s+(\d+)\b", line) if kind == "board" else None
    return Counter({kind: 1}), int(wrong[1]) if wrong else 0


def read_record(path):
    """Keep command boundaries even when the command leaves the state unchanged."""
    header, phases, current, previous = None, [], None, None
    notes, pending = Counter(), Counter()
    wrong, pending_wrong, command_number = 0, 0, 0
    command = "start"
    occurrences = Counter()
    required = {"t", "frame_ms", "running", "rate", "values", "mhz", "lag",
                "lit_missed", "value_missed"}
    with path.open() as source:
        for number, line in enumerate(source, 1):
            line = line.strip()
            if not line:
                continue
            if line.startswith("#"):
                counted, bad = note_counts(line)
                notes.update(counted)
                wrong += bad
                if "script" in counted:
                    parts = line.split(None, 3)
                    if len(parts) != 4:
                        raise ValueError(f"{path}:{number}: expected # script TIME COMMAND")
                    float(parts[2])
                    command_number += 1
                    command = parts[3]
                    current = None
                if current is None:
                    pending.update(counted)
                    pending_wrong += bad
                else:
                    current["notes"].update(counted)
                    current["wrong"] += bad
                continue
            if header is None:
                header = line.split("\t")
                missing = required - set(header)
                if missing:
                    raise ValueError(f"{path}: missing columns: {', '.join(sorted(missing))}")
                continue
            fields = line.split("\t")
            if len(fields) != len(header):
                raise ValueError(f"{path}:{number}: expected {len(header)} columns, got {len(fields)}")
            frame = dict(zip(header, fields))
            state = tuple(frame[k] for k in ("running", "rate", "values"))
            reset = previous is not None and float(frame["t"]) < float(previous["t"])
            if current is None or current["state"] != state or reset:
                base = (command_number, command, state)
                occurrences[base] += 1
                current = {"key": (*base, occurrences[base]), "state": state,
                           "frames": [], "seconds": 0.0, "missed": [0, 0],
                           "notes": pending, "wrong": pending_wrong}
                pending, pending_wrong = Counter(), 0
                phases.append(current)
            current["frames"].append(frame)
            current["seconds"] += float(frame["frame_ms"]) / 1000
            for i, column in enumerate(("lit_missed", "value_missed")):
                before = int(previous[column]) if previous else 0
                now = int(frame[column])
                # A counter reset starts a new total rather than a negative delta.
                current["missed"][i] += now - before if now >= before else now
            previous = frame
    if header is None:
        raise ValueError(f"{path}: no header")
    return {"path": path, "phases": phases, "notes": notes, "wrong": wrong, "last": previous}


def counts_text(notes):
    return ",".join(f"{kind}={count}" for kind, count in sorted(notes.items())) or "none"


def phase_text(phase):
    frames = phase["frames"]
    ms = [float(f["frame_ms"]) for f in frames]
    mhz = [float(f["mhz"]) for f in frames if f["mhz"] not in ("", "-")]
    lag_n, lag_sum, lag_max = 0, 0.0, 0.0
    for frame in frames:
        strips = frame["lag"].split(",")
        if len(strips) != 5:
            raise ValueError("lag must contain five n/sum/max triples")
        for strip in strips:
            n, total, largest = strip.split("/")
            if int(n):
                lag_n += int(n)
                lag_sum += float(total)
                lag_max = max(lag_max, float(largest))
    seconds = phase["seconds"]
    fps = f"{len(frames) / seconds:.2f}" if seconds else "-"
    lag = f"{lag_sum / lag_n:.2f}/{lag_max:.2f}" if lag_n else "-/-"
    khz = f"{statistics.median(mhz) * 1000:.2f}" if mhz else "-"
    return (f"{len(frames)} frames {seconds:.3f}s {fps}fps; "
            f"frame_ms p50/p90/max={percentile(ms, .5):.2f}/{percentile(ms, .9):.2f}/{max(ms):.2f}; "
            f"lag mean/max={lag}ms n={lag_n}; "
            f"missed lit/value=+{phase['missed'][0]}/+{phase['missed'][1]}; {khz}kHz; "
            f"board WRONG={phase['wrong']}; notes {counts_text(phase['notes'])}")


def phase_label(key):
    number, command, (running, rate, values), occurrence = key
    return f"{number}:{command} running={running} rate={rate} values={values} #{occurrence}"


def end_text(record):
    last = record["last"]
    if last:
        optional = {column: last.get(column, "n/a")
                    for column in ("valued_wires", "value_seen_wires", "dropped",
                                   "value_unseen", "map_h", "net")}
        valued, seen = optional["valued_wires"], optional["value_seen_wires"]
        if valued == "n/a" or seen == "n/a":
            ratio = "n/a"
        else:
            ratio = f"{int(seen) / int(valued):.1%}" if int(valued) else "-"
        coverage = (f"valued_wires={valued} value_seen_wires={seen} seen/valued={ratio}; "
                    + " ".join(f"{column}={optional[column]}"
                               for column in ("dropped", "value_unseen", "map_h", "net")))
    else:
        coverage = "no frames"
    return f"{coverage}; board WRONG={record['wrong']}; notes {counts_text(record['notes'])}"


def summarize(paths):
    projects = [path for path in paths if is_project_record(path)]
    for path in projects:
        events = project_events(path)
        print(f"project: {path}; {len(events)} events; notes {counts_text(Counter(e.lstrip('# ').split()[0] for e in events))}")
        for event in events:
            if event.startswith(("open ", "# editor frames", "chip error", "load failed", "# script error")):
                print(f"  {event}")
    records = [read_record(path) for path in paths if path not in projects]
    if len(records) == 2:
        a, b = records
        print(f"A: {a['path']}\nB: {b['path']}")
        left = {p["key"]: p for p in a["phases"]}
        right = {p["key"]: p for p in b["phases"]}
        for key in dict.fromkeys([*left, *right]):
            av = phase_text(left[key]) if key in left else "missing"
            bv = phase_text(right[key]) if key in right else "missing"
            print(f"{phase_label(key)} | A: {av} | B: {bv}")
        print(f"end | A: {end_text(a)} | B: {end_text(b)}")
    else:
        for record in records:
            print(f"record: {record['path']}")
            for phase in record["phases"]:
                print(f"{phase_label(phase['key'])}: {phase_text(phase)}")
            print(f"end: {end_text(record)}")


def console_unlocked():
    try:
        result = subprocess.run(["ioreg", "-n", "Root", "-d1", "-a"],
                                capture_output=True, check=True, timeout=10)
        registry = plistlib.loads(result.stdout)
    except (OSError, subprocess.SubprocessError, plistlib.InvalidFileException) as error:
        raise ValueError(f"cannot check whether the display is awake and unlocked with ioreg: {error}") from None
    locks = []

    def visit(value):
        if isinstance(value, dict):
            if "IOConsoleLocked" in value:
                locks.append(value["IOConsoleLocked"])
            for child in value.values():
                visit(child)
        elif isinstance(value, list):
            for child in value:
                visit(child)

    visit(registry)
    if not locks or any(value is not False for value in locks):
        raise ValueError("display must be awake and unlocked: ioreg must report IOConsoleLocked false")


def screen_lock_path():
    result = subprocess.run(["git", "rev-parse", "--git-common-dir"], cwd=ROOT,
                            capture_output=True, text=True, check=True)
    common = pathlib.Path(result.stdout.strip())
    if not common.is_absolute():
        common = ROOT / common
    return common.resolve().parent / "out/screen.lock"


@contextmanager
def screen_lock(label, who=None):
    who = who or os.environ.get("ECP5_SESSION") or label
    lock = screen_lock_path()
    lock.parent.mkdir(parents=True, exist_ok=True)
    identity = None
    created = False
    try:
        try:
            fd = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        except FileExistsError:
            content = lock.read_text(errors="replace")
            raise ValueError(f"screen lock already exists: {lock}\n{content}") from None
        created = True
        with os.fdopen(fd, "w") as output:
            identity = os.fstat(output.fileno())
            date = datetime.datetime.now().astimezone().isoformat(timespec="seconds")
            output.write(f"{who} {date} {label}\n")
        yield
    finally:
        if created and identity is not None:
            try:
                current = lock.stat()
                if (current.st_dev, current.st_ino) == (identity.st_dev, identity.st_ino):
                    lock.unlink()
            except FileNotFoundError:
                pass


def stop(process):
    if process is not None and process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def front_app():
    try:
        front = subprocess.run(["lsappinfo", "front"], capture_output=True,
                               text=True, check=True, timeout=2).stdout.strip()
        return subprocess.run(["lsappinfo", "info", "-only", "name", front],
                              capture_output=True, text=True, check=True,
                              timeout=2).stdout.strip()
    except (OSError, subprocess.SubprocessError) as error:
        return f"unavailable ({error})"


class StallWatch:
    """Read only new lines, and keep loading from spending the sample budget."""

    def __init__(self, trace, output, now, project=None):
        self.trace, self.output, self.project = trace, output, project
        self.project_ready, self.project_chip, self.project_building = False, False, False
        self.positions, self.pending = {}, {}
        self.ready, self.awaiting_design = False, False
        self.last_growth = self.last_sample = now
        self.last_load = float("-inf")

    def read_lines(self, path):
        position = self.positions.get(path, 0)
        try:
            with path.open("rb") as source:
                if os.fstat(source.fileno()).st_size < position:
                    position = 0
                    self.pending.pop(path, None)
                source.seek(position)
                data = source.read()
                self.positions[path] = source.tell()
        except FileNotFoundError:
            return [], False
        lines = (self.pending.get(path, b"") + data).split(b"\n")
        self.pending[path] = lines.pop()
        return [line.decode("utf-8", errors="replace") for line in lines], bool(data)

    def poll(self, now):
        lines, grew = self.read_lines(self.trace)
        if grew:
            self.last_growth = now
        for line in lines:
            parts = line.split(None, 3)
            if parts[:2] == ["#", "design"]:
                self.ready, self.awaiting_design = False, False
            elif (parts[:2] == ["#", "script"] and len(parts) == 4
                  and parts[3].split()[0] in ("open", "close")):
                # An open can record an empty start-screen frame before # design.
                self.ready, self.awaiting_design = False, True
            elif not line.startswith("#") and "\t" in line:
                try:
                    frame_time = float(line.split("\t", 1)[0])
                except ValueError:
                    continue
                if math.isfinite(frame_time) and not self.awaiting_design:
                    self.ready = True
        lines, _ = self.read_lines(self.output)
        if any(line.startswith(("load:", "fast mode:")) for line in lines):
            self.last_load = now
        if self.project is not None:
            events, _ = self.read_lines(self.project)
            for event in events:
                event = event.partition("\t")[2]
                if event.startswith(("emulate ", "chip error")):
                    self.project_ready = False
                elif event.startswith("chip ready"):
                    self.project_ready = True
                elif event == "view chip":
                    self.project_chip = True
                elif event == "view project":
                    self.project_chip = False
                elif event.startswith("build start"):
                    self.project_building = True
                elif event.startswith(("build ok", "build failed", "build cancelled")):
                    self.project_building = False
            if not self.project_ready or not self.project_chip or self.project_building:
                self.last_growth = now
                return False
        return self.ready and now - max(self.last_growth, self.last_load, self.last_sample) >= 3

    def sampled(self, now):
        self.last_sample = now


def prepare_run(args, validate=True):
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", args.label):
        raise ValueError("LABEL must contain only letters, digits, _, - or . and start with a letter or digit")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        raise ValueError("--timeout must be a positive number of seconds")
    project = getattr(args, "project", None)
    design = None
    if not project:
        design = pathlib.Path(args.design)
        if design.suffix != ".config":
            design = ROOT / "target/reference" / design / "top.config"
        design = design.resolve()
    else:
        project = project.resolve()
        if validate and not (project / "ecp5.toml").is_file():
            raise ValueError(f"project has no ecp5.toml: {project}")
    binary = args.binary.resolve()
    reader = (ROOT / "target/trellis_image_reader").resolve()
    board = (ROOT / "out/ulx3s_3d/ulx3s_v3.0.8.glb").resolve()
    script = args.script.resolve() if args.script else None
    required = [("GUI binary", binary), ("image reader", reader), ("board model", board)]
    if script:
        required.append(("script", script))
    if design:
        required.append(("design top.config", design))
    if getattr(args, "project_script", None):
        required.append(("project script", args.project_script.resolve()))
    for name, path in required:
        if validate and not path.is_file():
            raise ValueError(f"{name} does not exist: {path}")
    if validate and not os.access(binary, os.X_OK):
        raise ValueError(f"GUI binary is not executable: {binary}")
    env = dict(os.environ)
    for key in getattr(args, "unset_env", []):
        env.pop(key, None)
    for group in args.env:
        for pair in group:
            key, equal, value = pair.partition("=")
            if not equal or not key or "\0" in pair:
                raise ValueError(f"--env expects K=V, got {pair!r}")
            env[key] = value
    trace = RECORDS / f"{args.label}.tsv"
    output = RECORDS / f"{args.label}.out"
    # These paths belong to this run, including when --env supplies other options.
    env.update(ECP5_IMAGE_READER=str(reader), ECP5_BOARD_MODEL=str(board),
               ECP5_LOAD_TIMES="1", ECP5_GUI_TRACE=str(trace))
    if script:
        env["ECP5_GUI_SCRIPT"] = str(script)
    else:
        env.pop("ECP5_GUI_SCRIPT", None)
    project_trace = RECORDS / f"{args.label}.project.tsv" if project else None
    if project:
        env["ECP5_PROJECT_TRACE"] = str(project_trace)
        if args.project_script:
            env["ECP5_PROJECT_SCRIPT"] = str(args.project_script.resolve())
        else:
            env.pop("ECP5_PROJECT_SCRIPT", None)
    command = [str(binary)] + (["--project", str(project)] if project else
                              ([] if args.start_screen else [str(design)]))
    return command, env, trace, output, project_trace


def record_run(args, lock_held=False):
    command, env, trace, output, project_trace = prepare_run(args)
    console_unlocked()
    process, samples = None, []
    with nullcontext() if lock_held else screen_lock(args.label, args.who):
        RECORDS.mkdir(parents=True, exist_ok=True)
        trace.write_text("")
        if project_trace:
            project_trace.write_text("")
        with output.open("w", buffering=1) as log:
            try:
                process = subprocess.Popen(command, cwd=ROOT, env=env, stdout=log, stderr=log)
                started = time.monotonic()
                watch = StallWatch(trace, output, started, project_trace)
                stalls = 0
                print(f"record: {trace}\nwindow output: {output}", flush=True)
                while process.poll() is None:
                    now = time.monotonic()
                    if now - started >= args.timeout:
                        log.write(f"# timeout after {args.timeout:g} s; terminating PID {process.pid}\n")
                        stop(process)
                        raise ValueError(f"window timed out after {args.timeout:g} s; see {output}")
                    if stalls < 3 and watch.poll(now):
                        stalls += 1
                        sample = RECORDS / f"{args.label}_stall{stalls}.sample"
                        log.write(f"# stall {stalls} at {now - started:.1f}s; front app: {front_app()}; sample: {sample}\n")
                        try:
                            samples.append(subprocess.Popen(
                                ["sample", str(process.pid), "2", "-mayDie", "-file", str(sample)],
                                stdout=log, stderr=log))
                        except OSError as error:
                            log.write(f"# sample failed: {error}\n")
                        watch.sampled(time.monotonic())
                    try:
                        process.wait(timeout=.25)
                    except subprocess.TimeoutExpired:
                        pass
                if process.returncode:
                    raise ValueError(f"window exited with status {process.returncode}; see {output}")
            finally:
                stop(process)
                if process is not None and process.returncode is not None:
                    log.write(f"# window exit {process.returncode}\n")
                for sample in samples:
                    stop(sample)
    summarize([trace] + ([project_trace] if project_trace else []))


def is_project_record(path):
    if path.name.endswith(".project.tsv"):
        return True
    for line in path.read_text().splitlines():
        if line and not line.startswith("#"):
            fields = line.split("\t")
            return len(fields) == 2 and re.fullmatch(r"\d+(?:\.\d+)?", fields[0]) is not None
    return False


def project_events(path):
    events = []
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if not line:
            continue
        stamp, tab, event = line.partition("\t")
        if not tab or not event or not math.isfinite(float(stamp)) or float(stamp) < 0:
            raise ValueError(f"{path}:{number}: invalid project event")
        events.append(event)
    return events


def record_bundle(entry):
    trace = RECORDS / f"{entry['label']}.tsv"
    window = trace.read_text()
    if not window.strip():
        raise ValueError(f"empty window record: {trace}")
    # Parse frames as well as notes: malformed or truncated records cannot pass.
    read_record(trace)
    project = ""
    if entry.get("project"):
        project = "\n".join(project_events(RECORDS / f"{entry['label']}.project.tsv"))
        if not project:
            raise ValueError("empty project record")
    output = (RECORDS / f"{entry['label']}.out").read_text()
    return {"window": window, "project": project, "output": output}


def fields(note):
    return dict(re.findall(r"(\w+)=(.*?)(?=\s+\w+=|$)", note))


def layout_verdict(bundle, criterion):
    errors, centres, widths, gaps = [], [], set(), set()
    closed = opened = legacy = 0
    current_map = None
    for line in bundle["window"].splitlines():
        if line.startswith("# map-layout "):
            note = fields(line)
            rect = lambda key: tuple(map(float, note[key].split(",")))
            central, mapped, centre, cell = rect("central"), rect("map"), rect("centre"), float(note["cell"])
            if len(central) != 4 or len(mapped) != 4 or len(centre) != 2:
                return False, "invalid rectangle/camera shape"
            if not all(math.isfinite(n) for n in (*central, *mapped, *centre, cell)) or cell <= 0:
                return False, "invalid camera/rectangle number"
            errors.extend(abs(mapped[i] - central[i]) for i in (0, 1, 3))
            if note["report"] == "off":
                closed += 1
                widths.add((mapped[2] - mapped[0], central[2] - central[0]))
                errors.append(abs(mapped[2] - central[2]))
                if note["shelf"] != "-":
                    return False, "closed report reserves a shelf"
            elif note["report"] == "on":
                opened += 1
                shelf = rect("shelf")
                if len(shelf) != 4:
                    return False, "invalid shelf rectangle"
                gaps.add(shelf[0] - mapped[2])
                errors.extend((abs(shelf[0] - mapped[2] - 8), abs(shelf[2] - central[2]),
                               abs(shelf[1] - central[1]), abs(shelf[3] - central[3])))
            else:
                return False, "invalid report state"
            before, before_cell = rect("before_centre"), float(note["before_cell"])
            if len(before) != 2 or not all(math.isfinite(n) for n in (*before, before_cell)) or before_cell <= 0:
                return False, "invalid previous-frame camera"
            if note["resized"] not in ("0", "1"):
                return False, "invalid resize marker"
            # Fitted resizes change physical zoom; otherwise both cameras come from this one note.
            if note["resized"] == "1" and abs(before_cell - cell) < .0001:
                centres.append(math.dist(centre, before) * cell)
            current_map = tuple(round(n) for n in mapped)
        match = re.search(r"^# layout[^\n]*die map x ([-\d.]+)\.\.([-\d.]+) y ([-\d.]+)\.\.([-\d.]+)", line)
        if match:
            legacy += 1
            x0, x1, y0, y1 = map(float, match.groups())
            if (x0, y0, x1, y1) != current_map:
                return False, "# layout map disagrees with the current central/shelf measurement"
    if not errors:
        return False, "0 measured layouts (central/shelf/camera evidence missing)"
    error, drift = max(errors), max(centres, default=float("inf"))
    passed = closed > 0 and opened > 0 and legacy > 0 and error <= .01 and drift <= 1
    return passed, f"closed={closed} widths(map/central)={sorted(widths)} open={opened} gaps={sorted(gaps)}px layout={legacy} edge-error={error:.3f}px centre-drift={drift:.3f}px (limits .01/1px)"


def toolbar_verdict(bundle, criterion):
    seen, previous = set(), None
    count = 0
    for line in bundle["window"].splitlines():
        if not line.startswith("# toolbar "):
            continue
        note = fields(line)
        try:
            size = tuple(map(float, note["size"].split(",")))
            if len(size) != 2 or not all(math.isfinite(n) and n > 0 for n in size):
                return False, "invalid toolbar size"
            if size == previous:
                return False, "duplicate toolbar note without a size change"
            previous = size
            seen.add(size)
            names = ["row", "navigation", "build", "transport", "time", "view"]
            rects = {}
            for name in names:
                value = note[name]
                if value == "-" and name in ("build", "view"):
                    continue
                rect = tuple(map(float, value.split(",")))
                if len(rect) != 4 or not all(math.isfinite(n) for n in rect) or rect[2] <= rect[0] or rect[3] <= rect[1]:
                    return False, f"invalid {name} rectangle"
                rects[name] = rect
            row = rects.pop("row")
            if row[0] < 0 or row[1] < 0 or row[2] > size[0] + .01 or row[3] > size[1] + .01 or row[3] - row[1] > 46.01:
                return False, "toolbar row does not fit the window"
            right = row[0]
            for name, rect in rects.items():
                if rect[0] < right - .01:
                    return False, f"{name} overlaps the preceding group"
                if any((rect[0] < row[0] - .01, rect[1] < row[1] - .01,
                        rect[2] > row[2] + .01, rect[3] > row[3] + .01)):
                    return False, f"{name} does not fit the row"
                right = rect[2]
            if size[0] >= 1100 and "view" not in rects:
                return False, "view controls missing at supported width"
            if note["compact"] != str(int(size[0] < 1500)):
                return False, "incorrect compact treatment"
        except (KeyError, ValueError):
            return False, "incomplete toolbar measurement"
        count += 1
    required = {tuple(size) for size in criterion.get("sizes", [[1500, 950], [1100, 800]])}
    passed = count > 0 and required <= seen
    return passed, f"toolbar layouts={count} sizes={sorted(seen)} required={sorted(required)}; groups fit without overlap"


def live_verdict(bundle, criterion):
    if criterion.get("kind", "numeric") == "unavailable":
        expected = criterion["reason"]
        matches = re.findall(r"^# value .*?:\d+:\d+ (\S+) -> (.*)$", bundle["window"], re.M)
        counts = {name: sum(n == name and (reason == expected or reason.startswith(expected + "; "))
                             for n, reason in matches)
                  for name in criterion["names"]}
        return all(counts.values()), f"unavailable={counts} reason={expected!r}"
    running, found, rate = False, {}, None
    pattern = re.compile(r"^# value .*?:\d+:\d+ (\S+) = (.+?) @ (\d+)(?:;.*)?$")
    for line in bundle["window"].splitlines():
        command = re.match(r"# script\s+[\d.]+\s+(run|pause)\b", line)
        if command:
            running = command[1] == "run"
        rate_command = re.match(r"# script\s+[\d.]+\s+rate\s+(\S+)\s*$", line)
        if rate_command:
            rate = rate_command[1]
        match = pattern.match(line)
        if running and criterion.get("rate") and rate != criterion["rate"]:
            continue
        if match and "?" not in match[2] and re.search(r"\d", match[2]):
            found.setdefault(match[1], {False: set(), True: set()})[running].add((match[2], int(match[3])))
    names = criterion["names"]
    changing = []
    for name in names:
        states = found.get(name, {False: set(), True: set()})
        changing.append(bool(states[False] and states[True] and
                             len(states[False] | states[True]) > 1))
    inline = sum(int(fields(line).get("inline_values", 0)) for line in bundle["project"].splitlines()
                 if line.startswith("# editor frames") and fields(line).get("inline") == "on")
    details = []
    for name in names:
        states = found.get(name, {False: set(), True: set()})
        samples = states[False] | states[True]
        cycles = [sample[1] for sample in samples]
        details.append(f"{name}:paused={len(states[False])}/running={len(states[True])} distinct={len(samples)} cycles={min(cycles, default='-')}..{max(cycles, default='-')}")
    details = ", ".join(details)
    return all(changing) and (not criterion.get("inline") or inline > 0), f"{details}; running-rate={criterion.get('rate', 'any')}; inline-values={inline}"


def syntax_verdict(bundle, criterion):
    languages = set(re.findall(r"^open .* language=(\w+)$", bundle["project"], re.M))
    required = set(criterion.get("languages", ["verilog", "systemverilog", "vhdl"]))
    notes = [fields(line) for line in bundle["project"].splitlines() if line.startswith("# editor frames ")]
    typing = [n for n in notes if n.get("kind") == "typing" and int(n.get("n", 0)) > 0]
    plain = [float(n["frame_p50_ms"]) for n in notes if n.get("kind") == "idle" and n.get("file", "").endswith("picorv32_plain.txt")]
    idle = [float(n["frame_p50_ms"]) for n in notes if n.get("kind") == "idle" and n.get("file", "").endswith("picorv32.v")]
    p95 = max((float(n["frame_p95_ms"]) for n in typing), default=float("inf"))
    baseline, current = max(plain, default=0), max(idle, default=float("inf"))
    themes = {n.get("theme") for n in typing}
    theme_counts = Counter(n.get("theme") for n in typing)
    measured = [float(n["frame_p95_ms"]) for n in typing] + plain + idle
    finite = all(math.isfinite(n) and n >= 0 for n in measured)
    budget = float(criterion.get("budget_ms", 16.7))
    limit = baseline * float(criterion.get("idle_ratio", 1.25)) + float(criterion.get("idle_slack_ms", .5))
    passed = (required <= languages and len(typing) >= criterion.get("typing_phases", 6)
              and themes == {"dark", "light"} and all(theme_counts[t] >= 3 for t in themes)
              and finite and bool(plain) and bool(idle)
              and p95 <= budget and current <= limit)
    return passed, f"languages={','.join(sorted(languages))}; typing-phases={len(typing)} themes={','.join(sorted(themes))} p95={p95:.3f}/{budget:g}ms; idle-p50={current:.3f} plain={baseline:.3f} limit={limit:.3f}ms"


CHECKS = {"note", "numeric", "layout", "toolbar", "no_home", "no_error", "live", "syntax"}


def judge(bundle, criterion):
    kind = criterion["check"]
    source = "both" if kind in ("no_home", "no_error") else criterion.get("source", "both")
    if source not in ("window", "project", "both"):
        raise ValueError(f"unknown record source: {source}")
    text = "\n".join(bundle[key] for key in (("window", "project") if source == "both" else (source,)))
    if kind == "no_home":
        home = os.environ.get("HOME", "")
        leaks = sum(1 for line in text.splitlines() if
                    (home.startswith("/") and home in line) or re.search(r"/(?:Users|home)/[^/\s]+(?:/|$)", line))
        return leaks == 0, f"absolute-home-lines={leaks}"
    if kind == "no_error":
        errors = re.findall(r"(?im)^.*(?:panicked at|^live:|^error:|# timeout|# script error|chip error|load failed|build failed|open failed|project error|# tour failed|critical path:.*not supported).*$", text + "\n" + bundle["output"])
        exits = re.findall(r"^# window exit (-?\d+)$", bundle["output"], re.M)
        return not errors and bool(exits) and all(code == "0" for code in exits), f"window-errors={len(errors)} exit={','.join(exits) or 'missing'}"
    if kind in ("note", "numeric"):
        matches = list(re.finditer(criterion["pattern"], text, re.M))
        minimum = criterion.get("min", 1)
        if kind == "note":
            return len(matches) >= minimum, f"matches={len(matches)} min={minimum}"
        values = [float(m.groupdict().get("value") or m.group(1)) for m in matches]
        bound = float(criterion["bound"])
        op = criterion.get("op", "<=")
        compare = {"<=": lambda n: n <= bound, "<": lambda n: n < bound,
                   ">=": lambda n: n >= bound, ">": lambda n: n > bound,
                   "==": lambda n: n == bound}
        if op not in compare:
            raise ValueError(f"unknown numeric comparison: {op}")
        passed = len(values) >= minimum and all(math.isfinite(v) and compare[op](v) for v in values)
        return passed, f"n={len(values)} range={min(values, default=float('nan')):g}..{max(values, default=float('nan')):g} {op} {bound:g}"
    if kind == "layout":
        return layout_verdict(bundle, criterion)
    if kind == "toolbar":
        return toolbar_verdict(bundle, criterion)
    if kind == "live":
        return live_verdict(bundle, criterion)
    if kind == "syntax":
        return syntax_verdict(bundle, criterion)
    raise ValueError(f"unknown criterion: {kind}")


def batch_entries(args):
    if args.file.suffix == ".toml":
        import tomllib
        config = tomllib.loads(args.file.read_text())
    else:
        config = json.loads(args.file.read_text())
    entries = [entry for entry in config["records"] if entry.get("enabled", True)]
    selected = set(getattr(args, "only", []) or [])
    labels = [entry["label"] for entry in entries]
    if len(labels) != len(set(labels)):
        raise ValueError("batch labels must be unique")
    if selected - set(labels):
        raise ValueError(f"unknown --only names: {', '.join(sorted(selected - set(labels)))}")
    if selected:
        entries = [entry for entry in entries if entry["label"] in selected]
    seen = set()
    for entry in entries:
        if entry["label"] in seen:
            raise ValueError(f"duplicate label: {entry['label']}")
        seen.add(entry["label"])
        if bool(entry.get("design")) == bool(entry.get("project")):
            raise ValueError("each entry needs exactly one design or project")
        if not entry.get("script") and not entry.get("project_script"):
            raise ValueError("each entry needs a window script and/or a project script")
        if entry.get("project_script") and not entry.get("project"):
            raise ValueError("a project script requires a project")
        for rule in entry.get("criteria", []):
            if rule["check"] not in CHECKS:
                raise ValueError(f"unknown criterion: {rule['check']}")
            if rule["check"] in ("note", "numeric"):
                regex = re.compile(rule["pattern"])
                if rule.get("min", 1) < 1:
                    raise ValueError("a regex check needs min >= 1")
                if rule["check"] == "numeric" and (regex.groups < 1 or
                        rule.get("op", "<=") not in ("<", "<=", "==", ">=", ">") or
                        not math.isfinite(float(rule["bound"]))):
                    raise ValueError("numeric needs a capture, finite bound and valid op")
            if rule["check"] == "live":
                if not rule.get("names"):
                    raise ValueError("live needs at least one name")
                if rule.get("kind", "numeric") not in ("numeric", "unavailable"):
                    raise ValueError("live kind must be numeric or unavailable")
                if rule.get("kind") == "unavailable" and not rule.get("reason"):
                    raise ValueError("unavailable live check needs an exact reason")
        entry_environment(entry)
        if "prepare" in entry:
            prepare = entry["prepare"]
            if not isinstance(prepare, dict) or not isinstance(prepare.get("output"), str) or not prepare["output"]:
                raise ValueError("prepare needs command argv and an output path")
            command_argv(prepare.get("command"))
            command_timeout(prepare.get("timeout", 600))
        if "judge" in entry:
            command_argv(entry["judge"])
            command_timeout(entry.get("judge_timeout", 60))
        for check in ("no_home", "no_error"):
            if not any(c["check"] == check for c in entry.setdefault("criteria", [])):
                entry["criteria"].append({"check": check})
    if not entries:
        raise ValueError("batch has no enabled entries")
    return entries


def entry_args(entry, args):
    rooted = lambda key: (ROOT / entry[key]) if entry.get(key) else None
    design = entry.get("design")
    if design and design.endswith(".config"):
        design = str(ROOT / design)
    return argparse.Namespace(design=design, project=rooted("project"),
                              project_script=rooted("project_script"), script=rooted("script"),
                              label=entry["label"], binary=args.binary, who=args.who,
                              env=[[f"{k}={v}" for k, v in entry.get("env", {}).items()]],
                              timeout=entry.get("timeout", 300), start_screen=False,
                              unset_env=entry.get("unset_env", []))


def command_argv(command):
    if not isinstance(command, list) or not command or not command[0] or any(
            not isinstance(arg, str) or "\0" in arg for arg in command):
        raise ValueError("command must be a nonempty argv array of strings (no shell)")
    return command


def command_timeout(seconds):
    if not isinstance(seconds, (float, int)) or not math.isfinite(seconds) or seconds <= 0:
        raise ValueError("command timeout must be positive and finite")
    return seconds


def entry_environment(entry):
    env = dict(os.environ)
    unset = entry.get("unset_env", [])
    values = entry.get("env", {})
    if not isinstance(unset, list) or not isinstance(values, dict):
        raise ValueError("unset_env must be an array and env an object")
    for key in [*unset, *values]:
        if not isinstance(key, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            raise ValueError("invalid environment variable name")
    if set(unset) & values.keys():
        raise ValueError("a variable cannot be both set and unset")
    for key in unset:
        env.pop(key, None)
    for key, value in values.items():
        env[key] = str(value)
        if "\0" in env[key]:
            raise ValueError("environment values cannot contain NUL")
    return env


def display_command(command, entry, extra=None):
    prefix = ["env"]
    for key in entry.get("unset_env", []):
        prefix.extend(("-u", key))
    prefix.extend(f"{key}={value}" for key, value in entry.get("env", {}).items())
    prefix.extend(f"{key}={value}" for key, value in (extra or {}).items())
    return shlex.join(prefix + command)


def judge_command(entry):
    record = str(RECORDS / f"{entry['label']}.tsv")
    return [arg.replace("{record}", record) for arg in entry["judge"]]


def prepare_entry(entry, planned=None):
    """Run offline preparation, or describe it without mutating files in dry-run."""
    prepare = entry.get("prepare")
    if not prepare:
        return None
    output = (ROOT / prepare["output"]).resolve()
    command = prepare["command"]
    label = entry["label"]
    if output.exists() or (planned is not None and output in planned):
        why = "output already exists" if output.exists() else "output prepared earlier in this batch"
        print(f"SKIP prepare {label}: {why}: {prepare['output']}")
        return label, "prepare", f"skipped: {why}: {prepare['output']}", "PASS"
    print(f"PREPARE {label}: {display_command(command, entry)}", flush=True)
    if planned is not None:
        planned.add(output)
        return None
    output.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(command, cwd=ROOT, env=entry_environment(entry),
                            capture_output=True, text=True,
                            timeout=prepare.get("timeout", 600), check=False)
    ok = result.returncode == 0 and output.exists()
    detail = f"exit={result.returncode} output={prepare['output']} exists={output.exists()}"
    if not ok:
        detail += " " + " ".join((result.stdout + result.stderr).splitlines())
    return label, "prepare", detail, "PASS" if ok else "FAIL"


def external_verdicts(entry):
    try:
        result = subprocess.run(judge_command(entry), cwd=ROOT, env=entry_environment(entry),
                                capture_output=True, text=True,
                                timeout=entry.get("judge_timeout", 60), check=False)
    except (OSError, subprocess.SubprocessError) as error:
        return [(entry["label"], "judge exit", str(error), "FAIL")]
    rows = []
    for channel, text in (("judge", result.stdout), ("judge stderr", result.stderr)):
        for line in text.splitlines():
            match = re.match(r"^(PASS|FAIL)(?:\s|$)", line.lstrip())
            status = match[1] if match else ("PASS" if result.returncode == 0 else "FAIL")
            rows.append((entry["label"], channel, line, status))
    rows.append((entry["label"], "judge exit", f"exit={result.returncode}",
                 "PASS" if result.returncode == 0 else "FAIL"))
    return rows


def print_verdicts(rows):
    print("record | criterion | measurement | verdict")
    for row in rows:
        print(" | ".join(row))
    return bool(rows) and all(row[-1] == "PASS" for row in rows)


def record_batch(args):
    entries = batch_entries(args)
    label = f"batch-{args.file.stem}"
    if args.dry_run:
        planned = set()
        for entry in entries:
            prepare_entry(entry, planned)
        who = args.who or os.environ.get("ECP5_SESSION") or label
        print(f"LOCK {screen_lock_path()} who={who} label={label}")
        for entry in entries:
            command, env, *_ = prepare_run(entry_args(entry, args), validate=False)
            extra = {key: env[key] for key in ("ECP5_GUI_SCRIPT", "ECP5_PROJECT_SCRIPT",
                     "ECP5_GUI_TRACE", "ECP5_PROJECT_TRACE") if key in env}
            print(f"RECORD {entry['label']}: {display_command(command, entry, extra)}")
            if "judge" in entry:
                print(f"JUDGE {entry['label']}: {display_command(judge_command(entry), entry)}")
        print(f"UNLOCK {screen_lock_path()}")
        return True
    rows = []
    if not args.check:
        try:
            for entry in entries:
                result = prepare_entry(entry)
                if result:
                    rows.append(result)
                    if result[-1] == "FAIL":
                        return print_verdicts(rows)
            for entry in entries:
                prepare_run(entry_args(entry, args))
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            rows.append((entry["label"], "prepare/preflight", str(error), "FAIL"))
            return print_verdicts(rows)
        console_unlocked()
        for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(sig, interrupted)
    with nullcontext() if args.check else screen_lock(label, args.who):
        for entry in entries:
            try:
                if not args.check:
                    record_run(entry_args(entry, args), lock_held=True)
                bundle = record_bundle(entry)
                verdicts = []
                for criterion in entry["criteria"]:
                    try:
                        ok, measurement = judge(bundle, criterion)
                    except (ValueError, KeyError, TypeError, IndexError) as error:
                        ok, measurement = False, f"invalid/missing evidence: {error}"
                    verdicts.append(ok)
                    rows.append((entry["label"], criterion["check"], measurement, "PASS" if ok else "FAIL"))
                if "judge" in entry:
                    external = external_verdicts(entry)
                    rows.extend(external)
                    verdicts.extend(row[-1] == "PASS" for row in external)
                if not all(verdicts):
                    break
            except (OSError, ValueError, subprocess.SubprocessError) as error:
                rows.append((entry["label"], "record", str(error), "FAIL"))
                break
    return print_verdicts(rows)


def interrupted(signum, frame):
    # Unwind the process and lock contexts on termination as well as Ctrl-C.
    raise SystemExit(128 + signum)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = ap.add_subparsers(dest="command", required=True)
    run = commands.add_parser("run", help="record a scripted window run")
    run.add_argument("inputs", nargs="+", help="DESIGN SCRIPT LABEL, or SCRIPT LABEL with --project")
    run.add_argument("--project", type=pathlib.Path)
    run.add_argument("--project-script", type=pathlib.Path)
    run.add_argument("--who", metavar="NAME",
                     help="screen lock holder (default: ECP5_SESSION, else LABEL)")
    run.add_argument("--binary", type=pathlib.Path, default=ROOT / "target/release/ecp5_gui")
    run.add_argument("--env", nargs="+", action="append", default=[], metavar="K=V")
    run.add_argument("--start-screen", action="store_true", help="let the script open the design")
    run.add_argument("--timeout", type=float, default=300)
    summary = commands.add_parser("sum", help="sum records up; compare two side by side")
    summary.add_argument("records", nargs="+", type=pathlib.Path)
    batch = commands.add_parser("batch", help="record a batch in one screen-lock block and judge its records")
    batch.add_argument("file", type=pathlib.Path)
    batch.add_argument("--binary", type=pathlib.Path, default=ROOT / "target/release/ecp5_gui")
    batch.add_argument("--who")
    batch.add_argument("--only", nargs="+", default=[], metavar="NAME",
                       help="select entry labels; keep their batch order")
    mode = batch.add_mutually_exclusive_group()
    mode.add_argument("--dry-run", action="store_true", help="validate inputs and print commands without opening a window")
    mode.add_argument("--check", action="store_true", help="judge existing records without opening a window")
    args = ap.parse_args()
    try:
        if args.command == "batch":
            if not record_batch(args):
                ap.exit(1)
            return
        if args.command == "run":
            expected = 2 if args.project else 3
            if len(args.inputs) != expected:
                raise ValueError("run needs SCRIPT LABEL with --project, otherwise DESIGN SCRIPT LABEL")
            args.script, args.label = pathlib.Path(args.inputs[-2]), args.inputs[-1]
            args.design = None if args.project else args.inputs[0]
            if args.project_script and not args.project:
                raise ValueError("--project-script requires --project")
        if args.command == "sum":
            summarize(args.records)
        else:
            for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                signal.signal(sig, interrupted)
            record_run(args)
    except (OSError, ValueError, subprocess.SubprocessError, plistlib.InvalidFileException) as error:
        ap.exit(1, f"gui_record: {error}\n")


if __name__ == "__main__":
    main()
