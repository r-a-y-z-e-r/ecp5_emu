# Final README and site media pass

This kit prepares candidates from a **release build of ECP5 Emu.app**.
Capture commands write candidates; the separate offline `publish.py` step
prepares the selected takes in `site/media/`. README has three images: Life, Orchard
and cross-probe. The hero and Orchard (`oled_bricks`) stills were approved
on **2026-11-18**.

## Shot list

Sizes below are encoded pixels. Clips are 1100 px wide, even proportional height,
H.264/yuv420p at 12 fps, CRF 28, preset `veryslow`, with fast start and no audio.
The chosen `clip_encoding` profiles live in `batch.json` and are used by the next capture pass. Every WebP must be **≤250 KiB**.
Scripts are linked below; the machine-readable timing/output list is [batch.json](batch.json).

| File | Used on | Design | View / state | Size / duration | Script |
|---|---|---|---|---|---|
| `hero-life-full.webp`, `window-life-full.mp4` | README; site introduction | `ulx3s/life_full` | Chip; R44C60 wire values at 100 cycles/s and live 3D board, then fit / max / zoom | 1680×960 still (entire window, same aspect); 1100w / 30 s | [life](shots/life.window.script) |
| **NEW** `oled-bricks.webp`, `oled-bricks.mp4` | README second of three images | `gallery/oled_bricks` | Chip; attract only, ball in flight, cleared bricks, score >0; live 3D OLED module and OLED panel both visible | 1500×978; 1100w / 18 s | [oled](shots/oled.window.script) |
| `window-cross-probe.webp`, `window-cross-probe.mp4` | README; site projects, window-guide | `ulx3s/blinky_cycle` | Editor name probe → its nets on Chip map → `count[2]` in Waves; 20 cycles/s | 1500×898; 1100w / 15 s | [window](shots/cross-probe.window.script), [project](shots/cross-probe.project.script) |
| `window-start.webp` | site window-guide | none | Fresh start screen; **New project… / Start from a demo… first**, no recent history | 1500×978 | [start](shots/start.window.script) |
| **NEW** `window-new-project.webp` | site projects | none | New project dialog, basic resource checklist visible; Advanced stays collapsed (no script command to expand it); never submits | 1500×978 | [new-project](shots/new-project.window.script) |
| `window-project.webp`, `window-project-build.mp4` | site projects, window-guide | `gallery/riscv_shell` | Project; top.v, successful build steps and current toolbar | 1500×978; 1100w / 28 s | [window](shots/project.window.script), [project](shots/project.project.script) |
| `window-critical-path.webp` | site window-guide | `ulx3s/picorv32_console` | Timing line's critical path on die, 20,000 cycles/s | 1500×898 | [critical](shots/critical.window.script) |
| `window-waves.webp`, `window-waves.mp4` | site window-guide | `ulx3s/picorv32_console` | Waves: PC bus, LEDs, UART; console; 20,000 cycles/s | 1500×978; 1100w / 18 s | [waves](shots/waves.window.script) |
| `window-riscv-terminal.webp`, `window-riscv-terminal.mp4` | site window-guide | `gallery/riscv_terminal` | Live HDMI text terminal and UART; help, LED command, Mandelbrot, screen | 1500×978; 1100w / 60 s | [terminal](shots/terminal.window.script) |
| `window-life-full-zoom.webp` | site window-guide | `ulx3s/life_full` | Die-map detail at R44C60, 120 px/tile, values on, 100 cycles/s | **760×634, ≤250 KiB** | [life-zoom](shots/life-zoom.window.script) |
| `board-closeup.webp` | site window-guide | `gallery/riscv_terminal` | Live 3D board crop, HDMI cable / mode label | 1000×690 | [board](shots/board.window.script) |
| `window-tour.mp4` | site getting-started | `ulx3s/blinky_cycle` | Scripted build/chip/run/source/constraints/Waves walkthrough | 1100w / 43 s | [window](shots/tour.window.script), [project](shots/tour.project.script) |

The hero uses the current Chip window with board alongside the map, at 1680×960.
The other existing still dimensions are retained; detail and board images use
explicit crop rectangles and padding when needed to preserve their selected content.

## Preconditions for the media pass

- Set `ECP5_MEDIA_APP` to the final `ECP5 Emu.app`; default is
  `target/dist/ECP5 Emu.app` relative to the repository. The launcher, image reader,
  architecture, board model, example sources and FPGA toolchain come from that
  package. The kit strips inherited `ECP5_*` overrides from the launched app so a
  developer database, model, compiler, trace or demo source cannot replace them.
  The take records the package version and GUI SHA-256. Verify this is the final
  release artifact before the pass; the kit cannot establish release approval.
