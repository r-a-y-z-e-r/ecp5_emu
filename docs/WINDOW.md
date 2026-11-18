# The emulator's window

The emulator's window code is in [gui/src](../gui/src).

## Controls

The top toolbar is one 46 pt grouped row: navigation, build (for projects),
transport, cycle/rate and views. Design ▾ is shared by the plain config viewer
and both Project/Chip tabs. It starts with **New project…** and
**Start from a demo…**, disabled when creation is unavailable with
“Wait for the current build or emulation to finish”.
Then come Open…, Recent, Demos, Close and Theme > System / Light / Dark, followed
by the task menus Build (projects only), Run, View and Help. The design name
appears once in navigation, truncating with its full name/path on hover.
The visible **New project** button follows Design ▾ and Peripherals, with an icon
and label. It uses the same creation action, availability and disabled reason.

Project | Chip selects the workspace; these tabs also appear in Design > View.
The main half of Build runs an incremental build (Cmd/Ctrl+B); its arrow contains
Build, Rebuild, Stop build and Emulate, mirrored in Design > Build. Emulate uses
the project's availability check, including its build outputs on disk, and queues
the successful result when a build is running. Build status and a temporary Stop
action appear at the bottom on both tabs; detailed output stays in the output pane.
Plain configs have no project tabs, build controls or Build submenu.

Back, one fixed-width Run/Pause toggle and the Step split button form the
transport group. Back (←) and Step (→) use the selected amount. Step's main half
advances and pauses; its arrow selects 1, 1k, 1M or Custom…. Custom… opens a
labelled numeric popup with Apply and Cancel; applying sets the step without
advancing. No unused numeric fields occupy the resting row. The original
readiness, pause, earliest-cycle and keyboard-focus guards apply.

Click the cycle readout to type a destination. Enter seeks and pauses; Escape
cancels. Invalid input keeps the editor open with an explanation. Design > Run >
Go to cycle… opens the same editor. The readout contains a bounded **+N ahead**
badge only when N > 0; hover shows the exact count and explains replaying the same
inputs until a new input starts a run from that cycle. The rate combo is the target
rate, editable while running: 1, 4, 100 or 10k cycles/s, real time (25 MHz), as fast
as possible, or Custom…. Custom rate uses a popup with one cycles/s unit and
Apply/Cancel, preserving fractions and suffixes such as 0.5 and 2.5k. Design > Run
mirrors transport, step size and all rate choices.

Views sit at the right in this order: Values, the exclusive Wires | Tiles
selector, Waves and Critical path. Values colours wires, LUTs and flip-flops;
Wires shows wires/sites and Tiles shows tiles/sites without wires. Waves retains
W over a wire and click-to-seek. Critical path shows nextpnr's estimate, with its
clock selector and timing report. Design > View mirrors these controls, including
clock selection. At 1100 pt, the name is 64 pt, the cycle readout is 152 pt with
abbreviated counts and exact values on hover, and view controls use vector icons
with tooltips and accessible names. Widths below 1500 pt use this compact
treatment. Below 1100 pt, view actions remain in Design >
View. At 1500 pt the view icons have labels.

The toolbar is verified headlessly at 1500 × 950 and 1100 × 800 in both themes
and hosts. No window check is claimed. The `v010_toolbar` batch entry is prepared
for a later window run; its `toolbar` criterion checks the `# toolbar` rectangles
for fit and non-overlap at both sizes.

## Opening a design

The **Peripherals** menu beside **Design** reopens or closes each peripheral window;
its checkboxes show the open state and are enabled once the model is seen for the
current design. Panels still open automatically on first sight. EDID is inside
the HDMI Monitor window.

### What happens while a design opens

[src/live.rs](../src/live.rs) announces these stages:

1. Loading the device model
2. Reading the configuration
3. Reading the design's pins
4. Compiling the execution plan
5. Drawing the die's fabric (alongside layout geometry)
6. Starting fast mode (clang), if it is still running after the drawing work
7. Choosing fast mode's settings…; on a re-open with a remembered choice,
   Choosing fast mode's settings: 1 of about 1 (N threads, folding F), with
   more trials if the remembered settings run too slowly
8. Starting with N threads, folding F; tuning in the background
9. Ready, then Paused

Fast mode's program is generated and built alongside the layout geometry and
overlay. Its stage only appears if that work finishes first. The settings
check and tuner preparation wait for all three workers to finish, so their
measurements do not compete with the loading work. On a first open without
remembered settings, fast mode starts from a guess based on the plan's gate
count. The settings check keeps that program without a trial, then the tuner
tries other settings during the run. On a re-open it builds the remembered
program once and reuses it for the settings trial and the run.

The settings choice is skipped with `FAST_CHOOSE=0`. `Ready` is immediately replaced
by `Paused` when the design is handed to the window, so the window may only see
`Paused`.

The window starts the chip worker when it opens. While a design loads, it draws
the die as soon as `live::chip_fabric` supplies it from the chip cache, without
waiting for the design's overlay. On a cache miss the worker first builds the
fabric, so the die is not necessarily visible immediately. The stage called
“Drawing the die's fabric (alongside layout geometry)” waits for that fabric if
necessary and builds or reads the design's overlay, while a separate worker
builds or reads the layout geometry. It also prepares the design's names and
other map data.

Over the die, [gui/src/loading.rs](../gui/src/loading.rs) draws a card with the
current stage, its elapsed time, the total elapsed time and completed stages.
When this design has a previous successful load, its stage times supply the
weights for a progress bar and an estimate of the time left. A stage that runs
longer than before does not consume the budget of later stages.
[gui/src/start.rs](../gui/src/start.rs)'s `LoadTimes` keeps the last successful
load for each of at most 32 design paths in `load-times.txt`, beside `recent.txt`.
On macOS these files are in `$HOME/Library/Application Support/ecp5_emu/`; on
Windows, `%APPDATA%/ecp5_emu/`; elsewhere, `$XDG_CONFIG_HOME/ecp5_emu/`, falling
back to `$HOME/.config/ecp5_emu/`.

### What is cached on disk between openings

The cache directories below use Rust's system temporary directory
(`std::env::temp_dir()`), written as `$TMPDIR` here, as on macOS. These caches
avoid repeating work; missing or invalid entries are rebuilt.

- **The die's fabric.** [src/fabric.rs](../src/fabric.rs)'s `cached` stores it in
  `$TMPDIR/ecp5_emu_fabric`. Its SHA-256 key contains the cache format version,
  the length and bytes of the architecture's `model_manifest.json`, and the
  executable's SHA-256 digest. Changing any of those selects a new entry; the
  design's configuration is not part of this key. After a write it keeps the
  newest three entries by modification time. `ECP5_FABRIC_CACHE=0` disables reads
  and writes.
- **The decoded configuration.** [src/image.rs](../src/image.rs) stores CRAM,
  EBR data and service metadata in `$TMPDIR/ecp5_emu_image`. Its SHA-256 key
  contains the cache magic and format version, source digest and kind,
  executable digest, database file digests, and the native image reader and
  linked Trellis library digests. `ECP5_IMAGE_CACHE=0` or `ECP5_FABRIC_CACHE=0`
  disables reads and writes. Each entry is at most 32 MiB; after a write it
  keeps the newest three entries, bounding the directory to 96 MiB.
