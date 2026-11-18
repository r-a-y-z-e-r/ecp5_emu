#!/usr/bin/env python3
"""Runs a gallery demo in the emulator's window with its gui.script, then sums up the
window's own record (ECP5_GUI_TRACE, see gui/src/trace.rs).

Usage: examples/gallery/gui_run.py DEMO... [--no-run] [--trace FILE]
       examples/gallery/gui_run.py all
       examples/gallery/gui_run.py tour [--no-run]

`tour` runs examples/gallery/tour.script in ONE window, which opens each demo in turn
(`open`), with the record in target/gallery/tour_trace.tsv, and sums the record up demo by
demo: the frames, the rate, the script's commands that reached that demo (`# script` lines
after its `# design` line), the console's bytes, the 3D board's and the monitor's drawings.

Several demos, or `all`, run one after another: a tour of the gallery in the window, in the
order of TOUR below, each with its record summed up. Build them first (check.py --build).

Without --no-run it starts target/release/ecp5_gui on target/gallery/DEMO/top.config with
ECP5_GUI_SCRIPT=examples/gallery/DEMO/gui.script and the record in
target/gallery/DEMO/gui_trace.tsv, and waits for the script's `quit`. Then it prints:
- how long the run was, the cycles it reached and the rate the window showed;
- frame times;
- the 3D board's LED drawings: how many were checked against the LED values and how many
  LEDs were drawn wrong, and the LED patterns shown;
- OLED pictures painted, their completed-frame range and data CRC matches to the design;
- SW and PLAYER lines;
- the UART text the window's console received (`# uart` lines), written to
  target/gallery/DEMO/gui_uart.txt and compared with expected/uart.txt as check.py does.
Every window run requires an awake, unlocked display and takes the main checkout's
out/screen.lock, held across the whole sequence for `all` or several demos. Its label is
gallery-<demo>, gallery-all or gallery-tour; its holder is ECP5_SESSION, else demos.
--no-run only summarizes existing records, without checking the display or taking a lock.
Judge the window only by this record.
"""
import argparse
import os
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "examples/gallery"))
import check  # noqa: E402
sys.path.insert(0, str(ROOT / "tools"))
from gui_record import console_unlocked, screen_lock  # noqa: E402

# btn[0] .. btn[6], as src/board.rs names them.
BUTTONS = ["PWR", "FIRE1", "FIRE2", "UP", "DOWN", "LEFT", "RIGHT"]


def button_line(line):
    """`# button N down|up <seconds> <cycle>` as (N, down), or None."""
    parts = line.split("\t")
    if line.startswith("# button\t") and len(parts) >= 3 and parts[1].isdigit():
        return int(parts[1]), parts[2] == "down"
    return None


def presses(lines):
    """`FIRE1 x2, RIGHT x1`: the buttons pressed, from a record's `# button` lines."""
    counts = {}
    for line in lines:
        b = button_line(line)
        if b and b[1]:
            counts[b[0]] = counts.get(b[0], 0) + 1
    return ", ".join(f"{BUTTONS[i] if i < len(BUTTONS) else i} x{n}" for i, n in sorted(counts.items()))


# The order a tour shows the demos in: the computer you can see first.
TOUR = ["riscv_terminal", "video_life", "mandelbrot", "fir_scope", "ebr_life", "led_tennis",
        "riscv_shell", "serv_shell"]


def unescape(text):
    """The record's escaping: \\r \\n \\t \\\\ \\xHH."""
    out = bytearray()
    i = 0
    while i < len(text):
        c = text[i]
        if c == "\\" and i + 1 < len(text):
            n = text[i + 1]
            if n in "rnt\\":
                out += {"r": b"\r", "n": b"\n", "t": b"\t", "\\": b"\\"}[n]
                i += 2
                continue
            if n == "x" and re.fullmatch(r"[0-9a-fA-F]{2}", text[i + 2:i + 4]):
                out.append(int(text[i + 2:i + 4], 16))
                i += 4
                continue
        out += c.encode("latin-1", "replace")
        i += 1
    return bytes(out)


