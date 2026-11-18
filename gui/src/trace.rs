//! A record of what the window draws, frame by frame, and a script that
//! drives it, to check the map against what the run does: whether every
//! change of a wire in sight is drawn lit, and with 0/1 drawn with its new
//! value, within the time it takes to spread; how long after it came it was
//! first drawn lit, in each fifth of the map from left to right; and how
//! long frames take.
//!
//! `# design <config>` keeps project-relative paths for configs under the source
//! root. Other configs use working-directory-relative paths or `~/…` under home;
//! labels never substitute an unrelated prebuilt config or expose the home path.
//! Path-bearing notes, including embedded tool errors, use `DisplayPaths`.
//!
//! `# map-layout report=on|off central=x0,y0,x1,y1 shelf=x0,y0,x1,y1|-`
//! `map=x0,y0,x1,y1 resized=0|1 before_centre=x,y before_cell=N centre=x,y cell=N`
//! measures actual central/shelf/map rectangles. It is emitted initially and only
//! when those rectangles change. Zooms and drags alone add no notes. The camera
//! pair comes from the immediately preceding frame and the frame after resizing,
//! in die units (cell is pixels per die unit); resized=0 marks the initial note.
//! Project records add `open FILE:LINE language=LANG`, `# inline on|off`, and
//! `# editor frames phase=N file=FILE theme=THEME inline=on|off kind=typing|idle`
//! with n and p50/p95/max CPU milliseconds for frame and layouter, plus
//! inline_values (visible numeric inline lookups). Summaries flush on project
//! command, file/theme/inline changes, close, or 600 frames. First layout is idle;
//! a layout that saw a changed previously rendered buffer is typing. These CPU
//! frame times exclude sleeping between redraws, unlike frame_ms in this record.
//! `inline on|off` and `type FILE LINE COL TEXT` are project commands; type uses
//! the edit buffer path, one Unicode character per frame, without saving.
//!
//! `# value <relative-file:line:col> <name> = <displayed value> @ <cycle>`
//! records a source hover's captured publication. A changed result or publication
//! adds a note; redraws do not. Unavailable states use `-> <reason>` without a
//! numeric value or cycle. `hover FILE LINE COL` in the project script uses the
//! same current-buffer lexer and name-only lookup as pointer hovering.
//!
//! `# start build <demo> ok|failed <seconds>` records a start-screen build.
//! `# dev on|off` records each Dev mode preference change, from either UI control
//! or the window script. Redraws and repeated requests add no change note.
//! `# reader missing|built|found` records the shared image reader check.
//! Before step 1, `# tour failed reader <reason>` ends the tour when the
//! reader cannot be built; `# tour reader building` marks waiting for
//! `python3 tools/build_image_reader.py --output <reader path>` from the
//! repository root. The script discovers prjtrellis and libtrellis; failure
//! notes keep its last error, with display paths and a README Requirements
//! pointer. Success adds `# reader built`. The check adds no numbered step.
//!
//! While loading, `# load <seconds since opening the design> <stage>`
//! records each stage change the window sees. `# load die-shown <seconds>`
//! marks the first drawing of the die, with or without the design's wires.
//! These clocks restart for each design opened in the same window.
//! `# critical <clock> <MHz|-> <hops> <total ns>` records each published
//! report; `# critical cost <seconds> <log bytes>` its after-ready cost.
//! `# critical hop <n> <type> <ns> <name> -> <file:line|->` records every
//! row when a path is shown. Numbers match the cell outlines on the map.
//! `# hint debounce BUTTON CYCLES RATE` records the slow-rate debounce hint once
//! per hold: the button's name, cycles held so far, and requested cycles/second.
//! `# hint step-leds STEP SINCE` records a paused Step/Back/GoTo that left used
//! LEDs unchanged: cycles navigated and cycles since their last observed change
//! (or the initial publication when none has changed), once per completed step.
//!
//! `ECP5_GUI_TRACE=<file>` writes a line a frame (tab-separated, with a
//! header): the seconds since the design was ready, the time since the last
//! frame and the time drawing the map took, the run as the window shows it
//! (running, rate, cycle, MHz), 0/1, the zoom; then for the map: the wires
//! whose change came in this frame (of those in sight), the wires first
//! drawn lit in the drawings read this frame, and since the start how many
//! changes were drawn lit and how many not (missed) in time, and the same
//! for the 0/1 values; then `lag`: for each fifth of the map (by where a
//! wire's first piece in sight is), the changes judged this frame, the sum
//! and the greatest of how late the first frame that drew each lit was,
//! in milliseconds (`n/sum/max`, five of them): late after the moment the
//! change was to reach the wire's piece in sight nearest its net's driver
//! (it spreads from the driver as it is meant to; that is not lag). Missed
//! wires are written as `# missed …` lines. Last, `value_unseen`: changes
//! whose wire the drawing read has nowhere on the screen (under other wires
//! at every pixel, as most short wires are at fit on a full chip), whose
//! value is then not judged; `dropped`: frames of the map egui threw away
//! so far (it prepared them, and the window could not be drawn);
//! `valued_wires` and `value_seen_wires`: since the start, the wires whose
//! change was judged with 0/1 shown, and of them those whose value was on
//! the screen in at least one of their changes' judging (the rest were
//! under other wires each time).
//! Last, `gpu_ms`: the sum of timestamp-query times of the map's own
//! compute and render passes, read without waiting a few frames late;
//! `gpu_upload_ms`: the wire scatter pass only (queue writes happen before
//! these commands, so their transfer time is not measured); `gpu_list_ms`:
//! the cull pass listing pieces; `gpu_draw_ms`: drawing the cached image of
//! pieces when stale, and the glow mask when used. The total also includes
//! resetting the record's marks. These times exclude buffer copies and egui's render
//! pass, which draws the spots, colours and glows. A pass not needed costs
//! zero; no new timing, or no `TIMESTAMP_QUERY` on the adapter: -1.
//! `upload_kb`: CPU bytes written to map buffers / 1024 (queue writes and
//! mapped initial contents, including uniforms, pieces, spots and marks);
//! `full_upload`: 0/1, whether sync sent the whole wire-state buffer.
//! Upload counts are from the preceding preparation, since this line is
//! written before egui prepares the current frame. `map_frame` identifies
//! the current map frame; `gpu_frame` and `upload_frame` identify the
//! frames measured in the new columns (-1 when none). GPU samples may be
//! skipped when all four small readback buffers are busy; none is waited on.
//! `glow_unseen`: changes whose wire owns no pixel in the drawing read
//! while the colour pass gives only the top wire its glow. Like
//! `value_unseen`, these are covered everywhere: neither lit nor missed.
//! `gpu_mask_ms`: the sum of the mask's splat compute and strip render
//! durations; `gpu_splat_ms` and `gpu_strips_ms`, appended last: those
//! individual durations. Each uses only its own end minus its own start,
//! excluding gaps between passes or submissions. A missing or reversed
//! pair gives -1 for that part and totals containing it; it is never
//! sorted into a positive duration. Passes not needed still cost zero.
//! In quad and mask modes changes are judged as before. `# map glow
//! <requested mode> <drawn mode> <listed count> frame <frame>` notes a change
//! of either mode (e.g. `auto mask` or `mask quads`), with the count and
//! frame of the drawing read a few frames late (without indirect listing,
//! the visible-piece estimate). This count is read even with marks disabled.
//!
//! `ECP5_GUI_SCRIPT=<file>` gives commands, a line each: the seconds after
//! the design is ready, then `rate N|max`, `run`, `pause`, `step N`,
//! `values on|off`, `zoom F` (about the middle of the map), `fit`,
//! `critical on|off|CLOCK` (nextpnr's estimated critical path, selecting
//! the clock or full cross-domain label; waits for after-ready parsing),
//! `glow auto|quads|top|mask` (quads always, or top-wire / mask glow when
//! dense and far enough out; auto uses the dense mode chosen by
//! `gpu_map::AUTO_DENSE_GLOW`, mask for now),
//! `view X Y ZOOM` (the map's middle at shares X, Y of the die, at ZOOM
//! times the zoom that fits it), `sweep ZOOM SECONDS` (the die covered view
//! by view at ZOOM, a `view` every SECONDS from then, row by row: on a full
//! chip at fit most short wires are under others, and their values are
//! judged only when a view shows them), `waves open|close`,
//! `watch NAME|N|none|shown|pointed` (the waves watch an HDL name, every bit
//! of a bus by its name, the design's first N
//! names, or nothing; `shown` adds the net kept on the map, as the button;
//! `pointed` adds the net under `point X Y`, as W, including the check for
//! a text field owning the keyboard; it waits for the next frame's pointer),
//! `waves-chip NAME` / `waves-source NAME` (a watched bit such as `count[3]`
//! or a bus `count`, including when opened; the row's Show on the chip or
//! default Show in source action). Notes: `# waves chip <name> -> <n> nets`
//! and `# waves source <name> -> <file:line>[,…]`, with relative paths only.
//! Failures add `# waves <chip|source> <name> -> unavailable: <reason>`
//! (the menu's reason), or `# waves <chip|source> <name> -> not a Waves row`;
//! both are also printed as script errors.
//! `point X Y` (the pointer held at that point of the map as it is then, as
//! shares of its width and height; `point off` lets it go), `why`
//! (the wires missed most, as the map has them now: their pieces in sight,
//! on the screen, and their changes, written as `# why …` lines),
//! `button N down|up` (btn[N] held or let go, as from the 3D board),
//! `switch N on|off` (SW1 lever 1..4, sw[N-1]; it must be read by the design),
//! `panel NAME open|close` (NAME is monitor, oled, sd, flash, rtc, audio,
//! adc, sdram or header; unavailable panels do nothing and print a script
//! error; EDID is in the Monitor window),
//! `type TEXT` (TEXT sent to the UART receiver as the console sends a line,
//! with `\r`, `\n`, `\\` and `\xHH` for those bytes and no CR added; `#`
//! starts a comment, so a typed `#` is `\x23`),
//! `show-source X Y` (Show in source at shares of the map's width and
//! height, as `point` uses; copies file:line in a plain window or opens
//! the span in a project's editor),
//! `board 3d|top|bottom` (the 3D board's view), `hide` / `show` (the window
//! minimised and back), `size W H` (the window's size, points), `open PATH`
//! (another design; later times count from when it is ready), `build-demo NAME`
//! (the start screen's Build control; later times wait for the build and count
//! from when its design is ready), `close`, `theme light|dark|system`, `dev on|off`
//! (persist Dev mode; showcase demos only when off, all examples when on), or
//! `quit`. `#` starts a comment. An unknown or already-built demo adds
//! `# script build-demo NAME: REASON` and does nothing. A build uses the same
//! `# start build NAME ok|failed SECONDS` note and open as the row's control.
//! In a script with `open` or `build-demo`, times restart for each design,
//! so lines keep the file's order across opens, builds and closes, and run by
//! time between them. Each command adds `# script <seconds> <command>`.
//!
//! Creation commands work in both window and project scripts, with JSON strings
//! preserving spaces and `#` (comments begin only outside quoted strings):
//! `new-project {"dir":"DIR","name":"NAME","resources":["clock","leds"]}`,
//! `start-demo {"dir":"DIR","name":"NAME","demo":"ulx3s/uart_hello"}`,
//! `project-options {"name":"NAME","dir":"DIR","resources":["clock","leds","buttons"]}`,
//! `create-project`, and `cancel-project`. Options patch the open dialog; a demo
//! accepts demo instead of resources. Omitted fields use the visible defaults.
//! Submit waits for publication and the same-window transition, opening the top
//! source in the editor; subsequent window times restart then. Create never builds.
//! The removed `run` field is rejected; use Build and Emulate afterwards.
//! Dialog/options/cancel commands retain the current time origin and file order.
//! Creation never loads hardware.
//! Outcomes are mirrored into both active records: `new-project dialog NAME
//! resources=clock,leds`, `start-demo dialog DEMO NAME`, `project-options ok
//! resources=clock,leds`, `new-project NAME ok SECONDS`, `copy-demo DEMO
//! NAME ok SECONDS`, `new-project cancelled`, or `copy-demo cancelled`.
//! Failures use `ACTION [DEMO NAME] failed REASON`; ambiguous names and paths are
//! JSON quoted, and all notes use DisplayPaths. GUI notes have the `# ` prefix;
//! project notes have elapsed seconds and a tab. Writers and remaining commands
//! move with the existing App; opening a project does not reopen either record.
//!
//! Project scripts also accept `new-file {"path":"rtl/counter.v","module":"counter"}`.
//! `entity` is an alias for `module`; omitting the name uses the filename stem.
//! Supported extensions are .v, .sv, .vhd/.vhdl, .lpf and .hex. New file never
//! overwrites. Save a dirty ecp5.toml tab before submitting. Success refreshes
//! inputs and the clean manifest tab, opens the new tab and requires a rebuild.
//! Both traces receive `file new PATH ok` or `file new PATH failed REASON`, and
//! `manifest NOTE ok` for each membership change. Paths use DisplayPaths and
//! ambiguous paths are JSON quoted. This command is project-script only.
//!
//! Project scripts accept `memory-image {"path":"fw.hex","use":true|false}`
//! for existing .hex/.mem/.init files. The tree's Use as BRAM image / Stop using
//! as BRAM image actions confirm the post-route ecpbram rule; the script's use
//! boolean supplies that choice. Resolve dirty ecp5.toml/ecpbram.txt tabs first.
//! The library updates memory.images, explicit sources.files and ecpbram.txt;
//! success refreshes inputs and clean membership tabs and requires a rebuild.
//! Both records receive `file memory-image PATH on|off ok` or
//! `file memory-image PATH on|off failed REASON`, plus `manifest NOTE ok` for
//! each membership change, with DisplayPaths and JSON quoting as above.
//!
//! The UART console adds a `# uart` line whenever the window takes new bytes
//! from the run: tab-separated, the seconds since the design was ready, the
//! run's cycle then, and the bytes, printable ASCII as it is and `\r`, `\n`,
//! `\t`, `\\`, `\xHH` for the rest (src/app.rs, `record_text`).
//! A button held or let go (on the 3D board, in the rows or by `button`)
//! adds `# button N down|up <seconds since ready> <cycle>`, tab-separated.
//! When the window first sees a ready design's switch positions and each
//! time they change, including after going back, it adds
//! `# switches <seconds since ready> <cycle> <four 0/1, sw[0] first>`,
//! tab-separated. These are the runner's reported positions.
//! Each click or `switch N on|off` request adds
//! `# switch requested <seconds since ready> <cycle> <N 1..4> on|off`,
//! tab-separated. The lever is drawn at the requested position until the
//! runner reports it; going back, seeking, opening or closing a design
//! clears requests. The `# switches` lines remain the reported positions.
//! As fast as it goes, every two seconds a `# busy` line gives how busy the
//! wires that changed are and how much their changes glow (src/changes.rs
//! `BUSY_STEP`), as quantiles.
//!
//! Waves adds `# waves <seconds since ready> …` notes, tab-separated:
//! opened/closed; each add's source, net/name and result (freshness is
//! pending until its first row); each watch command's id and wire count;
//! freshness from rows, sampled or NOT_SAMPLED; every watch-status change
//! with the published edge. Once a second while watching, `taken` gives
//! received lists (id: runs, edges), the kept half-open edge span, dropped
//! rows by reason and NOT_SAMPLED counts per net. While open, `drawn` gives
//! visible lines, the inclusive edge span, present, end and following.
//!
//! Cross-probing adds `# probe source FILE:LINE:COL -> N nets: NAMES` (or
//! the same no-match message as the status),
//! `# probe map net N (FIRST_NAME +ALIASES) -> FILE:LINE[,FILE:LINE]`,
//! and `# probe index SECONDS NETS LOCATIONS` after ready. The extra Names
//! collection pass is timed separately as `# probe load-pass SECONDS`.
//! Source paths are relative; library and unknown paths never enter the
//! index. Project scripts also accept `source FILE LINE COL`, using the
//! editor buffer and the same token-first resolution as an editor click.
//!
//! `--tour` records the same `# tour N STEP CAPTION` notes in the window and
//! project records, including `6 done` for the closing caption.
//! Step notes use the entry caption; `# tour caption N CAPTION` records a
//! change within a step, such as the chip becoming ready. A missing
//! executable adds `# tour skip build REASON` and the fallback caption;
//! the prebuilt design uses the project's source list. The map pointer's hold
//! adds `# tour pointer on probed wire X Y` (map shares) before the ordinary `# probe map` and
//! project's `open FILE:LINE`. Completion adds `# tour done SECONDS`, physical
//! input adds `# tour stopped at STEP`, and a deadline or error adds
//! `# tour failed STEP REASON`. Paths in tour notes are relative.
//! The tour requires a ready chip's drawing read back and an increase in this
//! record's lit_in_time count, a nonempty source probe, a real editor jump,
//! and received Waves rows shown in its panel. Timers alone cannot pass them.
//! Tours default to `tour/last-window.tsv` and `tour/last-project.tsv` beside
//! recent.txt; ECP5_GUI_TRACE and ECP5_PROJECT_TRACE override these paths.
//!
//! The 3D board (src/board_view.rs) adds `# board …` lines, of what the GPU
//! drew into the board's picture (whether the window then showed that frame
//! is in the frame columns): for each of its drawings read back, the LEDs
//! D0..D7 it was to show lit, those the GPU drew lit and those it drew dark
//! (each as eight 0/1; neither: out of sight), `ok` or `WRONG n`; with new
//! LED values, how many of the window's frames after they came it was drawn
//! and how long after the drawing of the LED values before (how often the
//! LEDs change on the screen); running totals. A `# layout` line gives,
//! whenever they move, where the window, the board and the die's map are,
//! in points.
use crate::{
    changes::Motion,
    gpu_map::{Measurements, Readback},
};
use anyhow::{Context, Result};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed},
    time::Instant,
};