- macOS needs `python3`, `swiftc`, `ffmpeg`/`ffprobe` (libx264), `cwebp`, and existing
  Screen Recording / Accessibility permissions for the invoking terminal. No
  application build is attempted; chip-shot designs are built headlessly by the
  packaged CLI before the window batch. Compilation of the small metadata helper happens
  only in live mode.
- Display awake and unlocked, screen free, application foreground and **not
  minimised**. Enable Do Not Disturb; move other windows and notification banners
  away. The known media display is 1728×1117 logical points with a 37-point menu
  bar. At (30,80), the geometric limit is 1698×1037; all requested outer windows
  fit. Life uses 1680×960, below the previously observed 981-point OS height clamp;
  others use 1500×978 or 1500×898. After positioning, an actual/requested size
  mismatch fails with `window resized by the OS: asked WxH, got WxH` before any
  capture. Metadata guards also reject covering windows, offscreen placement,
  sleep, disappearance, moves and resizes. Never interact during a take.
- `gui_record.py` owns the common checkout's `out/screen.lock` for the **whole
  selected batch**; contention fails without removing someone else's lock.
  This is the repository's coordination lock, separate from the OS lock screen.
- All packaged examples are refreshed with their internal relative symlinks into
  `ECP5_MEDIA_NEUTRAL/examples/` when that variable is set. The folder is created
  if absent and reused if present; it must resolve beneath `/private/tmp`.
  Otherwise each invocation uses a fresh `/private/tmp/ecp5-media/take-…/` tree.
  Source refresh updates changed files and removes obsolete sources, preserving
  **every `build/` folder**, including `.flow/` and cached outputs. Projects build there.
  Each shot gets an isolated neutral HOME with dark theme, Dev off and no recent
  files in a separate session directory, so repeated captures do not collide.
  The New project destination is neutral too. Original projects and user
  preferences are never changed. Neutral folders remain after prebuild and capture.
- Review **every still** for visible paths and user names, including titlebars,
  editor tabs, logs, dialogs and status text. Review clips for the same leaks,
  covering windows, banners, stalls and the end frame. Automated guards do not
  replace this visual review; no candidate is approved automatically.

## Run offline checks now

From any directory, with the repository path substituted for the command path:

```sh
# All script commands, JSON, timings, output names and documented OLED budget.
./tools/site_media/batch.sh --check
./tools/site_media/run.sh oled --dry-run
./tools/site_media/run.sh prebuild ulx3s/life_full --check
python3 -m unittest discover -s tools/site_media -p 'test_media.py' -v
bash -n tools/site_media/run.sh tools/site_media/batch.sh
shellcheck tools/site_media/run.sh tools/site_media/batch.sh
python3 -m py_compile tools/site_media/media.py tools/site_media/encode.py tools/site_media/test_media.py
```

`--check` and `--dry-run` are aliases: **no windows, pixels, display queries, lock,
package probing, compilation or preparation**. They validate all 16 scripts,
including when selecting one shot. The validator intentionally accepts only the
commands used by this kit and checks their arguments against
[trace.rs](../../gui/src/trace.rs), [app.rs](../../gui/src/app.rs) and
[project/script.rs](../../gui/src/project/script.rs). New commands require updating
the allowlist. The offline check also verifies every window fits the known display,
full-window stills have the window's aspect, and crops declare valid rectangles.
This verifies syntax and kit invariants, not runtime visual state.

The kit imports `gui_record.py` unchanged for manifest checks, console checks,
process cleanup, signal handling and the inode-owned common screen lock. Its
`batch --dry-run` only plans launches; its `batch --check` judges existing traces.
Neither validates every window command. Its `record_run` forces development
reader/model paths and has no pixel-capture hook, so the kit supplies the bounded
capture lifecycle while retaining that runner's lock convention and trace format.
Do not invoke `gui_record.py batch batch.json` directly: this manifest adds media
metadata, and its start-screen placeholder is not a design to launch.

## Media pass order: prebuild before the screen slot

First build the **final** `.app` at the path you will use for both prebuild and
capture (the packaging prerequisites for `make_app.sh` must already be satisfied).
From the repository root:

```sh
./packaging/macos/make_app.sh
export ECP5_MEDIA_APP="$PWD/target/dist/ECP5 Emu.app"
export ECP5_MEDIA_NEUTRAL=/private/tmp/ecp5-media/final
# Ahead of the screen slot: Life place and route can take about 40 minutes.
./tools/site_media/run.sh prebuild ulx3s/life_full
# Once the screen is free, in the same shell or with the same exports:
./tools/site_media/batch.sh
```

`prebuild DESIGN...` requires `ECP5_MEDIA_NEUTRAL` and accepts one or more packaged
`GROUP/NAME` designs, for example `ulx3s/life_full gallery/oled_bricks`. It refreshes
sources, then invokes only the packaged CLI's `build` command; it never opens a
window, captures pixels, queries the display, compiles the window helper, or takes
`out/screen.lock`. The app is not rebuilt by this mode. Logs and `builds.json` go to
a new `out/site_media/` directory. It prints each step's `ran`/`skipped` result and
a per-design result (`skipped` means all steps were skipped). The full CLI log also
holds progress and diagnostics while the build is running.

Both prebuild and batch preparation use `batch.json`'s `build_timeouts`, keyed by
`GROUP/NAME`: **`ulx3s/life_full`: 3600 s**, all unspecified designs: **600 s**.
Both Life shots share one build per batch. A cache miss still builds with that
timeout; the kit always asks the flow to decide which steps can be skipped.

The flow skips a step only when its **tool path, tool version, arguments and input
hashes match**, and its expected outputs still exist with matching hashes. Keep
the `.app` at the **same path** and use the **same `ECP5_MEDIA_NEUTRAL`** for
prebuild and batch: moving either changes paths in those keys. Changed tools or
sources can legitimately rerun steps. Refreshing sources preserves `build/` but
does not bypass the flow's cache checks. Rebuild the final app before prebuilding;
if it changes afterward, prebuild again before the screen slot.

## Run one shot when the screen is free

```sh
# Use the same final package and neutral folder as the prebuild above.
./tools/site_media/run.sh oled       # one complete still/clip shot
```

Labels: `life`, `oled`, `cross-probe`, `start`, `new-project`, `project`, `critical`,
`waves`, `terminal`, `life-zoom`, `board`, `tour`. Each invocation creates a fresh
`out/site_media/DATE-TIME-NEUTRAL-…/` directory holding raw PNG/MOV takes, standard
window/project TSV records, logs, coverage metadata, encoded candidates and
`provenance.json`. Nothing is written to `site/media/`; a separate human review
copies chosen outputs and adds the two new media references.

Chip shots share [emulate.project.script](shots/emulate.project.script), which
opens the neutral project’s CLI-built outputs before window commands can advance
on an unbuilt Project tab. Their designs are built with the packaged CLI before
taking the screen lock; the riscv_shell build shot builds in its window instead.
With a reusable neutral folder, preparation invokes the same flow and reports
its cached/skipped steps before opening any window.

Chip script times are seconds since design readiness. Project script numbers are
delays after the previous command completes; build/emulate await completion.
Capture uses readiness notes, not guessed startup sleep. The New project dialog
gate reads the immediately flushed project trace, so it does not wait for a
static window's buffered notes. Other window-trace gates run on a live Chip;
`Trace::frame` flushes their notes each frame. Scripts never minimise
or quit. `screencapture -V` finishes itself while the app stays open; a one-second
capture margin allows automatic 0.25 s leading trim and trailing trim to the exact
requested clip duration. Only after capture is reaped does the helper stop the
window. On interruption/failure it kills and discards partial capture **before**
closing the window; it never relies on SIGINT to finalise a MOV. Raw images remain
for comparison with WebP compression. FFmpeg scales PNGs; `cwebp` encodes WebP without requiring FFmpeg’s
optional libwebp encoder. WebP quality is reduced until the size cap
is met, or encoding fails. Check legibility before accepting a low-quality result.
Full-window stills must have exactly the captured aspect ratio: scaling never
crops the toolbar or sides. Clips scale the whole window to 1100 px wide with
even-height rounding for H.264. Both encoders reject a raw media aspect that
differs from the measured capture window.

The detail and board entries explicitly declare `crop` as
`{"layout":"die map","rect":[0,0,1,1]}` or the corresponding `"board"` layout.
The rectangle is `[x,y,width,height]` in fractions of that trace layout's bounds;
`[0,0,1,1]` selects the entire region. Titlebar/backing-pixel conversion produces
the pixel crop. Only that rectangle is cropped; scaling fits it inside the target
still with padding if necessary, preserving all selected content. Check crop
framing on the first media pass.