def oled_pictures(lines, uart):
    """Check completed-frame CRCs; frames=0 has only the model's placeholder CRC."""
    pictures = []
    for line in lines:
        m = re.match(r"# oled picture (\d+) (on|off) frames (\d+) data crc ([0-9A-Fa-f]{8}) "
                     r"picture crc ([0-9A-Fa-f]{8}) at ", line)
        if m:
            pictures.append((int(m[1]), int(m[3]), m[4].upper()))
    crcs = {c.upper() for c in re.findall(rb"(?m)^F[0-9A-Fa-f]+\b.*\bC([0-9A-Fa-f]{8})\b", uart)}
    bad = [i for i, (_, frames, crc) in enumerate(pictures)
           if frames > 0 and crc.encode() not in crcs]
    allowed = bool(bad) and bad == [len(pictures) - 1]
    return pictures, bad, not bad or allowed


def summarize(demo, trace):
    header, frames, boards, uart, other = None, [], [], bytearray(), []
    for line in trace.read_text(errors="replace").splitlines():
        if line.startswith("# board\t"):
            boards.append(line.split("\t"))
        elif line.startswith("# uart"):
            # `# uart <seconds> <cycle> <text>`, tab or space separated; the text is last.
            rest = line[len("# uart"):].lstrip(" \t")
            parts = re.split(r"[ \t]", rest, maxsplit=2)
            uart += unescape(parts[2] if len(parts) == 3 else rest)
        elif line.startswith("#"):
            other.append(line)
        elif header is None:
            header = line.split("\t")
        else:
            frames.append(dict(zip(header, line.split("\t"))))
    print(f"record: {trace}")
    if frames:
        t = [float(f["t"]) for f in frames]
        cycles = [int(float(f["cycle"])) for f in frames if f.get("cycle") not in (None, "")]
        mhz = [float(f["mhz"]) for f in frames if f.get("mhz") not in (None, "", "-")]
        ms = sorted(float(f["frame_ms"]) for f in frames if f.get("frame_ms") not in (None, ""))
        print(f"frames: {len(frames)} over {t[-1] - t[0]:.1f} s; frame time median {statistics.median(ms):.1f} ms, "
              f"95th percentile {ms[int(0.95 * (len(ms) - 1))]:.1f} ms")
        if cycles:
            print(f"cycles: {cycles[-1]} reached ({cycles[-1] / 25e6:.3f} s of board time)")
        if mhz:
            print(f"rate shown: median {statistics.median(mhz):.2f} MHz, max {max(mhz):.2f} MHz")
    if boards:
        last = boards[-1][-1]
        patterns = sorted({b[3].split()[-1] for b in boards})
        wrong = sum(1 for b in boards if "WRONG" in b[6])
        print(f"board: {len(boards)} LED drawings read back, {wrong} with LEDs drawn wrong; {last}")
        print(f"board: {len(patterns)} distinct LED patterns shown lit (D0..D7): {' '.join(patterns[:24])}"
              + (" …" if len(patterns) > 24 else ""))
    else:
        print("board: no `# board` lines (no 3D board drawn?)")
    pressed = presses(other)
    print(f"buttons: {pressed or 'none pressed'}")
    # `# monitor frame N WxH crc XXXXXXXX ...`: the frames the HDMI receiver decoded from the
    # pads and the window painted (gui/src/monitor_view.rs).
    monitor = {}
    for line in other:
        m = re.match(r"# monitor frame (\d+) (\d+x\d+) crc ([0-9A-Fa-f]{8})", line)
        if m:
            monitor[int(m.group(1))] = (m.group(2), m.group(3).upper())
    out = ROOT / "target/gallery" / demo
    if uart:
        (out / "gui_uart.txt").write_bytes(bytes(uart))
        _, c = check.load_check(demo)
        ok, message = check.compare(c["expected"], bytes(uart), c.get("marker", ""))
        print(f"uart: {len(uart)} bytes in the window's console, written to {out / 'gui_uart.txt'}")
        print(f"uart: against expected/uart.txt: {'PASS' if ok else 'differs'}: {message.splitlines()[0]}"
              + ("" if ok else " (expected unless the window's script types check.json's input)"))
    else:
        print("uart: no `# uart` lines in the record")
    events = [line for line in bytes(uart).decode("latin-1").splitlines()
              if re.fullmatch(r"SW [01]{4}|PLAYER", line)]
    print(f"controls: {', '.join(events) if events else 'no SW or PLAYER lines'}")
    pictures, bad, oled_ok = oled_pictures(other, bytes(uart))
    if pictures:
        counts = [n for _, n, _ in pictures]
        completed = sum(n > 0 for n in counts)
        unmatched = set(bad)
        (out / "gui_oled.txt").write_text("".join(
            f"{serial} {n} {crc} {'unprinted' if i in unmatched else 'printed'}\n"
            for i, (serial, n, crc) in enumerate(pictures) if n > 0))
        print(f"oled: {len(pictures)} pictures painted, frame counts {min(counts)}..{max(counts)}; "
              f"{len(pictures) - completed} before the first completed frame; "
              f"{completed - len(bad)} of {completed} painted data CRCs among the design's printed CRCs; "
              f"{'PASS' if oled_ok else 'FAIL'}"
              + (f"; first exception: picture {pictures[bad[0]][0]}, frames {pictures[bad[0]][1]}, "
                 f"data crc {pictures[bad[0]][2]}"
                 + (" (last picture allowed)" if oled_ok else "") if bad else ""))
    if monitor:
        sizes = sorted({s for s, _ in monitor.values()})
        print(f"monitor: {len(monitor)} decoded frames painted, numbers {min(monitor)}..{max(monitor)}, "
              f"sizes {' '.join(sizes)}")
        (out / "gui_monitor.txt").write_text("".join(f"{n} {s} {c}\n" for n, (s, c) in sorted(monitor.items())))
        # Lines the design printed with each frame's CRC (`... C<8 hex>` after a frame number), from
        # the window's console or, failing that, from the emulator's or the board's transcript.
        printed = {}
        for source in ("gui_uart.txt", "emulator_uart.txt", "board_uart_1.txt"):
            path = out / source
            if path.exists() and not printed:
                for text in path.read_text(errors="replace").splitlines():
                    f = re.match(r"F([0-9A-Fa-f]+)\b.*\bC([0-9A-Fa-f]{8})\b", text.strip())
                    if f:
                        printed[int(f.group(1), 16)] = (f.group(2).upper(), source)
                    # riscv_terminal's `screen` command: the CRC of a frame, without a number.
                    g = re.match(r"screen crc ([0-9A-Fa-f]{8})\b", text.strip())
                    if g:
                        printed[-1 - len(printed)] = (g.group(1).upper(), source)
        both = sorted(set(monitor) & {n for n in printed if n >= 0})
        if both:
            bad = [n for n in both if monitor[n][1] != printed[n][0]]
            print(f"monitor: {len(both)} frames also printed by the design ({printed[both[0]][1]}): "
                  f"{len(both) - len(bad)} CRCs equal by frame number, {len(bad)} differ"
                  + (f" (first {bad[0]}: decoded {monitor[bad[0]][1]}, printed {printed[bad[0]][0]})" if bad else ""))
        if printed:
            # The receiver numbers the frames it decoded itself; whatever the numbering, every
            # decoded frame's CRC must be one the design printed.
            crcs = {c for c, _ in printed.values()}
            found = [n for n in monitor if monitor[n][1] in crcs]
            print(f"monitor: {len(found)} of {len(monitor)} decoded CRCs are among the "
                  f"{len(crcs)} the design printed ({next(iter(printed.values()))[1]})")
            seen = {c for _, c in monitor.values()}
            print(f"monitor: {len(crcs & seen)} of the {len(crcs)} printed CRCs were decoded from the pads")

    return oled_ok