/// The map's fifths, left to right, for the lag.
pub const STRIPS: usize = 5;

/// A wire's mark for being drawn lit in frame `frame`: the earliest frame
/// is the greatest mark (0: not drawn lit since it last changed).
pub fn mark(frame: u32) -> u32 {
    u32::MAX - frame
}

/// What the map drew: the drawing records into it as it draws, from its
/// threads too.
pub struct Drawn {
    /// Each wire: the first frame it was drawn lit since it last changed,
    /// as a `mark`.
    lit: Vec<AtomicU32>,
    /// Each wire: the values its pieces were drawn with in the drawing read
    /// (with 0/1), a bit each for 0, 1 and any other; the GPU marks those
    /// of the pixels that show the wire, the CPU those of its pieces drawn.
    seen: Vec<AtomicU32>,
    /// Pieces drawn lit this frame (the CPU's drawing), or listed to draw
    /// changes on in the drawing read (the GPU's).
    lit_pieces: AtomicUsize,
    /// Whether there is news of what was drawn: each frame the CPU draws,
    /// or when what the GPU drew is read back; the time (f64 bits) and the
    /// number of the frame it is of, to judge what was due by then.
    fresh: AtomicBool,
    at: std::sync::atomic::AtomicU64,
    of: AtomicU32,
    /// The frame being drawn.
    frame: AtomicU32,
    /// The drawing read gave only the top wire its glow: covered wires
    /// have no glow to judge either, not just no 0/1 value.
    pixel_glow: bool,
    /// Each wire: whether it has a piece in sight, the fifth of the map its
    /// first piece in sight is in, and the least share of its net's length
    /// of its pieces in sight (where a change reaches it first).
    in_sight: Vec<bool>,
    strip: Vec<u8>,
    share: Vec<f32>,
}

