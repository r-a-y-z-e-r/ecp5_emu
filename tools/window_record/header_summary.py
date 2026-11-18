#!/usr/bin/env python3
"""Judge the driven Header J1/J2 screen-batch record without opening a window."""

import argparse
import csv
import math
import re
from dataclasses import dataclass
from pathlib import Path

# Window endpoints and three-decimal notes need a little allowance. These
# are model-statistic tolerances, not analog or hardware accuracy claims.
HZ_RELATIVE = 0.01
DUTY_POINTS = 1.0
SETTLE_SECONDS = 2.0
PWM = {f"GP{i}": pair for i, pair in enumerate(
    ((1000, 25), (2000, 50), (5000, 75), (10000, 10)))}
NAMES = {f"{kind}{i}" for i in range(28) for kind in ("GN", "GP")}
PAD_COUNTS = ((1000, 6250000), (2000, 12500000),
              (5000, 18750000), (10000, 2500000))
NUMBER = r"(?:[0-9]+(?:\.[0-9]+)?)"
PIN = re.compile(r"(G[NP][0-9]+)=([01XZ])\Z")
MEASURE = re.compile(rf"(--|{NUMBER})Hz/(--|{NUMBER})%\Z")
UART = re.compile(
    r"HEADER ([0-9A-F]{8}) ([0-9A-F]{8}) ([0-9A-F]{8}) ([0-9A-F]{8}) P "
    r"([0-9A-F]{8})/([0-9A-F]{8}) ([0-9A-F]{8})/([0-9A-F]{8}) "
    r"([0-9A-F]{8})/([0-9A-F]{8}) ([0-9A-F]{8})/([0-9A-F]{8}) "
    r"([0-9A-F]{14}) E([01])\Z")


@dataclass
class Record:
    frames: list
    commands: list
    boards: list
    uart: bytes
    header_notes: int


def finite(value):
    result = float(value)
    if not math.isfinite(result) or result < 0:
        raise ValueError(f"invalid nonnegative time/rate: {value!r}")
    return result


def header_pins(text):
    words, pins = text.split(), {}
    if len(words) != 56 * 3:
        raise ValueError("# header needs all 56 three-field pin entries")
    for offset in range(0, len(words), 3):
        pin = PIN.fullmatch(words[offset])
        measure = MEASURE.fullmatch(words[offset + 1])
        external = re.fullmatch(r"ext=([01z])", words[offset + 2])
        if not pin or not measure or not external:
            raise ValueError("malformed # header pin, Hz/duty or external setting")
        name, level = pin.groups()
        if name not in NAMES or name in pins:
            raise ValueError(f"unknown or duplicate # header pin: {name}")
        hz, duty = (None if v == "--" else finite(v) for v in measure.groups())
        pins[name] = (level, hz, duty, external[1])
    return pins


def uart_bytes(text):
    """Decode app.rs record_text; do not interpret arbitrary Python escapes."""
    result, index = bytearray(), 0
    escapes = {"r": 13, "n": 10, "t": 9, "\\": 92}
    while index < len(text):
        char = text[index]
        index += 1
        if char != "\\":
            if not 0x20 <= ord(char) <= 0x7e:
                raise ValueError("non-printable byte in escaped # uart data")
            result.append(ord(char))
            continue
        if index == len(text):
            raise ValueError("unfinished escape in # uart data")
        escape = text[index]
        index += 1
        if escape in escapes:
            result.append(escapes[escape])
        elif escape == "x" and re.fullmatch(r"[0-9a-fA-F]{2}", text[index:index + 2]):
            result.append(int(text[index:index + 2], 16))
            index += 2
        else:
            raise ValueError("unknown or malformed escape in # uart data")
    return bytes(result)