- **The design's overlay.** [src/fabric_overlay.rs](../src/fabric_overlay.rs)
  stores it in `$TMPDIR/ecp5_emu_overlay`. Its SHA-256 key hashes the domain
  `ecp5-design-view-v2\0`, cache format version 2 and the plan identity from
  `compile::cache_key`, described below. Changing any of those selects a new
  entry. Without an identity, when the executable cannot be hashed, it builds
  without reading or writing this cache. `ECP5_OVERLAY_CACHE=0` also disables
  reads and writes. After a write it prunes older entries to a default budget
  of 1 GiB; `ECP5_OVERLAY_CACHE_MB` changes that budget in MiB. The newest entry
  always stays, even if it alone exceeds the budget, so 1 GiB is not a hard cap.
  Building uses the performance-core count on macOS when available, capped by
  the available thread count. Otherwise it uses the available thread count,
  or one if that count is unknown.
  `ECP5_OVERLAY_THREADS=N` overrides the worker count; `1` builds sequentially.
  Zero or an invalid value uses the default. The worker count does not change
  the cache key or bytes.
- **The layout geometry.** [src/layout.rs](../src/layout.rs) stores it in
  `$TMPDIR/ecp5_emu_geometry`. Its SHA-256 key uses the same domain, format
  version 2 and plan identity as the overlay. Without an identity, when the
  executable cannot be hashed, it builds without reading or writing this cache.
  `ECP5_GEOMETRY_CACHE=0` disables reads and writes. On cache hits and after
  writes it prunes older entries to a default budget of 256 MiB;
  `ECP5_GEOMETRY_CACHE_MB` changes that budget in MiB. The newest entry always
  stays, even if it alone exceeds the budget. The measured `life_full` entry is
  153,884,716 bytes (146.76 MiB). Geometry and overlay work run alongside each
  other; the overlay uses separately collected logic-use masks rather than
  waiting for the complete geometry.
- **The execution plan, for pure-logic designs only.**
  [src/compile/cache.rs](../src/compile/cache.rs) and
  [src/kernel/cache.rs](../src/kernel/cache.rs), introduced in `fe94613`, store
  the compiled kernel and report in `$TMPDIR/ecp5_emu_compile`. Plans with
  LUTRAM writes, EBR, DSP, PLL, OSC or IOLOGIC controllers are not stored. The
  SHA-256 key contains the cache magic and format version, configuration source
  digest, bound-device fingerprint, executable digest, and the serialized
  compile request and board. The fingerprint binds the architecture manifest,
  source configuration, decoded image and merged configuration catalogue. The
  manifest pins the architecture files and, through the inventory, the Trellis
  database files; the model loader verifies their digests. The request includes
  package, scope, startup, input drives, probes and kernel options; the board
  includes wires and oscillator frequency. Changes to these inputs select a
  new entry. This identity also keys the layout geometry and overlay
  caches, even when the plan itself cannot be stored. `ECP5_COMPILE_CACHE=0`
  or `ECP5_FABRIC_CACHE=0` disables reads and writes. On reads and writes it
  prunes the oldest entries to keep at most three files and 1,000,000,000 bytes
  (1 GB).
- **Fast mode's settings memory.** [src/fast.rs](../src/fast.rs)'s
  `choice_memory` selects `$TMPDIR/ecp5_emu_fast/<hash>.choice`, which keeps
  settings and measured speeds. Since `33db141`, the key hashes the kernel's
  configuration and plan identity, the generator's source, compiler and
  generation flags, performance-core count, which options are pinned by the
  environment, and the base fast-mode options, without generating C. Changes
  to those inputs select a new file; there is no executable digest in this
  key. Watched nets, probe labels, live values and clock history are not part
  of this settings key. Older source-keyed memories can still be read when
  the initial program uses the default options. With no remembered choice,
  `FastSim::start` builds a guess and `choose_options` keeps it without
  generating C again or running trials. The in-run tuner writes the memory
  after a new choice survives ten real run pieces; it can also remember the
  initial program after ten pieces once the other candidates have been
  checked. It uses measured speeds from the run, not a candidate's short
  trial speed. With memory, the warm start builds the remembered program and
  `choose_options` reuses it for one trial; if it reaches at least half its
  remembered speed, that is enough. Otherwise it searches further. After
  trials, `choose_options` writes the measured settings and speeds for next
  time, including when the one trial suffices. `FAST_CHOOSE=0` skips the startup
  choice and its memory reads and writes; the in-run tuner can still write memory.
  `FAST_CHOOSE=full` ignores the previous memory, measures all candidates again
  and writes the new results. See [FAST_MODE.md](FAST_MODE.md) for
  fast mode, including its generated C and library cache in the same directory,
  keyed by source and compiler flags.

A new build whose executable digest changes invalidates the fabric, decoded
configuration, layout geometry, overlay and plan entries. Fast mode's settings
memory survives a rebuild if its own key's inputs stay the same. The five `.bin`
caches also check their format, payload length and checksum before accepting an
entry. Pruning is best effort: files that cannot be read or removed may remain.

### Measured

These are `life_full` measurements on an Apple M3 Max, dev `c82aeb0`, on
2026-11-17. A fresh `TMPDIR` made every cache cold, including clang's output.
The window records are `out/records/load_c82_first.tsv`, followed by 25 s
running at max, then `load_c82_reopen.tsv` and `load_c82_reopen2.tsv`. A
`.choice` file was written during that run, so both re-opens checked the
remembered settings: 10 threads, folding 12.

The times come from the records' `# load` lines, in seconds since the window
opened the design. Stage durations are differences between successive stage
timestamps; `die-shown` is a separate event, not a stage boundary.

| stage | first open | re-open | second re-open |
|---|---|---|---|
| Loading the device model | 0.439 s | 0.267 s | 0.267 s |
| Reading the configuration | 6.531 s | 1.417 s | 1.317 s |
| Reading the design's pins | 0.702 s | 0.650 s | 0.584 s |
| Compiling the execution plan | 14.446 s | 1.449 s | 1.349 s |
| Drawing the die's fabric (alongside layout geometry) | 12.129 s | 1.151 s | 1.147 s |
| Starting fast mode (clang), remaining wait | Finished alongside drawing | 0.683 s | 0.690 s |
| Choosing fast mode's settings… | 0.125 s | 0.105 s | 0.100 s |
| Choosing fast mode's settings: 1 of about 1 (10 threads, folding 12) | No trial | 0.345 s | 0.330 s |
| Starting with 10 threads, folding 12; tuning in the background | 0.219 s | 0.133 s | 0.133 s |
| Whole load, to the run paused | 34.606 s | 6.215 s | 5.931 s |
| The die on screen | 9.687 s | 0.382 s | 0.382 s |