impl Drawn {
    fn new(wires: usize) -> Self {
        Self {
            lit: (0..wires).map(|_| AtomicU32::new(0)).collect(),
            seen: (0..wires).map(|_| AtomicU32::new(0)).collect(),
            lit_pieces: AtomicUsize::new(0),
            fresh: AtomicBool::new(false),
            at: std::sync::atomic::AtomicU64::new(0),
            of: AtomicU32::new(0),
            frame: AtomicU32::new(0),
            pixel_glow: false,
            in_sight: vec![false; wires],
            strip: vec![0; wires],
            share: vec![0.0; wires],
        }
    }

    /// A piece of wire `v` drawn lit, in the frame being drawn.
    pub fn lit(&self, v: u32) {
        if let Some(l) = self.lit.get(v as usize) {
            let m = mark(self.frame.load(Relaxed));
            if l.load(Relaxed) < m {
                l.fetch_max(m, Relaxed);
            }
            self.lit_pieces.fetch_add(1, Relaxed);
        }
    }

    /// The colours of the wires are drawn again: what was drawn before is
    /// no longer shown.
    pub fn colours_again(&self) {
        for seen in &self.seen {
            seen.store(0, Relaxed);
        }
    }

    /// A step of a piece of wire `v` drawn with value `x`.
    pub fn value(&self, v: u32, x: u8) {
        if let Some(seen) = self.seen.get(v as usize) {
            seen.fetch_or(1 << x.min(2), Relaxed);
        }
    }