The riscv_shell capture starts at `build start` and requires `build ok` within its
28 s recording. If the final package/machine is slower, increase that entry's
`clip_seconds` and `still_at` together, then retake/review the longer build clip.
The 600 s window timeout includes loading and any in-window build. Headless
preparation uses the separate per-design build timeouts above. Failures stop the
batch, preserving completed candidates and the neutral folder's caches.

## Orchard timing and first-pass review

The [demo README](../../examples/gallery/oled_bricks/README.md) says each normal
picture interval is **493,387 clocks**, initialization holds reset for **25,000**
clocks and sends **37 bytes**, and first score is **F000F / S001 / B61**.
It identifies **F004B** as its reference still. The helper derives its conservative
warmup from that documented reference rather than wall seconds:

```text
25,000 + 37 × 35 + (0x004B + 1) × 493,387 = 37,523,707 clocks
37,523,707 / 25,000,000 = 1.50094828 simulated seconds
```

This is roughly 3–20+ wall seconds depending on achieved emulation speed, not a
fixed delay. Event UART lines lengthen intervals, so the cycle threshold alone is
insufficient: capture also requires the latest UART frame report to have score >0
and <62 bricks, ≥76 painted OLED frames, and GPU evidence that the 3D module,
screen and lit pixels were drawn. The same gate is rechecked at the still. Scripts
send no input, keeping the fixed-seed attract game. The window's OLED panel opens
automatically when driven; the 3D module is fitted automatically from live pixels
by the current app. Verify both displays, ball in flight, cleared bricks and score
legibility visually; resize/reframe and retake if the panel covers the map/board.

Before the full pass, run `oled` and check its displays, exact 1500×978 framing,
score, paths and compression. Then check the cross-probe source coordinate still
selects `count`, the waves row is specifically `count[2]`, the Timing shelf/path is
visible, the terminal has finished Mandelbrot by its still, and the board label is
inside the crop. Build timings, Fmax and UI wording in existing page captions may
need a separate update after review. No window was opened to validate these here.

The current `--tour` is interactive and ignores script environment variables.
`tour.project.script` therefore preserves the former media walkthrough as a
scripted project sequence, without claiming to exercise the interactive Next /
Show me controls. A literal interactive tour recording needs a separate attended
pass; this batch does not inject clicks into that tour.

## Scratch provenance

Carried forward as adapted behavior: `record.sh`, `record_project.sh`,
`record_tour.sh`, `still.sh`, `stills_window.sh` → `media.py` (ready waits,
foreground/coverage checks, bounded clip/still capture, capture-before-close).
`life_full_v3.script`, `hero_waves2.script`, `xprobe3_gui.script`,
`xprobe2_project.script`, `crit2_gui.script`, `project.script`, `picorv32.script`,
`board_close.script` supply the shot sequences. `terminal` also uses the tracked
demo's `gui.script`. The metadata source `winpid.swift` becomes `window_info.swift`.

Dropped: obsolete `life_full`, `life_full_slow`, `life_full_v2` variants in favor
of v3's readable values; duplicate `record_gr`/`record_project_nc` and old retake
wrappers; `with_screen_lock.sh` in favor of the existing runner's shared lock;
`banner_hits.sh`/`nc_banners.sh` because their notification scraping and fixed PIDs
are replaced by direct intersecting-window metadata. No scratch binaries, images,
movies, take directories or machine-specific absolute paths are dependencies.


## Prepare the selected release takes offline

[publication.json](publication.json) names exactly the selected take folders
for the 2026-11-18 pass from app commit `0d53bef`, including source rectangle,
previous file size and chosen clip profile. The [media provenance](../../site/media/README.md)
records the package ZIP hash, capture command and output hashes. `publish.py` checks
selected GUI hashes, rectangles, still budgets, full-window aspects and the profile
against the capture batch. It copies stills and re-encodes each raw MOV; it never
opens a window, captures a screen or takes the screen lock.

```sh
python3 tools/site_media/publish.py --check  # read-only checks of the selected sources
python3 tools/site_media/publish.py        # prepare the selected media in site/media/; no commit
```

At 12 fps / CRF 28 / veryslow, all replacement clips are within 1.5× their HEAD
sizes (the helper enforces the recorded byte thresholds). The new OLED clip is in
the same few-hundred-KiB class. Sampled encoded frames were compared with scaled
raw frames for text clarity. The lower frame rate trades motion smoothness for
size while retaining readable static text. Raw takes are retained unchanged.
