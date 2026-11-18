#!/usr/bin/env python3
"""Summarize GPU board judgments by script phase."""

import argparse
import csv
import math
import re
import statistics
from dataclasses import dataclass
from pathlib import Path


def number(value):
    """Missing, incomplete and non-finite measurements have no numeric value."""
    try:
        result = float(value)
        return result if math.isfinite(result) else None
    except (TypeError, ValueError):
        return None


def read_record(path):
    frames, notes = [], []
    header = None
    with Path(path).open(encoding="utf-8") as source:
        for line_no, line in enumerate(source, 1):
            if not line.strip():
                continue
            if line.lstrip().startswith("#"):
                notes.append(line.rstrip("\r\n"))
                continue
            cells = next(csv.reader([line], delimiter="\t"))
            if header is None:
                header = cells
                if "t" not in header:
                    raise ValueError("record header must contain t")
                continue
            frame = dict(zip(header, cells))
            t = number(frame.get("t"))
            if t is None:
                raise ValueError(f"{path}:{line_no}: invalid frame time")
            frame["t"] = t
            if frames and t < frames[-1]["t"]:
                raise ValueError("record times restart; summarize one design at a time")
            frames.append(frame)
    return frames, notes


def record_end(frames, notes):
    times = [frame["t"] for frame in frames]
    for line in notes:
        fields = line.split("\t")
        if fields[0] == "# board" and len(fields) > 1:
            t = number(fields[1])
            if t is not None:
                times.append(t)
    return max(times, default=0.0)


@dataclass(frozen=True)
class Phase:
    start: float
    end: float
    name: str


def phases(script, end):
    commands = []
    for line_no, line in enumerate(Path(script).read_text(encoding="utf-8").splitlines(), 1):
        words = line.split("#", 1)[0].split()
        if not words:
            continue
        t = number(words[0])
        if t is None or t < 0 or len(words) < 2:
            raise ValueError(f"{script}:{line_no}: expected seconds and command")
        if words[1] in ("open", "close"):
            raise ValueError("scripts that switch designs must be split into separate records")
        commands.append((t, words[1:]))
    commands.sort(key=lambda command: command[0])  # Stable for simultaneous commands.
    rate, values, zoom, mode, view = "real", False, 1.0, "pause", "3d"
    for i, (t, words) in enumerate(commands):
        command = words[0]
        arg = words[1] if len(words) > 1 else None
        if command == "rate" and arg is not None:
            rate = arg
        elif command == "values":
            values = arg == "on"
        elif command == "zoom":
            factor = number(arg)
            if factor is None or factor <= 0:
                raise ValueError(f"invalid zoom factor: {arg}")
            zoom *= factor
        elif command == "fit":
            zoom = 1.0
        elif command in ("run", "pause", "step"):
            mode = command
        elif command == "board" and arg is not None:
            view = arg
        stop = commands[i + 1][0] if i + 1 < len(commands) else end
        start = t + 1.0
        if stop - start < 2.0:
            continue
        name = f"{rate} c/s" + (" 0/1" if values else "")
        yield Phase(start, stop, f"{name} x{zoom:g} {mode} view {view}")


def fmt(value, digits=1):
    if value is None:
        return "-"
    return str(value) if isinstance(value, int) and digits == 0 else f"{value:.{digits}f}"


def match_number(pattern, text):
    match = re.search(pattern, text)
    return number(match.group(1)) if match else None


def board_rows(notes):
    rows = []
    for line in notes:
        fields = line.split("\t")
        if fields[0] != "# board" or len(fields) < 2:
            continue
        t = number(fields[1])
        if t is not None:
            rows.append((t, fields))
    return rows


def option_counts(rows, kind):
    asked, drawn = 0, 0
    for fields in rows:
        note = next((field for field in fields if field.startswith(kind + " ")), None)
        if note is None:
            return "-"
        if note.startswith(kind + " asked for"):
            asked += 1
        if kind == "OLED module":
            # New form distinguishes the physical module from its screen.
            visible = bool(re.search(r": module drawn(?:,|$)", note))
            # The not-requested (including WRONG) form has no module subfield.
            visible |= bool(re.search(r", drawn(?:: WRONG)?$", note))
        else:
            visible = bool(re.search(r", drawn(?:: WRONG)?$", note))
        drawn += visible
    return f"{asked}/{drawn}" if rows else "-"


def summarize(rows, phase):
    selected = [fields for t, fields in rows if phase.start <= t < phase.end]
    wrong, unseen, new_values = 0, 0, 0
    bits_known = True
    intervals, gaps = [], []
    for fields in selected:
        wrong += sum(int(match.group(1)) for field in fields
                     if (match := re.fullmatch(r"WRONG (\d+)", field)))
        lit = next((field.removeprefix("drawn lit ") for field in fields
                    if re.fullmatch(r"drawn lit [01]{8}", field)), None)
        dark = next((field.removeprefix("drawn dark ") for field in fields
                     if re.fullmatch(r"drawn dark [01]{8}", field)), None)
        if lit is None or dark is None:
            bits_known = False
        else:
            unseen += sum(a == b == "0" for a, b in zip(lit, dark))
        change = next((field for field in fields if field.startswith("new LED values")), None)
        if change is not None:
            new_values += 1
            interval = match_number(r"([\d.]+) ms after the drawn values before", change)
            gap = match_number(r"drawn (\d+) frames after they came", change)
            if interval is not None:
                intervals.append(interval)
            if gap is not None:
                gaps.append(gap)
    last_options = next((match_number(r"^options drawn wrong (\d+)$", field)
                         for fields in reversed(selected) for field in fields
                         if field.startswith("options drawn wrong ")), None)
    return [
        phase.name, fmt(phase.start, 3), fmt(phase.end, 3), str(len(selected)), str(wrong),
        str(unseen) if bits_known else "-", str(new_values),
        fmt(statistics.median(intervals) if intervals else None),
        fmt(max(intervals) if intervals else None), fmt(max(gaps) if gaps else None, 0),
        option_counts(selected, "OLED module"), option_counts(selected, "HDMI cable"),
        fmt(last_options, 0),
    ]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("record")
    parser.add_argument("script")
    args = parser.parse_args()
    try:
        frames, notes = read_record(args.record)
        rows = board_rows(notes)
        print("\t".join([
            "phase", "start_s", "end_s", "checked", "led_wrong", "led_unseen", "new_values",
            "interval_p50_ms", "interval_max_ms", "came_frames_max", "oled_asked/drawn",
            "hdmi_asked/drawn", "options_wrong_last",
        ]))
        for phase in phases(args.script, record_end(frames, notes)):
            print("\t".join(summarize(rows, phase)))
        for line in notes:
            if line.split("\t", 1)[0] == "# layout":
                print(line)
    except (OSError, ValueError) as error:
        parser.exit(2, f"{parser.prog}: {error}\n")


if __name__ == "__main__":
    main()