The first open's load output reports 13.803 s for the plan kernel. Layout
geometry took 9.085 s, the fabric overlay 10.687 s and fast startup, including
clang, 9.526 s; those three ran together, so their times do not add up. The
settings check after the overlay took 0.125 s and tuner preparation 0.204 s.
On the re-opens, the settings check and its one trial together took
0.450 s and 0.430 s in the window record. The fast-mode row is only the
remaining wait after drawing, not the whole concurrent startup.

Whole-load times have changed as follows:

- Before the caches (2026-11-16/17): 55–62 s, with one line of text over the map.
- `35b98b1`: 48.7 s first open / 20.2 s re-open.
- `06fe437` + `bd444aa`: 50.2 / 11.4 s.
- `2f44f16` + `ca3ed84`: 46.2 / 10.2 s.
- `fb9cd42`, parallel overlay: 40.1 / 8.9 s.
- `d889e95`, geometry alongside the overlay: 36.5 / 8.8 s.
- `c82aeb0`, fast mode's one generation and remembered choice: 34.6 / 6.2 s.

From `fb9cd42` on, the first-open measurements use a fresh `TMPDIR` and
include a cold clang compile. The earlier first-open measurements did not.

To measure again, make a script containing `5 quit`, which quits five seconds
after the design is ready. Run `tools/gui_record.py run life_full <script> <label>`
twice, with different labels, leaving the caches from the first run for the
second. To compare an uncached first open, start with the cache entries above
absent. The tool writes `out/records/<label>.tsv` and `<label>.out` and sets
`ECP5_LOAD_TIMES=1`. In the record, `# load <seconds> <stage>` gives seconds
since the window opened that design, followed by the stage; differences between
successive stage timestamps give stage durations. `# load die-shown <seconds>`
marks the first drawing of the die. The window output reports the previous
stage's duration when the next stage starts, including stages too brief for a
window frame to see.

### What is left

On a first open, the plan kernel takes about 11–14 s in
[src/compile](../src/compile), and reading the configuration takes about 6 s.
The overlay, geometry and fast mode's cold compile then run together in
about 12 s, sharing the cores. Their individual times cannot be added to
predict the load time.

On a re-open, reading the configuration takes about 1.3 s, the execution
plan about 1.4 s and the fabric stage about 1.2 s. Fast mode has about 0.7 s
left after drawing, followed by about 0.4 s checking the remembered settings.
The whole load takes 5.9–6.2 s, with the die on screen after 0.382 s.
See [src/fast.rs](../src/fast.rs) and [FAST_MODE.md](FAST_MODE.md).

## The map's frame rate

The run's rate is in cycles per second (c/s); the window's frame rate is a
separate measurement. At every run rate, the GPU draws a full-screen colour
pass over the map. It reads a cached image of the top piece of wire at each
pixel, then colours that pixel from the wire's current record, including its
0/1 value when values are on. The image of pieces is drawn again when the view
moves. Changing many values does not require drawing all the pieces again.
Only the records of wires that changed are sent to the GPU each frame.
See [gui/src/gpu_map.rs](../gui/src/gpu_map.rs).

Changes glow over these colours. Close up, the GPU draws quads over the pieces
a change may light, wider where the glow is brighter. From afar, with hundreds
of thousands of pieces glowing, the default uses a glow mask. A compute pass
splats pieces shorter than 24 physical pixels into a buffer with atomic MAX;
longer pieces are drawn as strips with MAX blending. The colour pass reads
their maximum, so activity on a wire covered by others can still show. A
compute pass lists the pieces a change may light; the glow drawing uses that
list. The choice of quads or mask follows the view and the piece count, rather
than the run rate alone.

[gui/src/changes.rs](../gui/src/changes.rs)'s `Motion::of` sets how changes
spread from a net's driver to its far end, then fade at each point reached:

| run | time to spread | time to fade |
|---|---|---|
| A step while paused | 1.5 s | 0.8 s |
| 4 c/s | 0.1 s | 0.3 s |
| 100 c/s | 0.004 s | 0.3 s |
| 1000 c/s | 0.0004 s | 0.3 s |
| As fast as it goes | At once | 0.6 s |

For any set rate `r`, spread takes `min(0.4 / r, 1.5)` seconds and fade takes
`clamp(0.8 / r, 0.3, 0.6)` seconds. Changes keep the pace they had when they
came, even if the rate changes. At fast rates (spread under 1 ms), quiet wires
glow less brightly than busy ones; every reported change still glows.

### Measured

These are `life_full` measurements on an Apple M3 Max, dev `2a0f011`, on
2026-11-17, from the window's own records. The first seven rows are from
`out/records/vsync_splat24.tsv`, made with
[tools/gui_scripts/vsync.script](../tools/gui_scripts/vsync.script), in the
order recorded. At max, the run reached about 204 kHz.

| rate | 0/1 | fps |
|---|---|---|
| 4 c/s | off | 59.4 |
| 4 c/s | on | 59.8 |
| 100 c/s | on | 59.8 |
| max | on | 58.7 |
| max | off | 58.4 |
| max | on | 58.9 |
| 1000 c/s | on | 59.6 |
| max, long run before 45 s | on | 58.2 |
| max, long run from 45 to 60 s | off | 58.0 |
| max, long run from 60 to 76 s | on | 58.3 |

The last three rows are from `out/records/splat_default.tsv`: about 75 seconds
running as fast as it goes, including the design's busier phase from about
18 seconds. Its script turns values on at 0 s, sets max at 0.5 s, runs at 1 s,
turns values off at 45 s and on at 60 s, then quits at 76 s. These times are
since the design was ready, not since Run.

Misses lit/value were 0/0 in every row. In
[gui/src/trace.rs](../gui/src/trace.rs), a lit miss means a reported change on
a wire in sight was not drawn glowing by its deadline; a value miss means
the drawing still did not show its new value with 0/1 on. The deadline allows
the change's spread time plus 0.25 s. Every change the window learns of in
sight enters this check, rather than a sample of wires. Repeated changes
while one is pending keep the first arrival and update the expected value
and deadline, so this does not check every emulated clock edge separately.
Wires that leave the view are not judged. Values covered by other wires at
every pixel are counted as `value_unseen`, not as successes or misses; in
top-wire-only glow mode, covered glow is likewise `glow_unseen`. Zero misses
does not mean every wire's value was visible at fit.

To measure again, with the display awake and unlocked and the tool's screen
lock free, use [tools/gui_record.py](../tools/gui_record.py):

```sh
tools/gui_record.py run life_full tools/gui_scripts/vsync.script vsync_again
tools/gui_record.py sum out/records/vsync_again.tsv
```

`run` opens the window, sets its script and record paths, writes
`out/records/<label>.tsv` and `<label>.out`, and prints a summary. `sum` reads
existing records without opening a window; give it two paths for an A/B
comparison. It splits at script commands and changes of running, rate or
values; fps is frames divided by the sum of their frame times.
`ECP5_SPLAT_MAX` overrides the physical-pixel threshold at startup. For an A/B,
run the same script with different labels and `--env ECP5_SPLAT_MAX=1.5` or
`--env ECP5_SPLAT_MAX=24`. Without an override the threshold is 24.