def summarize_tour(trace, script):
    """The tour's record, split at its `# design` lines: one summary per demo."""
    opened = [line.split(None, 2)[2].strip() for line in script.read_text().splitlines()
              if len(line.split(None, 2)) == 3 and line.split(None, 2)[1] == "open"]
    header, segments, current = None, [], None
    for line in trace.read_text(errors="replace").splitlines():
        if line.startswith("# design\t"):
            current = {"design": line.split("\t", 1)[1], "frames": 0, "cycle": 0, "mhz": [],
                       "script": [], "uart": 0, "board": 0, "wrong": 0, "monitor": 0,
                       "buttons": []}
            segments.append(current)
        elif not line.startswith("#"):
            if header is None:
                header = line.split("\t")
            elif current is not None:
                f = dict(zip(header, line.split("\t")))
                current["frames"] += 1
                if f.get("cycle", "") not in ("", "-"):
                    current["cycle"] = max(current["cycle"], int(float(f["cycle"])))
                if f.get("mhz", "") not in ("", "-"):
                    current["mhz"].append(float(f["mhz"]))
        elif current is None:
            continue
        elif line.startswith("# script\t"):
            current["script"].append(line.split("\t", 2)[2])
        elif line.startswith("# uart"):
            parts = re.split(r"[ \t]", line[len("# uart"):].lstrip(" \t"), maxsplit=2)
            current["uart"] += len(unescape(parts[2] if len(parts) == 3 else ""))
        elif line.startswith("# board\t"):
            current["board"] += 1
            current["wrong"] += "WRONG" in line.split("\t")[6]
        elif line.startswith("# monitor frame"):
            current["monitor"] += 1
        elif line.startswith("# button\t"):
            current["buttons"].append(line)
    print(f"record: {trace}")
    good = True
    for n, seg in enumerate(segments):
        name = pathlib.Path(seg["design"]).parent.name or seg["design"]
        expected = opened[n] if n < len(opened) else None
        right = expected is not None and seg["design"].endswith(expected)
        good &= right
        own = [c for c in seg["script"] if not c.startswith(("open ", "close"))]
        mhz = f"{statistics.median(seg['mhz']):.2f} MHz" if seg["mhz"] else "-"
        # Every scripted press must have reached this demo as a button going down.
        scripted = sorted(int(c.split()[1]) for c in own
                          if c.startswith("button ") and c.split()[-1] == "down")
        recorded = sorted(b[0] for b in map(button_line, seg["buttons"]) if b and b[1])
        good &= scripted == recorded
        print(f"{name}: {'opened as scripted' if right else 'NOT the scripted design: ' + str(expected)}; "
              f"{seg['frames']} frames, {seg['cycle']} cycles, {mhz}; {len(own)} commands of its own "
              f"after its design line; buttons {presses(seg['buttons']) or 'none'}"
              f"{'' if scripted == recorded else ' (the script pressed ' + str(scripted) + ')'}; "
              f"console {seg['uart']} bytes; 3D board {seg['board']} drawings, "
              f"{seg['wrong']} with an LED drawn wrong; monitor {seg['monitor']} frames")
    if len(segments) != len(opened):
        print(f"tour: {len(segments)} designs in the record, {len(opened)} opened by the script")
        good = False
    return good