    /// The least and greatest value wire `i` was drawn with (2 for any
    /// other than 0 and 1); none: 255 and 0.
    fn values_of(&self, i: usize) -> (u8, u8) {
        match self.seen[i].load(Relaxed) & 7 {
            0 => (u8::MAX, 0),
            b => (b.trailing_zeros() as u8, 31 - b.leading_zeros() as u8),
        }
    }

    /// What the CPU drew this frame is all recorded.
    pub fn drawn(&self, now: f64) {
        self.at.store(now.to_bits(), Relaxed);
        self.of.store(self.frame.load(Relaxed), Relaxed);
        self.fresh.store(true, Relaxed);
    }

    /// What the GPU drew, read back: each wire's mark, and the values each
    /// was drawn with, taken over as they are (a million wires each time).
    pub fn load(&mut self, read: Readback) {
        if read.lit.len() != self.lit.len() || read.seen.len() != self.seen.len() {
            return;
        }
        self.at.store(read.at.to_bits(), Relaxed);
        self.of.store(read.of, Relaxed);
        self.pixel_glow = read.pixel_glow;
        self.lit = atomics(read.lit);
        self.seen = atomics(read.seen);
        self.lit_pieces.store(read.listed as usize, Relaxed);
        self.fresh.store(true, Relaxed);
    }

    /// The pieces in sight are these: each one's wire, where it is across
    /// the map (0 at the left edge, 1 at the right) and the share of its
    /// net's length a change reaches it at.
    pub fn sight(&mut self, pieces: impl Iterator<Item = (u32, f32, f32)>) {
        self.in_sight.iter_mut().for_each(|s| *s = false);
        for (v, x, share) in pieces {
            let i = v as usize;
            if let Some(s) = self.in_sight.get_mut(i) {
                if !*s {
                    *s = true;
                    self.strip[i] =
                        ((x.clamp(0.0, 1.0) * STRIPS as f32) as usize).min(STRIPS - 1) as u8;
                    self.share[i] = share;
                } else {
                    self.share[i] = self.share[i].min(share);
                }
            }
        }
    }
}

const _: () = assert!(std::mem::align_of::<AtomicU32>() == std::mem::align_of::<u32>());

/// Words as atomics, without copying them.
fn atomics(words: Vec<u32>) -> Vec<AtomicU32> {
    let mut words = std::mem::ManuallyDrop::new(words);
    // SAFETY: AtomicU32 has the same size and bit validity as u32, and here
    // the same alignment (checked above); the vector's memory is handed over.
    unsafe {
        Vec::from_raw_parts(
            words.as_mut_ptr().cast::<AtomicU32>(),
            words.len(),
            words.capacity(),
        )
    }
}

/// A change come in, to be drawn: lit by `by` (seconds on the window's
/// clock), and with 0/1 every piece with `value` by then; when it came, and
/// in time for which frame.
#[derive(Clone, Copy)]
struct Due {
    came: f64,
    spread: f64,
    frame: u32,
    by: f64,
    value: u8,
    lit: bool,
}

/// Which wires had a change judged with 0/1 shown (1), and of them those
/// whose value was on the screen when one was (2), with how many of each.
#[derive(Default)]
struct Valued {
    of: Vec<u8>,
    wires: u64,
    seen: u64,
}

impl Valued {
    /// A change of wire `i` judged with 0/1 shown, its value on the screen
    /// or not.
    fn judged(&mut self, i: usize, seen: bool) {
        if i >= self.of.len() {
            self.of.resize(i + 1, 0);
        }
        let was = self.of[i];
        let now = was.max(1 + u8::from(seen));
        self.wires += u64::from(was == 0);
        self.seen += u64::from(was < 2 && now == 2);
        self.of[i] = now;
    }
}

/// The run as the window shows it, for a line of the record.
pub struct Shown {
    /// The window's own time this frame, milliseconds: all of it before the
    /// record, taking the values in, and the record of the frame before.
    pub ui_ms: f64,
    pub update_ms: f64,
    pub trace_ms: f64,
    pub running: bool,
    pub rate: Option<f64>,
    pub cycles: u64,
    pub mhz: f64,
    pub values: bool,
    pub zoom: f32,
    /// The map's height in points, and whether a net is shown whole (the
    /// one pointed at, or kept).
    pub map_height: f32,
    pub pointing: bool,
    /// Frames of the map egui dropped so far (prepared, not drawn).
    pub dropped: u32,
    pub gpu: Measurements,
}

pub struct Trace {
    out: Lines,
    pub drawn: Drawn,
    /// Each wire's change due, if one is, and the wires with one: a list
    /// and an array, as a hundred thousand changes a frame may come in.
    due: Vec<Option<Due>>,
    waiting: Vec<u32>,
    came: usize,
    /// When each frame was drawn (on the window's clock).
    times: Vec<f64>,
    /// The wires whose marks of being drawn lit are to start again: a
    /// change came that is now waited on.
    resets: Vec<u32>,
    /// The frame of the drawings read last, the marks of the frames read
    /// with it (since the read before), and how far the wires waited on
    /// are judged against it: a third of them a frame, as a read comes
    /// every few frames (a million wires judged in one frame would make it
    /// late); none once all are. A read that comes while they are being
    /// judged has the rest judged against it, then all of them again.
    read: u32,
    window: (u32, u32),
    judging: Option<usize>,
    again: bool,
    /// Since the start: changes drawn lit in time and not; values shown in
    /// time and not, and changes whose wire was nowhere on the screen in the
    /// frame read (under other wires everywhere), its value not judged.
    lit: (u64, u64),
    values: (u64, u64),
    hidden: u64,
    glow_hidden: u64,
    glow_mode: Option<(crate::gpu_map::GlowChoice, crate::gpu_map::Glow)>,
    /// Changes whose wire left the view before they were due: not judged.
    left: u64,
    /// Which wires had a change judged with 0/1 shown, and whether one's
    /// value was on the screen.
    valued: Valued,
    /// How many times each wire was missed.
    missed: std::collections::BTreeMap<u32, u32>,
    last: Option<Instant>,
    ready: Option<Instant>,
}

impl Trace {
    /// The drawing read back and changes judged lit, also used by the tour.
    pub fn progress(&self) -> (u32, u64) {
        (self.read, self.lit.0)
    }