## The board's switch SW1

The four levers are numbered 1–4, for `sw[0]`–`sw[3]`; only those whose inputs
the design reads are clickable. Clicking toggles a lever. It moves on screen
at once, while [src/live.rs](../src/live.rs)'s runner queues the input for the
next simulation instant (after any inputs already queued). While paused, the
LEDs follow when the next step applies it. Going back restores the switch
positions recorded there and drops inputs not yet applied. Seeking also
clears the window's pending lever positions; opening or closing a design
clears them too.

Without the 3D board, the window provides four SW1 toggles beside the LED and
button rows, with unused inputs disabled. Both forms use the same requests.
`ECP5_SWITCHES` sets the start positions: exactly four 0/1 characters, with
`sw[0]` first. For example, `ECP5_SWITCHES=1010` starts levers 1 and 3 ON;
the default is all OFF. ON reads high; OFF reads the pad's configured pull
(DOWN gives 0, UP gives 1). No or unknown pull uses 0 in the runner, whose
fast-mode input pads accept Boolean levels. See
[examples/ulx3s/switches/README.md](../examples/ulx3s/switches/README.md).

## Between the source and the chip

The window uses the `src` attributes that yosys leaves on net names and cells
and nextpnr keeps in the routed JSON. [src/sources.rs](../src/sources.rs)
accepts relative project inputs only, discarding library paths and files
outside the project's input list. In a plain window, it instead accepts
relative files found beside the design's configuration. The source index is
prepared after the design is ready. See
[gui/src/project.rs](../gui/src/project.rs) for the project input list and
[src/live.rs](../src/live.rs) for loading and indexing.

Net names supply declarations; the cells driving their bits supply assignments.
The source menu calls these “declared in” and “assigned in”, with assignments
first. A port can also get its declaration from the IO cell where nextpnr
moved its source attribute. Several expressions on the same line share a menu
choice within each kind. A placed cell's own source locations are available
from its outline on the map.

After emulating the project, click a token in the editor to keep its nets lit
on the chip. The token's name in that file wins, even if its declaration is on
another line; a bus name can select all its routed bits. If the name does not
match, the index considers only spans starting on the clicked line: first the
narrowest covering the column, otherwise the narrowest on that line. It does
not select a surrounding multi-line block that starts earlier. With no match,
the status says `no net at FILE:LINE:COL`. A known name with no routed net
instead says it has no net on the chip, identifying a constant or a net inside
a cell. Coordinates are one-based characters. The editor uses its current
buffer, including unsaved edits; see
[gui/src/project/editor.rs](../gui/src/project/editor.rs).

“Watch in Waves” adds the probed nets as watches, with a source register's
bits shown individually. The map's right-click menu offers “Show in source”
for a wire or placed cell. Its default is the first assignment, falling back
to a declaration; “Source locations” offers the other choices. The project
window opens the file and selects the span in its editor. A plain window
shows the `file:line` in the menu and copies it to the clipboard when chosen.
See [gui/src/fabric_view.rs](../gui/src/fabric_view.rs) and
[gui/src/app.rs](../gui/src/app.rs).

Right-click a Waves row's name for “Show on the chip” or “Show in source”.
The first keeps the row's nets lit on the fabric map and switches a project
window to Chip. A bus row selects all its watched bits, including when the
bus is expanded. The second uses the same assignment-first source choices
as the map, opening the project's editor or copying `file:line` in a plain
window. See [gui/src/waves.rs](../gui/src/waves.rs).

When “Show in source” is unavailable, the map and Waves give one of three
reasons, as a disabled item's tooltip and as text:

| state | reason |
|---|---|
| No index has arrived | `design has no source index yet` |
| The index has no accepted locations | `no project source matched the build's src attributes`; in a plain window, `no source files found for this design (looked beside its top.config)` |
| The index has locations, but this net has none | `no source line for this net (synthesis left it only a library location)` |

The project warns `built from an older FILE: locations may be off` when the
editor buffer is dirty or the source file's modification time is newer than
the routed JSON's. Probing still uses the loaded build's locations; source
jumps give the same warning. The check uses timestamps and unsaved edits. See
[gui/src/project.rs](../gui/src/project.rs).

## The critical path

“Critical path” toggles the path drawing and the “nextpnr timing report”
panel beside the map. [src/timing_path.rs](../src/timing_path.rs) reads
nextpnr 0.11.1's printed report from `nextpnr.log`, beside the routed JSON.
The version comes from that JSON's `creator`.
The check accepts version strings starting with `nextpnr-0.11.1`, including
build suffixes; other or unrecognised versions report
`critical path: nextpnr VERSION not supported`. A missing log, absent path
or malformed timing block supplies no path. Parsing and mapping happen after
ready; the panel shows `Waiting for nextpnr.log…` until publication. See
[src/live.rs](../src/live.rs).

The map draws the selected path's wires and cell outlines with numbered
hops matching the panel's rows. Cell hop numbers belong to the source cell's
outline; routing hop numbers sit halfway along the drawn route, measured by
its length. Where route pieces are unavailable, tile endpoints can supply a
dashed connection. Off-screen and unmapped hops still get a badge inside the
map, and numbers sharing an outline can share a badge. See
[gui/src/fabric_view.rs](../gui/src/fabric_view.rs).

The panel selects a clock or cross-domain path when several are available.
It lists Fmax, the constraint and slack where available, total delay split
into logic and routing, then each hop's type, delay, cumulative delay, name
and source line. These delays are nextpnr's estimate, not measurements by
the emulator. Cross-domain paths have no Fmax, constraint or slack. Click a
hop to centre the map on it; right-click for “Show in source” when a source
location is available. Toggling the path alone does not move the map. See
[gui/src/app.rs](../gui/src/app.rs).

In the project Report's Timing table, “Show on the chip” opens the last
successful build in the emulator and selects that clock's critical path.
It can therefore replace an older design already running in Chip. See
[gui/src/project/output.rs](../gui/src/project/output.rs) and
[gui/src/project.rs](../gui/src/project.rs).

## The tour

“Take the tour” and `--tour` open the `blinky_cycle` project and drive these
six steps. Each step waits for observed window state, with a minimum reading
time and a deadline; elapsed time alone cannot advance it. A failure or
timeout ends the tour at that step. The actions are in
[gui/src/project.rs](../gui/src/project.rs); the waits and captions are in
[gui/src/project/tour.rs](../gui/src/project/tour.rs).

| step | action and evidence needed | minimum / deadline |
|---|---|---|
| 1. Build | Build the project; wait for a successful build, or the missing-tool fallback below. | 0 / 90 s |
| 2. Chip | Emulate the built design, or show the prebuilt one; wait for the ready design's die to be drawn in Chip. The caption changes when loading finishes. | 3 / 120 s |
| 3. Run | Run at 4 cycles/s; wait for that running state and newly changed wires drawn lit. | 8 / 16 s |
| 4. Source | Open `top.v`, find `count <= count + 1`, select and probe `count`; wait for nets to be found and the editor to be drawn. | 6 / 15 s |
| 5. Back | Return to Chip, wait for a new map drawing, mark a visible probed wire for 1.5 s, then use its “Show in source” action; wait for the editor to show the jump. | 6 / 15 s |
| 6. Waves | Return to Chip and watch the probed counter bits; wait for Waves rows to be drawn. | 8 / 45 s |