def run_gallery(args, ap):
    if args.demo == ["tour"]:
        script = ROOT / "examples/gallery/tour.script"
        trace = args.trace or ROOT / "target/gallery/tour_trace.tsv"
        if not args.no_run:
            env = dict(os.environ)
            env["ECP5_GUI_SCRIPT"] = str(script)
            env["ECP5_GUI_TRACE"] = str(trace)
            # The script's `open` paths are relative to the repository's root.
            subprocess.run([str(ROOT / "target/release/ecp5_gui")], env=env, cwd=ROOT, check=True)
        sys.exit(0 if summarize_tour(trace, script) else 1)
    demos = TOUR if args.demo == ["all"] else args.demo
    if args.trace and len(demos) > 1:
        ap.error("--trace takes one demo")
    skipped = []
    good = True
    for demo in demos:
        out = ROOT / "target/gallery" / demo
        if not (out / "top.config").exists():
            print(f"{demo}: not built; run examples/gallery/check.py {demo} --build", file=sys.stderr)
            skipped.append(demo)
            continue
        if len(demos) > 1:
            print(f"== {demo}")
        trace = args.trace or out / "gui_trace.tsv"
        if not args.no_run:
            env = dict(os.environ)
            env["ECP5_GUI_SCRIPT"] = str(ROOT / "examples/gallery" / demo / "gui.script")
            env["ECP5_GUI_TRACE"] = str(trace)
            subprocess.run([str(ROOT / "target/release/ecp5_gui"), str(out / "top.config")], env=env, check=True)
        good &= summarize(demo, trace)
    # A tour with gaps is not a whole tour.
    if skipped or not good:
        if skipped:
            print(f"skipped, not built: {' '.join(skipped)}", file=sys.stderr)
        sys.exit(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("demo", nargs="+", help="demo folder names, or `all` for the tour")
    ap.add_argument("--no-run", action="store_true")
    ap.add_argument("--trace", type=pathlib.Path)
    args = ap.parse_args()
    if args.no_run:
        run_gallery(args, ap)
        return
    try:
        console_unlocked()
        label = "gallery-" + "-".join(args.demo)
        with screen_lock(label, who=os.environ.get("ECP5_SESSION") or "demos"):
            run_gallery(args, ap)
    except ValueError as error:
        ap.exit(2, f"{error}\n")


if __name__ == "__main__":
    main()