    pub fn new(path: &Path) -> Result<Self> {
        let mut out =
            Lines::new(File::create(path).with_context(|| format!("creating {}", path.display()))?);
        writeln!(
            out,
            "t\tfresh\tframe_ms\tui_ms\tupdate_ms\tdraw_ms\ttrace_ms\trunning\trate\tcycle\tmhz\tvalues\tzoom\tcame\tlit_wires\tlit_pieces\tlit_in_time\tlit_missed\tvalue_in_time\tvalue_missed\tleft_view\tdue\tlag\tmap_h\tnet\tvalue_unseen\tdropped\tvalued_wires\tvalue_seen_wires\tgpu_ms\tgpu_upload_ms\tgpu_list_ms\tgpu_draw_ms\tupload_kb\tfull_upload\tmap_frame\tgpu_frame\tupload_frame\tglow_unseen\tgpu_mask_ms\tgpu_splat_ms\tgpu_strips_ms"
        )?;
        Ok(Self {
            out,
            drawn: Drawn::new(0),
            due: Vec::new(),
            waiting: Vec::new(),
            came: 0,
            times: Vec::new(),
            resets: Vec::new(),
            read: 0,
            window: (0, 0),
            judging: None,
            again: false,
            lit: (0, 0),
            hidden: 0,
            glow_hidden: 0,
            glow_mode: None,
            left: 0,
            valued: Valued::default(),
            missed: Default::default(),
            values: (0, 0),
            last: None,
            ready: None,
        })
    }

    /// The design's wires, once loaded; whether the record of what was
    /// drawn starts again (and with it what is in sight, to be found anew:
    /// the wires' values may come a frame after the design, when the view
    /// was already found).
    pub fn wires(&mut self, wires: usize) -> bool {
        if self.drawn.lit.len() != wires {
            // The changes of another design still waited on are out of the
            // view now: not judged.
            self.left += self.waiting.len() as u64;
            self.waiting.clear();
            self.due.clear();
            self.resets.clear();
            (self.judging, self.again) = (None, false);
            self.read = 0;
            self.window = (0, 0);
            self.drawn = Drawn::new(wires);
            self.valued = Valued::default();
            self.glow_mode = None;
            self.ready = Some(Instant::now());
            return true;
        }
        false
    }

    /// Frame `frame` is being drawn, at `now` on the window's clock.
    pub fn drawing(&mut self, frame: u32, now: f64) {
        let i = frame as usize;
        if self.times.len() <= i {
            self.times.resize(i + 1, f64::NAN);
        }
        self.times[i] = now;
        self.drawn.frame.store(frame, Relaxed);
    }

    /// A change of wire `v` came in at `now`, to be drawn first in frame
    /// `frame` and shown as `motion` says; `value` is the value it has now.
    /// Only wires in sight count. A wire whose change before is still
    /// waited on waits on from when that came, until the new one is due: a
    /// wire changing every frame is judged by whether it was drawn lit at
    /// all since, and how long after the first change it was.
    pub fn came(&mut self, v: u32, value: u8, now: f64, motion: Motion, frame: u32) {
        if !self
            .drawn
            .in_sight
            .get(v as usize)
            .copied()
            .unwrap_or(false)
        {
            return;
        }
        self.came += 1;
        // Spread all the way, with a few frames to spare.
        let by = now + motion.spread as f64 + 0.25;
        let i = v as usize;
        if i >= self.due.len() {
            self.due.resize(i + 1, None);
        }
        match &mut self.due[i] {
            Some(d) => {
                d.by = by;
                d.value = value;
            }
            None => {
                self.waiting.push(v);
                self.due[i] = Some(Due {
                    came: now,
                    spread: motion.spread as f64,
                    frame,
                    by,
                    value,
                    lit: false,
                });
                // Its marks start again, here and on the GPU.
                self.drawn.lit[i].store(0, Relaxed);
                self.resets.push(v);
            }
        }
    }

