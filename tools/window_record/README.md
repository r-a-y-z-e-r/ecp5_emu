# Board window records

This directory keeps the board-specific checks for the GUI window record.
Python 3.9 or newer and its standard library are sufficient for the summary.

- `life_full_board.script` checks full-chip Life under
  `target/reference/life_full`: rates 4, 100, max and real, button 1, and
  the 3D, top and bottom board views. It quits at 88 seconds since ready.
- `board_summary.py RECORD.tsv SCRIPT` is self-contained. It summarizes the
  GPU's `# board` LED/OLED/HDMI judgments and prints every `# layout` line.

Use `tools/gui_record.py` for window runs, preflight checks, frame summaries
and A/B comparisons. It owns the main checkout's `out/screen.lock`: it takes
the lock exclusively and releases it when the run ends. Let it manage that
lock for these commands.

## Run the board A/B

From the repository root, with the required GUI binary, image reader, board
model and reference design already built, run ABBA: with, without, without,
with. Numbered labels keep the four records distinct. Ensure an inherited
`ECP5_GUI_NO_BOARD` does not disable the board in the with runs:

```sh
unset ECP5_GUI_NO_BOARD
python3 tools/gui_record.py run life_full tools/window_record/life_full_board.script board_with_1
python3 tools/gui_record.py run life_full tools/window_record/life_full_board.script board_without_2 --env ECP5_GUI_NO_BOARD=1
python3 tools/gui_record.py run life_full tools/window_record/life_full_board.script board_without_3 --env ECP5_GUI_NO_BOARD=1
python3 tools/gui_record.py run life_full tools/window_record/life_full_board.script board_with_4
```

Each run writes `out/records/<label>.tsv` and `<label>.out` and prints its
frame summary. Choose fresh labels for another A/B; `gui_record.py` reuses
the output paths for an existing label.

Compare frame FPS, frame milliseconds, lag, misses and board WRONG counts
with `gui_record.py sum`, pairing the two halves of the ABBA:

```sh
python3 tools/gui_record.py sum out/records/board_with_1.tsv out/records/board_without_2.tsv
python3 tools/gui_record.py sum out/records/board_with_4.tsv out/records/board_without_3.tsv
```

For the detailed `# board` LED/OLED/HDMI lines and `# layout`, run the board
summary on each label:

```sh
for label in board_with_1 board_without_2 board_without_3 board_with_4; do
    python3 tools/window_record/board_summary.py "out/records/$label.tsv" tools/window_record/life_full_board.script
done
```

Keep the display awake, console unlocked and GUI window uncovered throughout.
A run with the display asleep or the window covered is **void for timing**.
Any frame gap longer than 0.5 seconds also makes the run void; inspect
`frame_ms` and consecutive frame timestamps. Discard a run known to have
been obscured or asleep even if its summary looks plausible. Preflight
checks do not establish that the display stayed visible for the whole run.

## Board summary measurements

Board phases are `[script command time + 1 second, next command time)`;
intervals shorter than two seconds are skipped. Every command ends a phase,
even a button or diagnostic command. Names retain the rate, values display,
zoom, run/pause/step and board view. The final command's interval ends at the
last record timestamp. These settling intervals are specific to this board
summary; `gui_record.py sum` uses its own recorded command/state boundaries.

Each row reports drawings checked; wrong and unseen LEDs; drawings with new
LED values; median/max milliseconds since the previously drawn values; the
maximum frames after the new values came; OLED module and HDMI cable
asked/drawn counts; and the last cumulative options-wrong count. Unseen
means both the drawn-lit and drawn-dark bit are zero. OLED drawn counts
refer to the module independently of its screen or lit pixels.

Unavailable measurements and missing optional columns print `-`, so older
board records without option details still work. The frame header requires
`t`. Other comments are ignored except `# layout`, which is printed unchanged.
Scripts that switch designs or records whose timestamps restart must be
split into separate measurements. Formats are defined in `gui/src/trace.rs`,
`gui/src/app.rs` and `gui/src/board_view.rs` (`judge`).

## Header J1/J2 panel

The real-window check is pending in a single screen batch before
publication. The tests below use synthetic records only. Build the driven
reference from the repository root, before the screen batch:

```sh
examples/ulx3s/build.sh examples/ulx3s/header_probe/driven target/reference/header_probe_driven
```

`header_probe.script` uses existing window commands only. It selects the top
board view, runs with the real-time rate cap for 12 seconds, then max until
29 seconds, pauses and quits at 30 seconds since ready. The real-time setting
is a cap of 25 MHz, not a promise of that achieved speed. The Header panel
opens automatically; no new script commands or GUI code are needed.

Run the two records through the same screen-lock owner as the board A/B.
Ensure the 3D board is enabled. The first run explicitly clears inherited
header overrides; the second drives **GN0=0** from outside in the emulator:

```sh
unset ECP5_GUI_NO_BOARD
python3 tools/gui_record.py run header_probe_driven tools/window_record/header_probe.script header_pulls --env ECP5_HEADER=
python3 tools/gui_record.py run header_probe_driven tools/window_record/header_probe.script header_external --env ECP5_HEADER=GN0=0
```

GN0 is the design's echoed input, J1 pad 5 / ball C11. Its configured pull is
UP, so `GN0=1` would leave the input, LED0 and UART echo unchanged. **GN0=0**
provides the clearer comparison: the panel should show level 0 and `ext=0`
(the 0 control selected), LED0 off, UART `E0`, and level word
`55555555555554`; the released run has level 1, `ext=z`, LED0 on, `E1` and
`55555555555555`. These are emulator inputs; this kit does not load hardware.
Only GP0..GP3 are driven by the design; all other GP inputs are DOWN and GN
inputs UP except the explicit GN0 override.

Keep the Header panel visible and unobscured, with the display awake and
console unlocked as in the board A/B. Visually check both 2x20 grids, greyed
power pins and input controls during the batch. The recorded notes alone do
not prove that the panel was visible, that its layout was correct, or that
its controls were clicked. This script sets an initial input through the
existing environment option; it does not test a mouse interaction.

Summarize both records:

```sh
python3 tools/window_record/header_summary.py out/records/header_pulls.tsv
python3 tools/window_record/header_summary.py out/records/header_external.tsv --external GN0=0
python3 tools/gui_record.py sum out/records/header_pulls.tsv out/records/header_external.tsv
```

`header_summary.py` is self-contained and uses only the standard library.
It prints PASS/FAIL per check and exits 1 if any check fails (0 on success):

- Both recorded running phases must exist, real then max. After **2 seconds
  of wall-time settling** in each phase, all sampled Header statuses must
  give GP0..GP3 **1000/2000/5000/10000 Hz within 1%** and **25/50/75/10% duty
  within 1 percentage point**. This allows window-endpoint and three-decimal
  rounding effects; it is not a hardware or analog accuracy specification.
- All **52 other header inputs** must read their pulls or the GN0 override,
  with the corresponding `ext=z/0/1`. Every note must name all 56 pins exactly
  once. Notes describe the following frame and their last state carries
  forward until another note changes it, including across rate changes.
- A valid `# board` drawing must be present. Each settled phase also needs
  visible, correct GPU LED0 readback matching the GN0 echo. The most recent
  drawing at or before the settled start carries forward until replaced;
  every later drawing inside the phase is checked too. An out-of-sight
  LED is not evidence of its level; missing drawings fail explicitly.
- UART bytes are decoded from app.rs's escaped `# uart` chunks and joined
  across notes. Only **newline-terminated** lines count. The internal period
  counters, per-second pad rising/high counts, full input word and E0/E1
  must match exactly, starting with the first full-second report. At least
  one complete report is required; an incomplete final report is counted
  as trailing bytes and ignored.

If a slow run has not advanced through a full emulated second and completed
its UART report, the summary fails. Extend the timed phases and use fresh
labels for another run; elapsed wall time alone is not evidence of emulated
time. The board A/B's visibility and frame-gap rules still apply to timing;
a PASS here does not override a void timing record or replace visual review.

Run the parser's synthetic unit tests and help without opening a window:

```sh
python3 -m unittest discover -s tools/window_record -p 'test_header_summary.py' -v
python3 tools/window_record/header_summary.py --help
```