def read_record(path):
    frames, commands, boards, data = [], [], [], bytearray()
    columns, pins, count, designs = None, None, 0, 0
    uart_time, uart_cycle = -1.0, -1
    with Path(path).open(encoding="utf-8") as source:
        for number, line in enumerate(source, 1):
            if not line.endswith("\n"):
                raise ValueError(f"line {number}: record ends in a truncated row")
            line = line.rstrip("\r\n")
            if not line.strip():
                continue
            if line.startswith("# header "):
                pins = header_pins(line[len("# header "):])
                count += 1
            elif line.startswith("# script\t"):
                _, time, command = line.split("\t", 2)
                time = finite(time)
                if commands and time < commands[-1][0]:
                    raise ValueError("script times restart; use one design per record")
                commands.append((time, command))
            elif line.startswith("# board\t"):
                fields = line.split("\t")
                if len(fields) < 3 or not re.fullmatch(r"drawing [0-9]+", fields[2]):
                    raise ValueError("malformed # board drawing")
                boards.append((finite(fields[1]), fields[3:]))
            elif line.startswith("# uart\t"):
                _, time, cycle, payload = line.split("\t", 3)
                time, cycle = finite(time), int(cycle)
                if time < uart_time or cycle < max(0, uart_cycle):
                    raise ValueError("UART time/cycle goes back; use a fresh run")
                uart_time, uart_cycle = time, cycle
                data.extend(uart_bytes(payload))
            elif line.startswith("# design"):
                designs += 1
                if designs > 1:
                    raise ValueError("use one design per record")
            elif line.startswith("#"):
                continue
            else:
                cells = next(csv.reader([line], delimiter="\t"))
                if columns is None:
                    columns = cells
                    if len(set(columns)) != len(columns) or not {"t", "running", "rate", "cycle"} <= set(columns):
                        raise ValueError("record needs unique t/running/rate/cycle columns")
                    continue
                if len(cells) != len(columns):
                    raise ValueError(f"line {number}: incomplete frame row")
                frame = dict(zip(columns, cells))
                frame["t"] = finite(frame["t"])
                frame["cycle"] = int(frame["cycle"])
                if frame["cycle"] < 0 or frame["running"] not in ("0", "1"):
                    raise ValueError("invalid frame cycle/running state")
                if frame["rate"] != "max":
                    finite(frame["rate"])
                if frames and frame["t"] < frames[-1]["t"]:
                    raise ValueError("frame times restart; use one design per record")
                # Notes have no timestamp. They describe the next frame and
                # remain the displayed status until another note changes it.
                frame["pins"] = pins
                frames.append(frame)
    if columns is None or not frames:
        raise ValueError("no frame rows")
    return Record(frames, commands, boards, bytes(data), count)


def phases(record):
    result, rate, start = [], "real", None
    for time, command in record.commands:
        words = command.split()
        if not words:
            raise ValueError("empty script command")
        if words[0] in ("open", "close", "build-demo", "back", "goto", "step"):
            raise ValueError("use a fresh uninterrupted run for this summary")
        if words[0] == "rate":
            if start is not None:
                result.append((start, time, rate))
                start = time
            rate = words[1] if len(words) == 2 else "invalid"
        elif words[0] == "run" and start is None:
            start = time
        elif words[0] in ("pause", "quit") and start is not None:
            result.append((start, time, rate))
            start = None
    if start is not None:
        result.append((start, record.frames[-1]["t"], rate))
    return result