    /// The wires whose marks are to start again on the GPU.
    pub fn take_resets(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.resets)
    }

    /// The end of a frame: what was drawn checked against what is due, and
    /// a line written.
    pub fn frame(&mut self, draw_ms: f64, shown: &Shown) -> Result<()> {
        if let Some(mode) = shown.gpu.glow {
            if self.glow_mode != Some((mode.choice, mode.mode)) {
                self.glow_mode = Some((mode.choice, mode.mode));
                self.note(&format!(
                    "map glow {} {} {} frame {}",
                    mode.choice.name(),
                    mode.mode.name(),
                    mode.listed,
                    mode.frame
                ))?;
            }
        }
        let at = Instant::now();
        let frame_ms = self.last.map_or(0.0, |l| (at - l).as_secs_f64() * 1e3);
        self.last = Some(at);
        let fresh = self.drawn.fresh.swap(false, Relaxed);
        // What was due by the frame read is judged: not what came after.
        let seen_at = f64::from_bits(self.drawn.at.load(Relaxed));
        let of = self.drawn.of.load(Relaxed);
        if fresh {
            self.window = (mark(of), mark(self.read));
            self.read = of;
            match self.judging {
                None => self.judging = Some(0),
                Some(_) => self.again = true,
            }
        }
        // The wires judged this frame first drawn lit in the drawings read
        // (only a wire waited on has its mark started again, so only those
        // can be).
        let mut lit_wires = 0;
        let window = self.window.0..self.window.1;
        let lit_pieces = self.drawn.lit_pieces.swap(0, Relaxed);
        let mut missed = Vec::new();
        let mut lag = [(0u64, 0.0f64, 0.0f64); STRIPS];
        let drawn = &self.drawn;
        let times = &self.times;
        let (lit, values, hidden, left, valued) = (
            &mut self.lit,
            &mut self.values,
            &mut self.hidden,
            &mut self.left,
            &mut self.valued,
        );
        let due = &mut self.due;
        // Without news of what was drawn nothing is judged: what the GPU
        // drew comes a few frames late.
        let waiting = &mut self.waiting;
        let (start, end) = match self.judging {
            Some(k) => (
                k.min(waiting.len()),
                (k + (waiting.len() / 3).max(50_000)).min(waiting.len()),
            ),
            None => (0, 0),
        };
        let mut judge = |v: u32| -> bool {
            let i = v as usize;
            let Some(d) = due[i].as_mut() else {
                return false;
            };
            // Judged, it is no longer due.
            let judged = |due: &mut Vec<Option<Due>>| {
                due[i] = None;
                false
            };
            let (lo, hi) = drawn.values_of(i);
            let no_pixel = lo == u8::MAX && hi == 0;
            // A covered wire in pixel mode was not drawn lit. Do not turn
            // that into a successful mark, or a miss of a visible change.
            // Only a drawing made since the change came can retire it.
            if drawn.pixel_glow && no_pixel && of >= d.frame && drawn.in_sight[i] {
                self.glow_hidden += 1;
                if shown.values {
                    *hidden += 1;
                    valued.judged(i, false);
                }
                return judged(due);
            }
            // Drawn lit in a frame since it came (a mark of a frame before
            // is of a change before).
            let m = drawn.lit[i].load(Relaxed);
            if window.contains(&m) {
                lit_wires += 1;
            }
            let first = (m != 0).then(|| u32::MAX - m).filter(|&f| f >= d.frame);
            if let (false, Some(f)) = (d.lit, first) {
                d.lit = true;
                lit.0 += 1;
                // Late after it was to reach the wire where it reaches it first.
                let reach = d.came + d.spread * drawn.share[i] as f64;
                let ms = (times.get(f as usize).copied().unwrap_or(f64::NAN) - reach) * 1e3;
                if ms.is_finite() {
                    let l = &mut lag[drawn.strip[i] as usize];
                    l.0 += 1;
                    l.1 += ms.max(0.0);
                    l.2 = l.2.max(ms);
                }
            }
            // Drawn with no value at all: the drawing read has the wire
            // nowhere on the screen (the GPU marks a value where a pixel of
            // the wire's shows), so there is no value to judge.
            let unseen = shown.values && no_pixel;
            let shows = !shown.values || unseen || lo == d.value && hi == d.value;
            if d.lit && shows {
                if unseen {
                    *hidden += 1;
                } else if shown.values {
                    values.0 += 1;
                }
                if shown.values {
                    valued.judged(i, !unseen);
                }
                return judged(due);
            }
            if seen_at > d.by {
                // The view moved and the wire is out of it: nothing to judge.
                if !drawn.in_sight.get(v as usize).copied().unwrap_or(false) {
                    *left += 1;
                    return judged(due);
                }
                if !d.lit {
                    lit.1 += 1;
                }
                if !shows {
                    values.1 += 1;
                }
                if shown.values {
                    valued.judged(i, !unseen);
                }
                missed.push((v, d.lit, shows, d.came, d.by));
                return judged(due);
            }
            true
        };
        // Those judged and still due keep their places.
        let mut kept = start;
        for k in start..end {
            let v = waiting[k];
            if judge(v) {
                waiting[kept] = v;
                kept += 1;
            }
        }
        waiting.drain(kept..end);
        self.judging = match self.judging {
            Some(_) if kept < self.waiting.len() => Some(kept),
            Some(_) if std::mem::take(&mut self.again) => Some(0),
            _ => None,
        };
        let t = self.ready.map_or(0.0, |r| (at - r).as_secs_f64());
        let rate = shown.rate.map_or("max".to_string(), |r| format!("{r}"));
        let lag = lag
            .iter()
            .map(|(n, sum, max)| format!("{n}/{sum:.0}/{max:.0}"))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            self.out,
            "{t:.3}\t{}\t{frame_ms:.1}\t{:.1}\t{:.1}\t{draw_ms:.1}\t{:.1}\t{}\t{rate}\t{}\t{:.4}\t{}\t{:.2}\t{}\t{lit_wires}\t{lit_pieces}\t{}\t{}\t{}\t{}\t{}\t{}\t{lag}\t{:.1}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}\t{}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}",
            u8::from(fresh),
            shown.ui_ms,
            shown.update_ms,
            shown.trace_ms,
            u8::from(shown.running),
            shown.cycles,
            shown.mhz,
            u8::from(shown.values),
            shown.zoom,
            self.came,
            self.lit.0,
            self.lit.1,
            self.values.0,
            self.values.1,
            self.left,
            self.waiting.len(),
            shown.map_height,
            u8::from(shown.pointing),
            self.hidden,
            shown.dropped,
            self.valued.wires,
            self.valued.seen,
            shown.gpu.gpu.map_or(-1.0, |g| g.ms),
            shown.gpu.gpu.map_or(-1.0, |g| g.upload_ms),
            shown.gpu.gpu.map_or(-1.0, |g| g.list_ms),
            shown.gpu.gpu.map_or(-1.0, |g| g.draw_ms),
            shown.gpu.upload_bytes as f64 / 1024.0,
            u8::from(shown.gpu.full_upload),
            self.drawn.frame.load(Relaxed),
            shown.gpu.gpu.map_or(-1, |g| i64::from(g.frame)),
            shown.gpu.upload_frame.map_or(-1, i64::from),
            self.glow_hidden,
            shown.gpu.gpu.map_or(-1.0, |g| g.mask_ms),
            shown.gpu.gpu.map_or(-1.0, |g| g.splat_ms),
            shown.gpu.gpu.map_or(-1.0, |g| g.strips_ms),
        )?;
        for &(v, ..) in &missed {
            *self.missed.entry(v).or_default() += 1;
        }
        for (v, lit, shows, came, by) in missed.iter().take(5) {
            writeln!(
                self.out,
                "# missed wire {v}: drawn lit {lit}, drawn with its value {shows}; came {came:.3}, due by {by:.3}, judged on the drawing of {seen_at:.3}"
            )?;
        }
        self.came = 0;
        self.out.flush()?;
        Ok(())
    }
}

impl Trace {
    /// The wires missed most, with how many times, most first.
    pub fn most_missed(&self, n: usize) -> Vec<(u32, u32)> {
        let mut m: Vec<(u32, u32)> = self.missed.iter().map(|(&v, &k)| (v, k)).collect();
        m.sort_by_key(|&(v, k)| (std::cmp::Reverse(k), v));
        m.truncate(n);
        m
    }

    /// The runner's reported SW1 positions, in sw[0..3] order.
    pub fn switches(&mut self, seconds: f64, cycle: u64, on: [bool; 4]) -> Result<()> {
        let bits: String = on
            .into_iter()
            .map(|on| if on { '1' } else { '0' })
            .collect();
        self.note(&format!("switches\t{seconds:.3}\t{cycle}\t{bits}"))
    }

    /// A line of explanation, as a comment, including the window's load
    /// stages and the first drawing of the die (seconds since opening).
    pub fn note(&mut self, line: &str) -> Result<()> {
        writeln!(self.out, "# {line}")?;
        Ok(())
    }
}

/// The record's lines, written to its file by a thread of their own: a
/// frame only hands them over (a busy disk once held a frame 600 ms in a
/// write). Flushing hands over what was written since; the lines are all in
/// the file once this is dropped.
struct Lines {
    text: Vec<u8>,
    to: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    writer: Option<std::thread::JoinHandle<()>>,
}

impl Lines {
    fn new(file: File) -> Self {
        let (to, from) = std::sync::mpsc::channel::<Vec<u8>>();
        let writer = std::thread::spawn(move || {
            let mut file = BufWriter::new(file);
            for text in from {
                if let Err(e) = file.write_all(&text).and_then(|_| file.flush()) {
                    eprintln!("trace: {e}");
                    return;
                }
            }
        });
        Self {
            text: Vec::new(),
            to: Some(to),
            writer: Some(writer),
        }
    }
}

impl Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.text.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.text.is_empty() {
            if let Some(to) = &self.to {
                let _ = to.send(std::mem::take(&mut self.text));
            }
        }
        Ok(())
    }
}