Waves also waits until 56 seconds have passed since step 1 began, so a cached
load still leaves time to follow. A final caption lasts four seconds, then
the user has control. A pointer-button press, key press or touch start stops
the tour before that input is passed on; moving the pointer alone does not.
The card also has a Stop button.

Only a build failure reporting a missing `yosys`, `nextpnr-ecp5`, `ecppack`
or `ecpbram` executable permits the fallback. The tour says the FPGA toolchain
is not installed and opens the prebuilt design, leaving that caption up for
at least two seconds. Other build failures stop it. The fallback requires
both `top.config` and `top_routed.json` in the prebuilt directory.

Before step 1, [gui/src/reader.rs](../gui/src/reader.rs) checks the native
image reader, as it does for other design openings. If missing, a shared
background job runs `python3 tools/build_image_reader.py --output <reader path>`
from the repository root. The tour waits on an image-reader card without a
numbered step; this wait does not use the build's deadline. Success starts
step 1. Failure stops before it, showing the builder's last error and
`See README Requirements.`; it does not try the prebuilt design to bypass a
missing reader.

The tour project is found as follows:

1. If `ECP5_TOUR` is set, use that folder alone; it overrides the search.
2. Otherwise try `tour/blinky_cycle` beside the folder named by
   `ECP5_ARCHITECTURE`.
3. Try `../Resources/tour/blinky_cycle`, then `../share/tour/blinky_cycle`,
   relative to the executable's directory.
4. Try `examples/ulx3s/blinky_cycle` in the checkout.

A candidate needs `ecp5.toml`, `top.v` and `top.lpf`. Its prebuilt directory
is `prebuilt` when that contains `top.config`, otherwise
`target/reference/blinky_cycle` in the checkout. A read-only project is
copied to `tour/blinky_cycle` beside `recent.txt`, along with the prebuilt
configuration and routed JSON; an existing source copy there is reused.
See [gui/src/project/tour.rs](../gui/src/project/tour.rs).

## Scripts and the window's record

`ECP5_GUI_SCRIPT=<file>` drives the window from a text file. Each line starts
with seconds since the design was ready, then a command, for example
`0 rate 4` and `1 run`. `#` starts a comment outside quoted JSON strings. Commands run by time between
`open`/`build-demo`/`close` commands, keeping file order across those boundaries;
after `open` or `build-demo`, later times restart when the new design is ready,
waiting for the build as needed, and after `close`
they restart on the screen with no design. The format is described in
[gui/src/trace.rs](../gui/src/trace.rs); the commands are handled in
[gui/src/app.rs](../gui/src/app.rs).

| command | meaning |
|---|---|
| `rate N`, `rate max`, `rate real` | Set cycles per second, run as fast as possible, or use the board clock's real-time rate. |
| `run` | Start or resume the run. |
| `pause` | Pause the run. |
| `step N` | Advance N cycles, then pause. |
| `values on`, `values off` | Turn 0/1 value colours on or off. |
| `zoom F` | Multiply the map's zoom by F about its middle. |
| `fit` | Fit the die in the map. |
| `critical on`, `critical off`, `critical CLOCK` | Show or hide nextpnr's estimated critical path, or select a clock or full cross-domain label; the selection waits for after-ready parsing. |
| `glow auto`, `glow quads`, `glow top`, `glow mask` | Choose the default dense glow (currently mask), always quads, or top-wire-only/mask glow when dense and far out. |
| `view X Y ZOOM` | Centre on shares X and Y of the die, at ZOOM times the zoom that fits it. |
| `sweep ZOOM SECONDS` | Cover the die row by row at ZOOM, scheduling a view every SECONDS from now. |
| `waves open`, `waves close` | Open or close the waves panel. |
| `watch NAME` | Add an HDL name to waves, including every bit when NAME is a bus. |
| `watch N` | Add the design's first N HDL names to waves. |
| `watch none` | Clear the waves watches. |
| `watch shown` | Add the net kept on the map, as the watch button does. |
| `watch pointed` | Add the net under the scripted pointer next frame, as W does, respecting a text field that owns the keyboard. |
| `waves-chip NAME`, `waves-source NAME` | Use a watched bit or bus row's “Show on the chip” or default “Show in source” action, including an expanded bus. |
| `point X Y`, `point off` | Hold the pointer at shares X and Y of the map's current width and height, or release it. |
| `show-source X Y` | Use the map's “Show in source” at shares X and Y of its width and height; open the span in a project's editor or copy `file:line` in a plain window. |
| `why` | Write `# why` notes about the most-missed wires, their pieces in sight and their changes. |
| `button N down`, `button N up` | Hold or release `btn[N]` (N is 0–6), as on the board. |
| `switch N on`, `switch N off` | Request SW1 lever N (1–4), for `sw[N-1]`; the design must read it. |
| `panel NAME open`, `panel NAME close` | Open or close `monitor`, `oled`, `sd`, `flash`, `rtc`, `audio`, `adc`, `sdram` or `header`; an unavailable panel is unchanged and prints a script error. Like other commands, this adds a `# script` record. |
| `type TEXT` | Send TEXT to the UART receiver, accepting `\r`, `\n`, `\\` and `\xHH`; no CR is added, and a literal `#` must be `\x23`. |
| `board 3d`, `board top`, `board bottom` | Choose the 3D board's view. |
| `hide`, `show` | Minimise the window or bring it back; while minimised it draws no frames, so show waits for a frame. |
| `size W H` | Set the window's size in points. |
| `new-project {"dir":"out/my counter","name":"Counter #1","resources":["clock","leds"]}` | Open New project with these fields; omitted fields use the visible defaults. Also accepted in project scripts. |
| `start-demo {"dir":"out/my uart","name":"My UART","demo":"ulx3s/uart_hello"}` | Open Start from a demo, making a personal copy on Create. Also accepted in project scripts. |
| `project-options {"resources":["clock","leds","buttons"]}` | Patch the active dialog; also accepts name/dir. Demo dialogs accept demo instead of resources. Also accepted in project scripts. |
| `create-project` | Publish and open in the same window with top.v selected. Create only opens the project; use Build and Emulate afterwards. Also accepted in project scripts. |
| `cancel-project` | Close the dialog without writing files or preferences. Also accepted in project scripts. |
| `open PATH` | Open another design; the rest of the line is its path. |
| `build-demo NAME` | Use the start screen demo's Build control, then open its design. Outside the start screen, or for unknown or already-built demos, do nothing and record the reason. |
| `close` | Close the design. |
| `theme light`, `theme dark`, `theme system` | Set the window's theme for this session. |
| `dev on`, `dev off` | Set and immediately save Dev mode, as from Design > Dev mode or the start gallery toggle. Off shows only showcase demos; on shows every example, with a small `dev` tag on other demos. Recent is always unfiltered. |
| `quit` | Close the window. |