def judge(record, external=None):
    checks = []
    def add(name, passed, detail):
        checks.append((name, bool(passed), detail))
    gn0 = int(external[-1]) if external else 1
    add("3D board", record.boards, f"{len(record.boards)} # board drawings")
    add("Header notes", record.header_notes, f"{record.header_notes} complete # header notes")
    runs = phases(record)
    add("rate phases", [p[2] for p in runs] == ["real", "max"],
        "expected running at real, then max")
    for start, end, rate in runs:
        rows = [f for f in record.frames if start + SETTLE_SECONDS <= f["t"] < end]
        state_ok = rows and all(f["running"] == "1" and (
            f["rate"] == "max" if rate == "max" else f["rate"] != "max" and float(f["rate"]) == 25000000
        ) for f in rows)
        add(f"{rate} running", state_ok, f"{len(rows)} frames after {SETTLE_SECONDS:g} s settling")
        snapshots = {id(f["pins"]): f["pins"] for f in rows if f["pins"] is not None}
        known = rows and all(f["pins"] is not None for f in rows)
        for name, (hz, duty) in PWM.items():
            bad = []
            for pins in snapshots.values():
                level, got_hz, got_duty, ext = pins[name]
                if (level not in "01" or ext != "z" or got_hz is None or got_duty is None
                        or abs(got_hz - hz) > hz * HZ_RELATIVE
                        or abs(got_duty - duty) > DUTY_POINTS):
                    bad.append(f"{level} {got_hz}Hz/{got_duty}% ext={ext}")
            add(f"{rate} {name}", known and not bad,
                "; ".join(bad) or f"{hz} Hz +/-1%, {duty}% +/-1 percentage point")
        bad = set()
        for pins in snapshots.values():
            for name in NAMES - PWM.keys():
                want = gn0 if name == "GN0" else int(name.startswith("GN"))
                want_ext = str(gn0) if external and name == "GN0" else "z"
                level, hz, duty, ext = pins[name]
                if (level, hz, duty, ext) != (str(want), None, None, want_ext):
                    bad.add(f"{name}={level} ext={ext}, expected {want} ext={want_ext}")
        add(f"{rate} 52 inputs", known and not bad,
            "; ".join(sorted(bad)) or f"pulls and external setting {external or 'all released'}")
        # LED drawings are change records: the last drawing remains in force.
        # Include the settled-start drawing once, then every later drawing.
        settled_start = start + SETTLE_SECONDS
        previous, board_rows = None, []
        for time, fields in record.boards:
            if time <= settled_start:
                if previous is None or time >= previous[0]:
                    previous = (time, fields)
            elif time < end:
                board_rows.append(fields)
        if previous is not None:
            board_rows.insert(0, previous[1])
        visible, wrong = 0, 0
        for fields in board_rows:
            values = []
            for prefix in ("LEDs D0..D7 to show lit ", "drawn lit ", "drawn dark "):
                value = next((f[len(prefix):] for f in fields if f.startswith(prefix)), "")
                values.append(value if re.fullmatch(r"[01]{8}", value) else None)
            asked, lit, dark = values
            if asked is None or lit is None or dark is None:
                wrong += 1
            elif asked[0] != str(gn0):
                wrong += 1
            elif lit[0] == dark[0] == "0":
                continue  # No GPU evidence when this LED is out of sight.
            elif (lit[0], dark[0]) != (str(gn0), str(1 - gn0)):
                wrong += 1
            else:
                visible += 1
        add(f"{rate} LED0 echo", visible and not wrong,
            f"{visible} visible correct drawings, {wrong} wrong; GN0={gn0}" if board_rows else
            "no # board drawing at or before the settled start or inside this phase")

    # UART bytes arrive in arbitrary fragments. Only LF-terminated lines
    # are evidence; retain and report an incomplete tail without parsing it.
    chunks = record.uart.split(b"\n")
    tail, lines = chunks[-1], chunks[:-1]
    add("UART complete lines", lines, f"{len(lines)} lines; {len(tail)} trailing bytes ignored")
    expected_word = 0x55555555555555 if gn0 else 0x55555555555554
    for second, raw in enumerate(lines, 1):
        text = raw.removesuffix(b"\r").decode("ascii", errors="replace")
        match = UART.fullmatch(text)
        add(f"UART {second} format", match is not None, "HEADER counters P pads levels echo")
        if match is None:
            continue
        values = [int(v, 16) for v in match.groups()[:-1]]
        add(f"UART {second} counters", values[:4] == [second * n for n in (1000, 2000, 5000, 10000)],
            "cumulative internal 1/2/5/10 kHz period counts")
        add(f"UART {second} pad counts", values[4:12] == [n for pair in PAD_COUNTS for n in pair],
            "per-second rising edges/high cycles, GP0..GP3")
        add(f"UART {second} inputs/echo", values[12] == expected_word and match[14] == str(gn0),
            f"56-bit word {expected_word:014X}, E{gn0}")
    return checks


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, epilog=(
        "Checks both rate phases after 2 s settling: frequencies +/-1%, duty +/-1 percentage point. "
        "UART counts and input levels must match exactly; fragmented UART is reassembled."))
    parser.add_argument("record", metavar="RECORD.tsv", type=Path)
    parser.add_argument("--external", choices=("GN0=0", "GN0=1"),
                        help="expected startup override; absent means all header inputs released")
    args = parser.parse_args(argv)
    try:
        checks = judge(read_record(args.record), args.external)
    except (OSError, ValueError) as error:
        print(f"FAIL record: {error}")
        return 1
    for name, passed, detail in checks:
        print(f"{'PASS' if passed else 'FAIL'} {name}: {detail}")
    return 0 if all(passed for _, passed, _ in checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