impl Drop for Lines {
    fn drop(&mut self) {
        let _ = self.flush();
        self.to = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

/// A script's commands, by the second after the design is ready, in the
/// order they run (see `order`).
pub fn script(path: &Path) -> Result<Vec<(f64, String)>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = crate::new_project::uncomment(line).trim();
        if line.is_empty() {
            continue;
        }
        let (t, command) = line
            .split_once(char::is_whitespace)
            .context("a time and a command")?;
        crate::new_project::Command::parse(command.trim())?;
        lines.push((
            t.parse::<f64>()
                .with_context(|| format!("time in {line:?}"))?,
            command.trim().to_string(),
        ));
    }
    Ok(order(lines))
}

/// `added` put among the lines of a script still to run (`lines`), each by
/// its time among those up to the next `open`, `build-demo` or `close`
/// (later lines are timed from another design); a line due at the same time
/// as one already there runs after it.
pub fn insert(lines: &mut Vec<(f64, String)>, added: Vec<(f64, String)>) {
    for line in added {
        let at = lines
            .iter()
            .position(|(t, c)| {
                *t > line.0
                    || matches!(
                        c.split_whitespace().next(),
                        Some(
                            "open"
                                | "build-demo"
                                | "close"
                                | "new-project"
                                | "start-demo"
                                | "project-options"
                                | "create-project"
                                | "cancel-project"
                        )
                    )
            })
            .unwrap_or(lines.len());
        lines.insert(at, line);
    }
}

/// The order a script runs in. Opens and builds start another design's
/// clock; closes return to the start screen's clock. Keep the file's order
/// across these actions and sort only the commands between them by time.
fn order(mut lines: Vec<(f64, String)>) -> Vec<(f64, String)> {
    let switches = |command: &str| {
        matches!(
            command.split_whitespace().next(),
            Some(
                "open"
                    | "build-demo"
                    | "close"
                    | "new-project"
                    | "start-demo"
                    | "project-options"
                    | "create-project"
                    | "cancel-project"
            )
        )
    };
    let by_time = |a: &(f64, String), b: &(f64, String)| a.0.total_cmp(&b.0);
    if !lines.iter().any(|(_, c)| {
        matches!(
            c.split_whitespace().next(),
            Some(
                "open"
                    | "build-demo"
                    | "new-project"
                    | "start-demo"
                    | "project-options"
                    | "create-project"
                    | "cancel-project"
            )
        )
    }) {
        lines.sort_by(by_time);
        return lines;
    }
    let mut ordered = Vec::with_capacity(lines.len());
    let mut between = Vec::new();
    for line in lines {
        if switches(&line.1) {
            between.sort_by(by_time);
            ordered.append(&mut between);
            ordered.push(line);
        } else {
            between.push(line);
        }
    }
    between.sort_by(by_time);
    ordered.append(&mut between);
    ordered
}

#[cfg(test)]
mod tests {
    use super::{insert, order, Valued};

    #[test]
    fn switch_positions_are_recorded_in_board_order() {
        let path =
            std::env::temp_dir().join(format!("ecp5_switch_trace_{}.tsv", std::process::id()));
        let mut trace = super::Trace::new(&path).unwrap();
        trace.switches(0.0, 0, [false; 4]).unwrap();
        trace
            .switches(1.23456, 42, [true, false, true, false])
            .unwrap();
        trace.switches(2.0, 17, [false, true, false, true]).unwrap();
        drop(trace);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        let notes: Vec<_> = text.lines().filter(|line| line.starts_with('#')).collect();
        assert_eq!(
            notes,
            [
                "# switches\t0.000\t0\t0000",
                "# switches\t1.235\t42\t1010",
                "# switches\t2.000\t17\t0101",
            ]
        );
    }

    fn lines(script: &[(f64, &str)]) -> Vec<(f64, String)> {
        script.iter().map(|&(t, c)| (t, c.to_string())).collect()
    }

    fn shown() -> super::Shown {
        super::Shown {
            ui_ms: 0.0,
            update_ms: 0.0,
            trace_ms: 0.0,
            running: false,
            rate: None,
            cycles: 0,
            mhz: 0.0,
            values: false,
            zoom: 1.0,
            map_height: 0.0,
            pointing: false,
            dropped: 0,
            gpu: super::Measurements::default(),
        }
    }