The `resources` list in `new-project` and `project-options` accepts `clock`,
`leds`, `buttons`, `switches`, `uart`, `oled`, `hdmi`, `sd`, `audio`, `sdram`,
`adc`, `rtc`, `flash` and `header`. The default is `["clock","leds"]`.
Include `clock` for every recipe and `leds` with `buttons` or `switches`.
Peripheral starters may be selected without LEDs or UART. `header` adds J1/J2
inputs only, never driven by default: safe with nothing connected.
Selected names appear in the creation trace's `resources=` payload.

Project scripts use `ECP5_PROJECT_SCRIPT=<file>`. Each line is
`<delay-seconds> <command>`, with `#` starting a comment. Unlike window
timestamps, each delay starts when the previous command completes. Build
and emulation commands wait for their work; `emulate` also waits
for the design to be ready or fail. A source probe separately waits for the
after-ready source index. See
[gui/src/project/script.rs](../gui/src/project/script.rs) for parsing and
waiting, and [gui/src/project.rs](../gui/src/project.rs) for the actions.

| project command | meaning |
|---|---|
| `new-project {…}`, `start-demo {…}`, `project-options {…}`, `create-project`, `cancel-project` | Same creation handlers and JSON fields as the window table above; accepted even on the start screen before a project exists. |
| `new-file {"path":"rtl/counter.v","module":"counter"}` | Create a relative .v, .sv, .vhd/.vhdl, .lpf or .hex without overwriting. `entity` aliases `module`; omit the name to use the filename stem. Save a dirty ecp5.toml tab first. Refresh inputs and the clean manifest tab, open the new file and require a rebuild. Project script only. |
| `memory-image {"path":"fw.hex","use":true}` | Use an existing relative .hex/.mem/.init as an ecpbram image; `false` stops using it. Save dirty ecp5.toml/ecpbram.txt tabs first. Uses the same library edit and refresh as the tree action, with the explicit script choice taking the place of confirmation. Project script only. |
| `build`, `rebuild` | Save dirty buffers and build, or force a rebuild; wait for completion. |
| `emulate` | Open the last successful build in Chip; if a build is running, wait for it and then emulate on success. Wait for loading to finish. |
| `view project`, `view chip` | Select the project or chip tab. |
| `open FILE [LINE]` | Open a relative file in the editor, optionally at a line. FILE is one whitespace-delimited argument. |
| `source FILE LINE COL` | Probe the current editor buffer at one-based character coordinates, opening FILE if needed. FILE must be a listed relative project input and may contain spaces. |
| `hover FILE LINE COL` | Read the current identifier's published live value without probing or watching; repeats as new publications arrive. |
| `inline on`, `inline off` | Set the editor's existing Inline values checkbox. |
| `type FILE LINE COL TEXT` | Insert TEXT at one-based character coordinates, one Unicode character per frame, using the same unsaved buffer path as `edit`. The next command waits until typing finishes. FILE is one argument; TEXT cannot include a literal `#` because it begins a script comment. |
| `click-message N` | Open the location of visible message N, indexed from zero. |
| `edit FILE LINE TEXT` | Replace a line in FILE's editor buffer with TEXT; FILE is one whitespace-delimited argument. |
| `save` | Save the active editor buffer. |
| `quit` | Close the project window. |

`ECP5_PROJECT_TRACE=<file>` writes a tab-separated event record: seconds since
the project window opened, then the event. It records project and tab changes,
editor opens, build steps and results, messages, reports and chip
loading. Cross-probe and timing notes are also copied into it. This is
separate from the chip window's per-frame record below.

New file is available from the project root/source-group context menus,
Design > New file…, and Cmd/Ctrl+N when no text field has focus. The dialog's
kind selects the extension; HDL names default to the filename. Library errors
appear beside the path. Enter submits and Escape cancels. The status shows
membership notes; LPFs remain inactive and hex data joins auxiliary files.
Both traces receive `file new PATH ok` or `file new PATH failed REASON` and
`manifest NOTE ok` for membership changes, with DisplayPaths and JSON quoting.

Right-click a .hex, .mem or .init in Files and choose **Use as BRAM image
(ecpbram)…**, or choose **Stop using as BRAM image** in Memory images. The
confirmation explains: “Swapped into the placed design by ecpbram after place
and route, so new firmware needs no new place and route. A $readmemh file that
is not listed here is built in by Yosys; changing it reruns synthesis.” Save a
dirty ecp5.toml (or ecpbram.txt) tab in the confirmation, or Cancel to review or
revert it first. Other dirty tabs stay untouched.

The edit makes `[memory].images` explicit, starting from ecpbram.txt when that
was the input, and updates the legacy list while retaining its comments. An
image is removed from explicit `sources.files`; stopping appends it there only
when that list is explicit. Default discovery automatically picks up root data
files again; it does not scan subfolders. Success reloads inputs, refreshes the
clean membership tabs and tree, shows the change notes, drops the old build and
emulator association, and requires a rebuild. Listed images appear in the
Memory images (ecpbram) build step and its tooltip.

Both traces receive `file memory-image PATH on|off ok` or
`file memory-image PATH on|off failed REASON`, plus `manifest NOTE ok` for each
change. Paths use DisplayPaths and ambiguous paths are JSON quoted.

Creation outcomes go to both active traces: `new-project dialog NAME resources=clock,leds`,
`start-demo dialog DEMO NAME`, `project-options ok resources=clock,leds`,
`new-project NAME ok SECONDS`, `copy-demo DEMO NAME ok SECONDS`,
`new-project cancelled` and `copy-demo cancelled`; failures use `ACTION [DEMO NAME] failed REASON`.
Ambiguous names/paths are JSON quoted and paths use DisplayPaths. GUI notes begin
with `# `; project notes retain their elapsed-seconds/tab prefix. The existing
writers and remaining commands survive the transition without truncation or replay.
Only `create-project` restarts the window script clock, after publication and the
same-window transition. Create opens the project with its top source in the editor;
use Build and Emulate when ready. The removed `run` field is an error in both
parsers. Dialog/options/cancel commands keep file order and the current clock. Project delays start after completion.

### Publication record batches

The v0.1.0 kit is prepared in `tools/gui_scripts/v010/batch.json`. **No window
validation is claimed here.** Only use the launch command during the booked
screen block. Build the release GUI first; a debug build is not a useful typing
budget measurement. All cargo commands on the working Mac use `nice -n 19`.

```sh
nice -n 19 cargo build --release -p ecp5_gui
python3 tools/gui_record.py batch tools/gui_scripts/v010/batch.json --dry-run
# Preview or retake only selected labels (batch order is retained):
python3 tools/gui_record.py batch tools/gui_scripts/v010/batch.json --only header_pulls header_external --dry-run
# During the booked screen block only:
python3 tools/gui_record.py batch tools/gui_scripts/v010/batch.json --who v010
# Read and judge the captured files again without opening a window:
python3 tools/gui_record.py batch tools/gui_scripts/v010/batch.json --check
python3 tools/gui_record.py sum out/records/v010_live_blinky.tsv out/records/v010_live_blinky.project.tsv
```