    #[test]
    fn only_pixel_glow_exempts_covered_changes_and_visible_misses_still_count() {
        use crate::gpu_map::{Glow, GlowChoice, GlowMode, Readback};
        let path = std::env::temp_dir().join(format!("ecp5_glow_trace_{}.tsv", std::process::id()));
        for values in [false, true] {
            for pixel in [false, true] {
                let mut trace = super::Trace::new(&path).unwrap();
                let mut shown = shown();
                shown.values = values;
                trace.wires(3);
                trace
                    .drawn
                    .sight([(0, 0.2, 0.0), (1, 0.5, 0.0), (2, 0.8, 0.0)].into_iter());
                trace.drawing(1, 0.0);
                for wire in 0..3 {
                    trace.came(
                        wire,
                        1,
                        0.0,
                        super::Motion {
                            spread: 0.0,
                            fade: 0.3,
                        },
                        1,
                    );
                }
                trace.drawn.load(Readback {
                    at: 1.0,
                    of: 1,
                    lit: vec![0, super::mark(1), 0],
                    seen: vec![0, 2, 2],
                    listed: 3,
                    pixel_glow: pixel,
                });
                shown.gpu.glow = Some(GlowMode {
                    frame: 1,
                    choice: GlowChoice::Auto,
                    mode: if pixel { Glow::Top } else { Glow::Quads },
                    listed: 3,
                });
                trace.frame(0.0, &shown).unwrap();
                assert_eq!(trace.glow_hidden, u64::from(pixel));
                assert_eq!(trace.lit, (1, if pixel { 1 } else { 2 }));
                assert_eq!(trace.values.1, 0);
                assert_eq!(trace.hidden, u64::from(pixel && values));
                assert!(trace.waiting.is_empty());
                // The same mode is noted once, then its switch once.
                trace.frame(0.0, &shown).unwrap();
                shown.gpu.glow = Some(GlowMode {
                    frame: 2,
                    choice: GlowChoice::Auto,
                    mode: if pixel { Glow::Quads } else { Glow::Top },
                    listed: 5,
                });
                trace.frame(0.0, &shown).unwrap();
                drop(trace);
                let text = std::fs::read_to_string(&path).unwrap();
                let notes: Vec<_> = text
                    .lines()
                    .filter(|l| l.starts_with("# map glow "))
                    .collect();
                assert_eq!(
                    notes,
                    [
                        format!(
                            "# map glow auto {} 3 frame 1",
                            if pixel { "top" } else { "quads" }
                        ),
                        format!(
                            "# map glow auto {} 5 frame 2",
                            if pixel { "quads" } else { "top" }
                        ),
                    ]
                );
            }
        }
        // A read from before a change came cannot dismiss it as covered.
        let mut trace = super::Trace::new(&path).unwrap();
        trace.wires(1);
        trace.drawn.sight(std::iter::once((0, 0.5, 0.0)));
        trace.drawing(2, 0.0);
        trace.came(
            0,
            1,
            0.0,
            super::Motion {
                spread: 0.0,
                fade: 0.3,
            },
            2,
        );
        trace.drawn.load(Readback {
            at: 0.0,
            of: 1,
            lit: vec![0],
            seen: vec![0],
            listed: 1,
            pixel_glow: true,
        });
        trace.frame(0.0, &shown()).unwrap();
        assert_eq!(trace.glow_hidden, 0);
        assert_eq!(trace.waiting, [0]);
        drop(trace);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn mask_marks_are_judged_and_notes_name_the_request_and_choice() {
        use crate::gpu_map::{Glow, GlowChoice, GlowMode, Readback};
        let path = std::env::temp_dir().join(format!("ecp5_mask_trace_{}.tsv", std::process::id()));
        let mut trace = super::Trace::new(&path).unwrap();
        let mut shown = shown();
        shown.values = true;
        trace.wires(2);
        trace
            .drawn
            .sight([(0, 0.2, 0.0), (1, 0.8, 0.0)].into_iter());
        trace.drawing(1, 0.0);
        for wire in 0..2 {
            trace.came(
                wire,
                1,
                0.0,
                super::Motion {
                    spread: 0.0,
                    fade: 0.3,
                },
                1,
            );
        }
        trace.drawn.load(Readback {
            at: 1.0,
            of: 1,
            lit: vec![super::mark(1); 2],
            seen: vec![0, 2],
            listed: 2,
            pixel_glow: false,
        });
        shown.gpu.glow = Some(GlowMode {
            frame: 1,
            choice: GlowChoice::Auto,
            mode: Glow::Mask,
            listed: 2,
        });
        trace.frame(0.0, &shown).unwrap();
        assert_eq!(trace.lit, (2, 0));
        assert_eq!(trace.glow_hidden, 0);
        assert_eq!(trace.values, (1, 0));
        assert_eq!(trace.hidden, 1, "the covered value is still unseen");
        shown.gpu.glow = Some(GlowMode {
            frame: 2,
            choice: GlowChoice::Mask,
            mode: Glow::Mask,
            listed: 2,
        });
        trace.frame(0.0, &shown).unwrap();
        drop(trace);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert!(text.contains("# map glow auto mask 2 frame 1\n"));
        assert!(text.contains("# map glow mask mask 2 frame 2\n"));
    }

    /// The new diagnostics stay at the end, and missing GPU results are
    /// distinct from passes that took zero time.
    #[test]
    fn gpu_measurements_are_appended_to_the_record() {
        let path = std::env::temp_dir().join(format!("ecp5_gui_trace_{}.tsv", std::process::id()));
        let mut trace = super::Trace::new(&path).unwrap();
        trace.drawing(9, 0.1);
        let mut shown = shown();
        trace.frame(0.0, &shown).unwrap();
        shown.gpu = super::Measurements {
            upload_frame: Some(8),
            upload_bytes: 43 * 1024,
            full_upload: true,
            gpu: Some(crate::gpu_map::GpuTime {
                frame: 6,
                ms: 7.0,
                upload_ms: 1.0,
                list_ms: 2.0,
                draw_ms: 3.0,
                mask_ms: 2.5,
                splat_ms: 1.0,
                strips_ms: 1.5,
            }),
            glow: None,
        };
        trace.frame(0.0, &shown).unwrap();
        drop(trace);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let mut lines = text.lines();
        let header: Vec<_> = lines.next().unwrap().split('\t').collect();
        let start = header
            .iter()
            .position(|&column| column == "gpu_ms")
            .unwrap();
        assert_eq!(header[start - 1], "value_seen_wires");
        assert_eq!(
            &header[start..],
            &[
                "gpu_ms",
                "gpu_upload_ms",
                "gpu_list_ms",
                "gpu_draw_ms",
                "upload_kb",
                "full_upload",
                "map_frame",
                "gpu_frame",
                "upload_frame",
                "glow_unseen",
                "gpu_mask_ms",
                "gpu_splat_ms",
                "gpu_strips_ms",
            ]
        );
        for (line, expected) in lines.zip([
            [
                "-1.000", "-1.000", "-1.000", "-1.000", "0.000", "0", "9", "-1", "-1", "0",
                "-1.000", "-1.000", "-1.000",
            ],
            [
                "7.000", "1.000", "2.000", "3.000", "43.000", "1", "9", "6", "8", "0", "2.500",
                "1.000", "1.500",
            ],
        ]) {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), header.len());
            assert_eq!(&fields[start..], &expected);
        }
    }

    #[test]
    fn a_script_without_open_runs_by_time() {
        let parsed = order(lines(&[
            (5.0, "run"),
            (1.0, "rate max"),
            (3.0, "close"),
            (2.0, "pause"),
        ]));
        assert_eq!(
            parsed,
            lines(&[
                (1.0, "rate max"),
                (2.0, "pause"),
                (3.0, "close"),
                (5.0, "run")
            ])
        );
    }

    #[test]
    fn open_keeps_the_files_order_and_sorts_between() {
        let parsed = order(lines(&[
            (0.0, "open a/top.config"),
            (5.0, "type mandel\\r"),
            (1.0, "run"),
            (60.0, "open b/top.config"),
            (2.0, "button 1 down"),
            (1.0, "run"),
            (30.0, "close"),
            (0.5, "open c/top.config"),
        ]));
        assert_eq!(
            parsed,
            lines(&[
                (0.0, "open a/top.config"),
                (1.0, "run"),
                (5.0, "type mandel\\r"),
                (60.0, "open b/top.config"),
                (1.0, "run"),
                (2.0, "button 1 down"),
                (30.0, "close"),
                (0.5, "open c/top.config"),
            ])
        );
    }

    /// A sweep's views go in by time among the lines for the same design.
    #[test]
    fn inserted_lines_keep_to_their_design() {
        let mut script = lines(&[(12.0, "pause"), (20.0, "open b/top.config"), (1.0, "run")]);
        insert(
            &mut script,
            lines(&[
                (10.0, "view 0.1 0.1 4"),
                (12.0, "view 0.3 0.1 4"),
                (14.0, "view 0.5 0.1 4"),
                (30.0, "view 0.7 0.1 4"),
            ]),
        );
        assert_eq!(
            script,
            lines(&[
                (10.0, "view 0.1 0.1 4"),
                (12.0, "pause"),
                (12.0, "view 0.3 0.1 4"),
                (14.0, "view 0.5 0.1 4"),
                (30.0, "view 0.7 0.1 4"),
                (20.0, "open b/top.config"),
                (1.0, "run"),
            ])
        );
    }

    /// Wires counted once each: judged with 0/1, and seen at least once.
    #[test]
    fn values_judged_are_counted_by_wire() {
        let mut v = Valued::default();
        v.judged(5, false);
        v.judged(5, false);
        assert_eq!((v.wires, v.seen), (1, 0));
        v.judged(5, true);
        v.judged(5, false);
        assert_eq!((v.wires, v.seen), (1, 1));
        v.judged(2, true);
        v.judged(9, false);
        assert_eq!((v.wires, v.seen), (3, 2));
    }
}