`run --project DIR [--project-script FILE] SCRIPT LABEL` uses the same console
check, timeout, screen lock and stall watch as `run DESIGN SCRIPT LABEL`. It
launches `ecp5_gui --project DIR` and writes `LABEL.tsv`, `LABEL.project.tsv` and
`LABEL.out` in `out/records`. Project builds, the period before emulation and
the Project tab do not spend the stall-sampling budget. Once Chip is ready and
visible, an unexplained three-second pause can be sampled as before. Existing
build outputs can be emulated without rebuilding. Window scripts also advance
while the Project tab is visible. Project delays are relative to the previous
command's completion; window times are relative to ready (or opening a project
with no design). Typing finishes before the next project delay begins.

Batches accept JSON or TOML (`records` is an array of entries). Paths are
relative to the repository, not the batch file. Each enabled entry has a unique
`label`, exactly one `design` (reference name or config path) or `project`, a
window `script` and/or `project_script`, optional `timeout` seconds, and
`criteria`. `--only NAME…` selects labels in their original batch order; unknown
names fail before any work. It works with launch, `--dry-run` and `--check`.

The enabled `v010_new_project` entry uses
`tools/gui_scripts/v010/new_project.window.script` to cancel, create/build/emulate,
and copy a UART demo in one window, checking creation notes in both records. It
starts from a project because the batch schema requires a design or project;
headless host tests cover creation from the start screen. Its destinations under
`out/records/v010/` must be absent or empty. Before a retake, move the previous
created projects aside explicitly; creation never overwrites or picks a suffix.
This entry is prepared for capture; no displayed-window run is claimed here.

The optional fields are general commands and environment settings:

| field | meaning |
|---|---|
| `prepare: {"command": ["program", "arg", …], "output": "path", "timeout": 600}` | Run once before this record, skipping and reporting when the output exists. All selected prepares finish before taking the screen lock. Output parent directories are created before execution. A successful command must create its declared output; use a final build artifact as the guard. Shared outputs are prepared once. |
| `env: {"NAME": "value"}` | Set variables for prepare, window and judge child processes. An empty value stays empty (e.g. `ECP5_HEADER: ""`). |
| `unset_env: ["NAME", …]` | Remove inherited variables from those child environments, without changing the caller's environment. A variable cannot be both set and unset. |
| `judge: ["program", "arg", "{record}"]` | Run an external judge after the record's built-in criteria. Replace literal `{record}` in arguments with the window record path. Its stdout PASS/FAIL lines, stderr and exit status appear under that entry in the verdict table; a FAIL line or non-zero exit fails the entry. `judge_timeout` defaults to 60 seconds. |

Commands are argv arrays, never shell strings. Dry-run prints prepares (or skip
reasons), the main checkout's lock path and owner, each window command with its
set/unset environment, judges with substituted paths, and unlock, in execution
order. It executes no prepare/window/judge commands, creates no outputs and takes no lock;
missing files may be previewed. A planned shared prepare output
makes the second prepare print a skip. Normal launch validates input files after
preparation and before screen access. `--check` runs only verdicts and external
judges against existing records, with no prepare or screen lock.

The two enabled `header_pulls` and `header_external` entries refer to the
`tools/window_record/header_probe.script` and
`header_summary.py` from commit `01f64e6`. Both guard the supplied
build command with `target/reference/header_probe_driven/top.bit`, unset
`ECP5_GUI_NO_BOARD`, and explicitly set `ECP5_HEADER` to `""` or `GN0=0`. The
external case adds `--external GN0=0` to the same judge. For example:

```json
{"label":"header_external", "design":"header_probe_driven",
 "script":"tools/window_record/header_probe.script",
 "prepare":{"command":["examples/ulx3s/build.sh","examples/ulx3s/header_probe/driven","target/reference/header_probe_driven"],
            "output":"target/reference/header_probe_driven/top.bit"},
 "env":{"ECP5_HEADER":"GN0=0"}, "unset_env":["ECP5_GUI_NO_BOARD"],
 "judge":["python3","tools/window_record/header_summary.py","--external","GN0=0","{record}"],
 "criteria":[{"check":"no_home"},{"check":"no_error"}]}
```

No kit project script uses `load`. Hardware loading belongs to the
`ecp5_emu load` CLI. The Header summaries judge recorded model values, rate
phases, board readbacks and UART data; they do not certify panel visibility,
mouse interaction or analog/hardware accuracy. See
`tools/window_record/README.md` for those limits.

The generic checks are `note` (regex count, `min` defaults to 1), `numeric`
(first or named `value` regex capture, every match compared to `bound` using
`<`, `<=`, `==`, `>=` or `>`; missing and non-finite values fail), `layout`,
`no_home`, and `no_error`. Regex checks support `source: "window"`, `"project"`
or `"both"` (default); project timestamps are removed before matching. The
mandatory `no_home` and `no_error` checks are added automatically: both records
are scanned for absolute home paths, and both records plus window output are
scanned for errors; `LABEL.out` also records and must contain `# window exit 0`.
After offline preparation and file validation, the batch holds
one screen lock across the selected runs and their judges. Each record is judged before the next one
starts; the first run error or failed criterion stops the batch. One final
table prints each criterion's measurement and PASS/FAIL. `--dry-run` never
reserves the screen or launches; `--check` only judges existing files.

The named `live` check has two kinds. `kind: "numeric"` (the default) takes
`names` and optional `inline: true`. Each name must have numeric `# value` notes
while paused and running, with a changed value or cycle. `rate: "max"` requires
the running samples to follow that window-script rate setting. `?`, missing
notes and unavailable explanations cannot pass a numeric check. Inline checks
require visible numeric lookups in editor summaries. `kind: "unavailable"` takes
`names` and exact `reason` text and passes when each name's `-> REASON` is recorded.
A trailing stale-source annotation after `; ` does not change that reason.

Blinky keeps a clean source buffer: it hovers `count` at its declaration (6) and
increment expression (7), then `led` (8), which carries the same counter value,
while paused and running, then enables inline values. Life checks `noise` and
`generation` numerically at max. It separately requires the honest response
"`live` has no net on the chip: optimised away, a constant, or inside a cell".
The supplied routed design has source-named nets only for `noise`, `generation`,
`sync`, `led` and `btn`; `live`, `cell`, `next`, `count` and `population` are
inside LUTs. No source-named Life net is in a folded cone. Exact published values
for folded nets are covered headlessly by `tests/published_values.rs`, not by
this window record.

The `syntax` check requires languages in `open FILE:LINE language=LANG` events,
typing phases in both themes, and a **16.7 ms p95 CPU frame budget** for every
typing phase. The default minimum is six typing phases. Idle p50 for highlighted
PicoRV32 must be no higher than **1.25 × plain idle p50 + 0.5 ms**. The plain
baseline is prepared by copying `examples/ulx3s/picorv32_console/picorv32.v` to
`out/records/v010/picorv32_plain.txt` at batch time, then opened in the same
project window with its plain-text extension. No second PicoRV32 source copy
is stored under tools. Remove that generated output to refresh it after changing
the source; preparation otherwise skips an existing output. Syntax
emulates the existing build and pauses first, so device-model loading finishes
before its baseline/typing phases. These are CPU
UI costs; they exclude sleeping between redraws. They are not compared to the
main record's wall-clock `frame_ms`. The SV/VHDL samples exercise lexer token
categories (comments, strings, identifiers, literals, attributes, directives,
keywords and operators). Their minimal project is never built by this kit.

`# editor frames` project notes include phase, file, theme, inline state,
`kind=typing|idle`, n, and `frame_p50_ms`, `frame_p95_ms`, `frame_max_ms`, plus
corresponding `layout_*` times and `inline_values`. A typing frame laid out a
changed previously rendered buffer; a newly opened buffer is idle. Each project
command, file/theme/inline change, shutdown or 600 accumulated frames flushes
the bounded summary. Script phases also have `# script phase=N COMMAND` notes.
Window `# map-layout` notes give actual `report=on|off`, `central`, `shelf`,
`map` rectangles (x0,y0,x1,y1). They are emitted on the first recorded map and
only when one of those rectangles changes; camera zooms and drags alone add no
notes. Each resize note carries `before_centre`/`before_cell` from the immediately
preceding frame and `centre`/`cell` after the resize (die units and pixels per
unit), plus `resized=1`; the initial note has `resized=0`. Layout checks reconcile
every legacy `# layout` map with these measurements, check full width when closed
and an 8 px gap when open, and limit the before/after centre drift from that one
note to 1 px when physical zoom is unchanged. Fitted resizes can change physical
zoom. These notes make the shelf and camera check independent of screenshots.

Synthetic tests, without screen access:

```sh
python3 -m unittest discover -s tools/gui_scripts/v010 -p 'test_*.py'
nice -n 19 cargo test -p ecp5_gui
```

The tour ignores both script variables. Without explicit trace paths, it
writes `tour/last-project.tsv` and `tour/last-window.tsv` beside `recent.txt`;
`ECP5_PROJECT_TRACE` and `ECP5_GUI_TRACE` override those destinations. Its
step notes go to both records. See
[gui/src/project/script.rs](../gui/src/project/script.rs).

`ECP5_GUI_TRACE=<file>` writes a tab-separated record with a header and a line
per frame, plus `#` notes. See [gui/src/trace.rs](../gui/src/trace.rs) for the
columns: frame and map times, run state, values, zoom, change checks, lag across
the map's five fifths, coverage, dropped frames, GPU pass times and uploads.
GPU results arrive a few frames late; `map_frame`, `gpu_frame` and
`upload_frame` identify which frames the measurements describe.

| note | what it records |
|---|---|
| `# design CONFIG` | The opened configuration, relative to the project root when under it (preserving older records), otherwise relative to the working directory or `~/…` under home. No absolute home path or substitute prebuilt label. `closed` marks closing. |
| `# load` | Stage changes seen by the window, timed since opening this design; `die-shown` marks its first drawing of the die. |
| `# probe source FILE:LINE:COL -> RESULT` | The editor probe's net summary or no-net explanation, including a stale-source warning when applicable. |
| `# probe map TARGET -> FILE:LINE,…` | The map target, including net aliases, and its source choices, using relative paths. No match from `show-source` records `# probe map none -> no source at this point`. |
| `# probe index SECONDS NETS LOCATIONS`, `# probe load-pass SECONDS` | The after-ready index's build time and counts, and time collecting source attributes during loading. |
| `# critical CLOCK MHz HOPS TOTAL` | Each published path's clock or domain, Fmax (`-` if absent), hop count and total delay in ns. |
| `# critical cost SECONDS BYTES` | The after-ready timing work's time and log byte count. Parser errors are recorded as their explanation, including `# critical path: nextpnr VERSION not supported`. |
| `# critical hop N TYPE NS NAME -> FILE:LINE` | Each row when a path is shown, numbered as on the map, with hop delay and its first source location (`-` if absent). |
| `# switches` | The runner's reported positions, initially and whenever they change, including after going back: seconds since ready, cycle, then four 0/1 characters with `sw[0]` first. |
| `# switch requested` | Each click or scripted request: seconds since ready, cycle, lever 1–4 and on/off; the lever can move before a `# switches` report confirms it. |
| `# waves` | Panel open/close, watch additions and results, watch ids and counts, freshness/status changes, received and dropped rows, and spans received or drawn. |
| `# waves chip NAME -> N nets`, `# waves source NAME -> FILE:LINE,…` | A row's chip or source action, using relative source paths. Failed scripted actions record `unavailable: REASON` or `not a Waves row` after the arrow, and print a script error. |
| `# tour N STEP CAPTION`, `# tour caption N CAPTION` | Entry into one of six steps (then `6 done`), or a caption change within a step, including chip readiness. |
| `# tour skip build REASON` | Missing-tool fallback; a second `# tour 1 build CAPTION` records its caption. |
| `# tour pointer on probed wire X Y` | The marked wire's 1.5-second pointer hold, at shares of the map's width and height. |
| `# tour done SECONDS`, `# tour stopped at STEP`, `# tour failed STEP REASON` | Completion time, user interruption or failure. Reader failure before step 1 uses `# tour failed reader REASON`; timeouts end without advancing. Tour paths are relative. |
| `# tour reader building`, `# reader missing`, `# reader built`, `# reader found` | Waiting for the shared image-reader build, and the reader check's missing, built or already-present result. There is no numbered reader step. |
| `# start build NAME ok\|failed SECONDS` | A start-screen demo build's result and elapsed time, from either its control or `build-demo`. |
| `# dev on`, `# dev off` | Each Dev mode preference change, from the Design entry, start gallery toggle, or window script. Redraws and repeated requests for the current setting add no change note. |
| `# board` | Readbacks of the GPU's board picture: LEDs D0–D7 expected lit, drawn lit and drawn dark, `ok` or `WRONG n`, response times and running totals; frame columns say whether the window showed it. |
| `# busy` | At max, every two seconds, how busy the wires that changed are and how strongly they glow, as quantiles. |
| `# map glow` | Requested and drawn glow modes when either changes, listed piece count and the drawing's frame number; this is read back a few frames late. |
| `# script` | Seconds since ready and each command run. `# script build-demo NAME: REASON` explains an unknown or already-built demo. |
| `# uart` | Seconds since ready, cycle and bytes received from the design, with escapes for non-printable bytes. |
| `# button` | Button number, down/up, seconds since ready and cycle. |
| `# toolbar` | Once per window size change: `size=W,H`, `row`, `navigation`, `build` (`-` for a config), `transport`, `time`, `view` (`-` below 1100 pt), and `compact=0|1`. Rectangles are `left,top,right,bottom` in logical points. |
| `# layout` | Window, board and map positions and sizes in points when they move. |
| `# missed`, `# why` | Changes missed and the requested diagnosis of the most-missed wires. |
