//! The live runner behind the GUI (src/live.rs) on the reference designs,
//! driven by its commands as the window drives it: the UART console, a board
//! button, the LEDs, real-time pacing and the die map.
mod common;
use common::reference;
use ecp5_emu::{
    layout::{SegmentKind, SiteKind},
    live::{self, Class, Command, Design, Shared},
};
use std::{
    sync::{mpsc::Sender, Arc, Mutex},
    time::{Duration, Instant},
};

fn start(design: &str, run: bool) -> (Arc<Mutex<Shared>>, Sender<Command>) {
    start_with_names(design, run, true)
}

fn start_with_names(
    design: &str,
    run: bool,
    with_names: bool,
) -> (Arc<Mutex<Shared>>, Sender<Command>) {
    let root = reference::design(design);
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn(
        Design {
            config: root.join("top.config"),
            names: with_names.then(|| root.join("top_routed.json")),
            run,
        },
        Arc::clone(&shared),
        || {},
    );
    (shared, commands)
}

/// Waits until `done` holds, or fails with what the runner shows.
fn wait(shared: &Mutex<Shared>, seconds: u64, what: &str, done: impl Fn(&Shared) -> bool) {
    let start = Instant::now();
    loop {
        {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
            if done(&s) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(seconds),
                "no {what} after {seconds} s: stage {:?}, cycle {}, text {:?}",
                s.stage,
                s.cycles,
                String::from_utf8_lossy(&s.text)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn text(s: &Shared) -> String {
    String::from_utf8_lossy(&s.text).into_owned()
}

/// PicoRV32 prints its banner to the console, echoes a line in capitals and
/// shows the last byte on the LEDs. The die has every grid position and the
/// logic tiles the CPU uses.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn the_live_console_talks_to_picorv32() {
    let (shared, commands) = start("picorv32_console", true);
    wait(&shared, 120, "prompt", |s| text(s).ends_with("> "));
    {
        let s = shared.lock().unwrap();
        assert_eq!(
            text(&s),
            "PicoRV32 on ECP5\r\nprimes below 2000: 303, 151493 cycles\r\n> "
        );
        let map = s.ready.as_ref().unwrap();
        assert_eq!((map.rows, map.cols, map.cells.len()), (96, 127, 12192));
        let g = &map.geometry;
        assert!(g.logic.len() > 200, "{} logic tiles in use", g.logic.len());
        // A slice in use has a LUT or a flip-flop in use, or carries alone.
        let carry_only = g
            .logic
            .iter()
            .filter(|t| t.luts | t.flip_flops == 0)
            .count();
        assert!(
            carry_only * 50 < g.logic.len(),
            "{carry_only} logic tiles with no LUT or FF"
        );
        // Wires are drawn, every transfer is a dot or a jumper, and every
        // net has a driver at distance 0.
        assert!(g.segments.iter().any(|x| x.kind == SegmentKind::Track));
        assert!(
            g.dots.len() * 2 >= s.ready.as_ref().unwrap().routes,
            "{} dots",
            g.dots.len()
        );
        assert!(g.segments.iter().any(|x| x.d0 == 0.0) && g.longest > 1.0);
        assert_eq!(s.values.len(), g.values.len());
        assert!(s.values.iter().filter(|&&v| v < 2).count() * 10 > s.values.len() * 9);
        assert_eq!(
            g.sites
                .iter()
                .filter(|x| x.used && x.kind == SiteKind::Ebr)
                .count(),
            4
        );
        assert!(map.uart_tx && map.uart_rx && map.leds.iter().all(|&l| l));
        assert!(map.buttons.iter().all(|&b| !b));
        assert!(
            map.cells
                .iter()
                .filter(|c| c.class == Class::Memory && !c.blocks.is_empty())
                .count()
                >= 4
        );
    }
    commands.send(Command::Keys(b"hi\r".to_vec())).unwrap();
    wait(&shared, 60, "echo", |s| text(s).ends_with("HI\r\n> "));
    // The LEDs show the last byte, CR = 0b00001101.
    wait(&shared, 10, "LEDs", |s| {
        s.leds == [true, false, true, true, false, false, false, false].map(Some)
    });
}

/// debounce counts presses of FIRE1 on the LEDs. A press held for 8 ms of
/// board time counts once. In real time the design runs at the board's
/// 25 MHz; without it, faster.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn a_live_button_press_counts_on_debounce() {
    let (shared, commands) = start("debounce", true);
    wait(&shared, 120, "running", |s| s.ready.is_some() && s.running);
    assert!(shared.lock().unwrap().ready.as_ref().unwrap().buttons[1]);
    // Real time: about 25 million cycles per second of wall time.
    let (c0, t0) = (shared.lock().unwrap().cycles, Instant::now());
    std::thread::sleep(Duration::from_millis(1000));
    let rate = (shared.lock().unwrap().cycles - c0) as f64 / t0.elapsed().as_secs_f64();
    assert!(rate < 27e6, "real time ran at {rate:.0} cycles/s");
    wait(&shared, 10, "LEDs at 0", |s| s.leds[0] == Some(false));
    let held = |pressed: bool| {
        commands.send(Command::Button(1, pressed)).unwrap();
        let at = shared.lock().unwrap().cycles;
        wait(&shared, 10, "8 ms of board time", |s| {
            s.cycles > at + 200_000
        });
    };
    held(true);
    held(false);
    wait(&shared, 10, "one press", |s| {
        s.leds == [true, false, false, false, false, false, false, false].map(Some)
    });
    // Without real time this design runs faster than the board.
    commands.send(Command::Rate(None)).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let (c0, t0) = (shared.lock().unwrap().cycles, Instant::now());
    std::thread::sleep(Duration::from_millis(1000));
    let rate = (shared.lock().unwrap().cycles - c0) as f64 / t0.elapsed().as_secs_f64();
    assert!(rate > 30e6, "free running at {rate:.0} cycles/s");
}

/// What the window shows once the run has paused at `cycle`: the cycle,
/// the value of every wire of the map and of the geometry, the LEDs, and the
/// UART text as the window keeps it (`console`: started again after going
/// back).
type Seen = (u64, Vec<u8>, Vec<u8>, [Option<bool>; 8], Vec<u8>);

fn paused_at(shared: &Mutex<Shared>, console: &mut Vec<u8>, cycle: u64) -> Seen {
    let start = Instant::now();
    loop {
        {
            let mut s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
            if let Some(all) = s.console.take() {
                *console = all;
            }
            console.extend(std::mem::take(&mut s.text));
            if !s.running && s.cycles == cycle && !s.lit.is_empty() {
                return (
                    s.cycles,
                    s.lit.clone(),
                    s.values.clone(),
                    s.leds,
                    console.clone(),
                );
            }
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "not paused at cycle {cycle}: at {}",
                s.cycles
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn same(what: &str, a: &Seen, b: &Seen) {
    assert_eq!(a.0, b.0, "{what}: cycle");
    let differ = a.1.iter().zip(&b.1).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{what}: {differ} wires of the map differ");
    let differ = a.2.iter().zip(&b.2).filter(|(x, y)| x != y).count();
    assert_eq!(differ, 0, "{what}: {differ} wires of the geometry differ");
    assert_eq!(a.3, b.3, "{what}: LEDs");
    assert_eq!(
        String::from_utf8_lossy(&a.4),
        String::from_utf8_lossy(&b.4),
        "{what}: console"
    );
}

/// Going back: the run is as it was at that cycle, every wire, the LEDs and
/// the console text; stepping on again gives what it gave before. uart_hello
/// sends a character about every 2170 cycles.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn going_back_returns_the_run_as_it_was() {
    let (shared, commands) = start("uart_hello", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    let mut console = Vec::new();
    let c0 = shared.lock().unwrap().cycles;
    let _ = paused_at(&shared, &mut console, c0);
    commands.send(Command::Step(5_000)).unwrap();
    let a = paused_at(&shared, &mut console, c0 + 5_000);
    commands.send(Command::Step(50_000)).unwrap();
    let b = paused_at(&shared, &mut console, c0 + 55_000);
    assert!(
        b.4.len() > a.4.len() + 10,
        "the console got {:?}",
        String::from_utf8_lossy(&b.4)
    );
    commands.send(Command::Back(50_000)).unwrap();
    same(
        "back to A",
        &paused_at(&shared, &mut console, c0 + 5_000),
        &a,
    );
    assert_eq!(shared.lock().unwrap().ahead, 50_000);
    commands.send(Command::Step(50_000)).unwrap();
    same(
        "on to B",
        &paused_at(&shared, &mut console, c0 + 55_000),
        &b,
    );
    // Back in two goes, on in many.
    commands.send(Command::Back(20_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 35_000);
    commands.send(Command::Back(30_000)).unwrap();
    same(
        "back to A again",
        &paused_at(&shared, &mut console, c0 + 5_000),
        &a,
    );
    for k in 1..=10 {
        commands.send(Command::Step(5_000)).unwrap();
        let _ = paused_at(&shared, &mut console, c0 + 5_000 + k * 5_000);
    }
    same(
        "on to B by steps",
        &paused_at(&shared, &mut console, c0 + 55_000),
        &b,
    );
    // A cycle to go to, back and on.
    commands.send(Command::GoTo(c0 + 5_000)).unwrap();
    same(
        "to A's cycle",
        &paused_at(&shared, &mut console, c0 + 5_000),
        &a,
    );
    commands.send(Command::GoTo(c0 + 55_000)).unwrap();
    same(
        "to B's cycle",
        &paused_at(&shared, &mut console, c0 + 55_000),
        &b,
    );
    commands.send(Command::GoTo(c0 + 60_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 60_000);
    // Not before fast mode starts.
    commands.send(Command::Back(1_000_000)).unwrap();
    let earliest = shared.lock().unwrap().earliest;
    let _ = paused_at(&shared, &mut console, earliest);
}

/// The inputs given after a point one went back to are given again stepping
/// on, until a new one starts another run: on debounce a press of FIRE1
/// counts 1 on the LEDs.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn going_back_replays_the_inputs_until_a_new_one() {
    let (shared, commands) = start("debounce", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    let mut console = Vec::new();
    let c0 = shared.lock().unwrap().cycles;
    commands.send(Command::Step(1_000)).unwrap();
    let a = paused_at(&shared, &mut console, c0 + 1_000);
    assert_eq!(a.3, [Some(false); 8], "no press yet");
    commands.send(Command::Button(1, true)).unwrap();
    commands.send(Command::Step(100_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 101_000);
    commands.send(Command::Button(1, false)).unwrap();
    commands.send(Command::Step(100_000)).unwrap();
    let b = paused_at(&shared, &mut console, c0 + 201_000);
    let one = [true, false, false, false, false, false, false, false].map(Some);
    assert_eq!(b.3, one, "one press");
    commands.send(Command::Back(200_000)).unwrap();
    same(
        "back before the press",
        &paused_at(&shared, &mut console, c0 + 1_000),
        &a,
    );
    commands.send(Command::Step(200_000)).unwrap();
    same(
        "the press again",
        &paused_at(&shared, &mut console, c0 + 201_000),
        &b,
    );
    // Back into the press, where no snapshot is: the run is rebuilt from the
    // one before it with the press from the log. Pressing again there (the
    // level it has) starts a new run from the rebuilt state, which then runs
    // on as the first did only if the press was rebuilt.
    commands.send(Command::Back(150_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 51_000);
    commands.send(Command::Button(1, true)).unwrap();
    commands.send(Command::Step(50_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 101_000);
    assert_eq!(shared.lock().unwrap().ahead, 0, "a new run from the press");
    commands.send(Command::Button(1, false)).unwrap();
    commands.send(Command::Step(100_000)).unwrap();
    same(
        "rebuilt through the log",
        &paused_at(&shared, &mut console, c0 + 201_000),
        &b,
    );
    // A new input where the press was: the old run after it is gone, and
    // this press counts as the first.
    commands.send(Command::Back(200_000)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 1_000);
    commands.send(Command::Button(1, true)).unwrap();
    commands.send(Command::Step(1)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 1_001);
    assert_eq!(shared.lock().unwrap().ahead, 0, "the recorded run is gone");
    commands.send(Command::Step(99_999)).unwrap();
    let _ = paused_at(&shared, &mut console, c0 + 101_000);
    commands.send(Command::Button(1, false)).unwrap();
    commands.send(Command::Step(100_000)).unwrap();
    assert_eq!(
        paused_at(&shared, &mut console, c0 + 201_000).3,
        one,
        "the new press"
    );
}

/// Going back after a long free run takes a moment, however far: the
/// snapshots thin out with age and leave no gap. A pause shows at once.
/// (Once a gap made 50 million cycles back take 1.6 s, and the window,
/// waiting, took no command.)
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn going_back_far_is_quick() {
    let (shared, commands) = start("uart_hello", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    std::thread::sleep(Duration::from_secs(3));
    commands.send(Command::Pause).unwrap();
    wait(&shared, 1, "the pause", |s| !s.running);
    let (mut at, earliest) = {
        let s = shared.lock().unwrap();
        (s.cycles, s.earliest)
    };
    assert!(at > 60_000_000, "only {at} cycles in 3 s");
    for n in [1u64, 1_000, 1_000_000, 10_000_000, 40_000_000] {
        assert!(at - n > earliest);
        let t = Instant::now();
        commands.send(Command::Back(n)).unwrap();
        wait(&shared, 10, "the cycle gone back to", |s| {
            s.cycles + n == at
        });
        assert!(
            t.elapsed() < Duration::from_millis(500),
            "{n} cycles back took {:?}",
            t.elapsed()
        );
        at -= n;
    }
}

/// At full speed the window sees the values now and then, after an even
/// number of edges, so the clock is at the same level in every sample; the
/// wires that changed in the last cycle before each sample are given as well,
/// and the clock is among them, as is `led[0]`, which toggles every cycle.
/// (On Life, 10 000 LUTs, the clock did not light at real time.)
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn at_full_speed_the_clock_shows_as_changing() {
    use ecp5_emu::model::topology::{Location, Wire};
    let (shared, commands) = start("blinky_cycle", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    let s = shared.lock().unwrap().ready.clone().unwrap();
    let wire = |row, col, name: &str| Wire {
        location: Location { row, col },
        name: name.into(),
    };
    let clock = s
        .overlay
        .value_of(&s.model, &wire(47, 0, "JPADDIA_PIO"))
        .expect("the clock pad's wire is in the overlay");
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    let mut seen = 0;
    // The window takes the values only when the run has published since
    // (gui/src/app.rs `Taken`): anything said to have changed came with a
    // publish.
    let mut published = shared.lock().unwrap().published;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(20));
        let mut sh = shared.lock().unwrap();
        let toggled = std::mem::take(&mut sh.toggled);
        let since = sh.published != published;
        published = sh.published;
        if !toggled.is_empty() {
            assert!(since, "changes without a publish");
            assert_eq!(toggled.len(), sh.lit.len());
            assert_eq!(
                toggled[clock as usize], 1,
                "the clock did not change in the last cycle"
            );
            seen += 1;
        }
    }
    assert!(seen > 10, "only {seen} samples said what changed");
    commands.send(Command::Pause).unwrap();
    wait(&shared, 2, "the pause", |s| !s.running);
    assert!(
        shared.lock().unwrap().toggled.is_empty(),
        "paused, nothing is changing"
    );
}

/// At a set rate the window takes the values only at its frames, maybe
/// several edges apart: a wire that changed in between, an even number of
/// times (a clock), is still among those that changed.
#[test]
#[ignore]
fn at_a_set_rate_every_change_reaches_the_window() {
    use ecp5_emu::model::topology::{Location, Wire};
    let (shared, commands) = start("blinky_cycle", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    let s = shared.lock().unwrap().ready.clone().unwrap();
    let clock = s
        .overlay
        .value_of(
            &s.model,
            &Wire {
                location: Location { row: 47, col: 0 },
                name: "JPADDIA_PIO".into(),
            },
        )
        .expect("the clock pad's wire is in the overlay");
    commands.send(Command::Rate(Some(100.0))).unwrap();
    commands.send(Command::Run).unwrap();
    wait(&shared, 2, "the run", |s| s.running);
    let mut taken = 0;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        // Ten edges at 100 cycles a second.
        std::thread::sleep(Duration::from_millis(50));
        let toggled = std::mem::take(&mut shared.lock().unwrap().toggled);
        assert_eq!(
            toggled.get(clock as usize),
            Some(&1),
            "the clock changed ten times since the last frame, unseen"
        );
        taken += 1;
    }
    assert!(taken >= 30, "only {taken} frames");
    commands.send(Command::Pause).unwrap();
    wait(&shared, 2, "the pause", |s| !s.running);
    assert!(
        shared.lock().unwrap().toggled.is_empty(),
        "paused, nothing is changing"
    );
}

/// A step tells the window every wire that changed on its edges, not only
/// those whose value differs at its end: after a cycle the clock is back
/// where it was, and it changed twice.
#[test]
#[ignore]
fn a_step_reports_every_change_on_its_edges() {
    use ecp5_emu::model::topology::{Location, Wire};
    let (shared, commands) = start("blinky_cycle", false);
    wait(&shared, 120, "ready", |s| s.ready.is_some());
    let s = shared.lock().unwrap().ready.clone().unwrap();
    let clock = s
        .overlay
        .value_of(
            &s.model,
            &Wire {
                location: Location { row: 47, col: 0 },
                name: "JPADDIA_PIO".into(),
            },
        )
        .expect("the clock pad's wire is in the overlay") as usize;
    let before = shared.lock().unwrap().cycles;
    let was = shared.lock().unwrap().lit[clock];
    commands.send(Command::Step(1)).unwrap();
    wait(&shared, 10, "the step", |s| {
        !s.running && s.cycles == before + 1
    });
    let sh = shared.lock().unwrap();
    assert_eq!(
        sh.lit[clock], was,
        "after a cycle the clock is where it was"
    );
    assert_eq!(
        sh.toggled.get(clock),
        Some(&1),
        "the step did not say the clock changed"
    );
    drop(sh);
    commands.send(Command::Pause).unwrap();
    wait(&shared, 2, "the pause", |s| s.toggled.is_empty());
}

/// Selecting a port's value cannot change the physical overlay, including
/// the data wires at its input and output pads and their edge publications.
#[test]
#[ignore = "requires the native image reader and a local Trellis database"]
fn port_observations_preserve_the_complete_overlay_and_pad_data_wires() {
    use ecp5_emu::board;
    let (named, named_commands) = start_with_names("blinky_cycle", false, true);
    let (plain, plain_commands) = start_with_names("blinky_cycle", false, false);
    for shared in [&named, &plain] {
        wait(shared, 120, "ready", |s| {
            s.ready.is_some() && !s.lit.is_empty()
        });
    }
    let statics = named.lock().unwrap().ready.clone().unwrap();
    let unnamed = plain.lock().unwrap().ready.clone().unwrap();
    assert!(
        statics.overlay == unnamed.overlay,
        "loading HDL names changed the physical overlay"
    );
    assert!(unnamed.net_names.is_empty());
    let mut pad_values = Vec::new();
    for (ball, port) in
        std::iter::once((board::CLOCK, "O")).chain(board::LEDS.iter().map(|&ball| (ball, "I")))
    {
        let pio = statics
            .model
            .package_pin(board::PACKAGE, ball)
            .unwrap()
            .pio
            .unwrap();
        let wire = statics
            .model
            .resource_ports(pio)
            .into_iter()
            .find(|p| p.name == port)
            .unwrap()
            .wire;
        let value = statics
            .overlay
            .value_of(&statics.model, &wire)
            .unwrap_or_else(|| panic!("pad {ball}'s {port} data wire is absent: {wire:?}"));
        assert!(
            statics.overlay.pieces.iter().any(|p| p.value == value),
            "pad {ball}'s {port} data wire is not drawn"
        );
        pad_values.push(value as usize);
    }
    for cycle in 1..=4 {
        for (shared, commands) in [(&named, &named_commands), (&plain, &plain_commands)] {
            let target = shared.lock().unwrap().cycles + 1;
            commands.send(Command::Step(1)).unwrap();
            wait(shared, 10, "step", |s| !s.running && s.cycles == target);
        }
        let named = named.lock().unwrap();
        let plain = plain.lock().unwrap();
        assert_eq!(named.lit, plain.lit, "physical values at step {cycle}");
        assert_eq!(
            named.toggled, plain.toggled,
            "physical edges at step {cycle}"
        );
        for &value in &pad_values {
            assert!(named.lit[value] < 2, "unknown pad data at step {cycle}");
        }
        assert_eq!(named.toggled[pad_values[0]], 1, "the clock pad changed");
    }
}

/// Running, a large design's values change as it runs, at a set rate and
/// as fast as it goes: Life turns over thousands of wires each generation,
/// not only its clock. What the window shows is these values.
#[test]
#[ignore]
fn a_running_design_shows_its_values_changing() {
    let (shared, commands) = start("life", false);
    wait(&shared, 300, "ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    for rate in [Some(4.0), Some(100.0), None] {
        commands.send(Command::Rate(rate)).unwrap();
        commands.send(Command::Run).unwrap();
        wait(&shared, 5, "the run", |s| s.running);
        let first = shared.lock().unwrap().lit.clone();
        let mut last = first.clone();
        let mut changed = vec![false; first.len()];
        let (start, cycles) = (Instant::now(), shared.lock().unwrap().cycles);
        while start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(20));
            let now = shared.lock().unwrap().lit.clone();
            for (i, (a, b)) in last.iter().zip(&now).enumerate() {
                if a != b {
                    changed[i] = true;
                }
            }
            last = now;
        }
        let ran = shared.lock().unwrap().cycles - cycles;
        let n = changed.iter().filter(|&&c| c).count();
        println!(
            "{rate:?} cycles/s: {ran} cycles in 3 s, {n} of {} wires changed",
            first.len()
        );
        commands.send(Command::Pause).unwrap();
        wait(&shared, 5, "the pause", |s| !s.running);
        assert!(ran >= 8, "{rate:?}: only {ran} cycles");
        assert!(n > 1000, "{rate:?}: only {n} wires changed while running");
    }
}

/// The edges' rows of watched wires the window has been sent, by edge.
fn take_rows(shared: &Mutex<Shared>, id: u64, rows: &mut std::collections::BTreeMap<u64, Vec<u8>>) {
    for run in std::mem::take(&mut shared.lock().unwrap().waves) {
        assert_eq!(run.id, id, "rows of another watch list");
        for e in 0..run.edges() {
            let at = e as usize * run.width;
            rows.insert(run.first + e, run.rows[at..at + run.width].to_vec());
        }
    }
}

/// Watched wires: their values after every clock edge come to the window,
/// and are those the run shows when paused there (a wire the running
/// program does not keep up to date is marked not sampled, never wrong); at
/// a set rate every edge comes; going back sends the same rows again.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn watched_wires_give_their_value_after_every_edge() {
    use ecp5_emu::fast::NOT_SAMPLED;
    let (shared, commands) = start("blinky_cycle", false);
    wait(&shared, 120, "ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    let s = shared.lock().unwrap().ready.clone().unwrap();
    // A wire of each named net, up to 64.
    let mut nets: Vec<u32> = s.net_names.keys().copied().collect();
    nets.sort_unstable();
    let wires: Vec<u32> = nets
        .iter()
        .filter_map(|&net| s.overlay.nets.iter().position(|&n| n == net))
        .map(|v| v as u32)
        .take(64)
        .collect();
    assert!(wires.len() > 8, "only {} named nets", wires.len());
    commands
        .send(Command::Watch {
            id: 1,
            wires: wires.clone(),
            keep: 1 << 16,
        })
        .unwrap();
    let mut rows = std::collections::BTreeMap::new();
    // The last row against what the window shows paused there.
    let (mut sampled, mut not) = (0, 0);
    let mut check = |rows: &std::collections::BTreeMap<u64, Vec<u8>>, what: &str| {
        let sh = shared.lock().unwrap();
        let edge = sh.edges;
        assert_eq!(edge / 2, sh.cycles);
        let row = rows
            .get(&edge)
            .unwrap_or_else(|| panic!("{what}: no row for edge {edge}"));
        for (k, &w) in wires.iter().enumerate() {
            if row[k] == NOT_SAMPLED {
                not += 1;
            } else {
                sampled += 1;
                assert_eq!(
                    row[k], sh.lit[w as usize],
                    "{what}: wire {w} after edge {edge}"
                );
            }
        }
    };
    for _ in 0..40 {
        let before = shared.lock().unwrap().cycles;
        commands.send(Command::Step(1)).unwrap();
        wait(&shared, 10, "the step", |s| {
            !s.running && s.cycles == before + 1
        });
        take_rows(&shared, 1, &mut rows);
        check(&rows, "a step");
    }
    // At a set rate every edge comes.
    let from = shared.lock().unwrap().edges;
    commands.send(Command::Rate(Some(1000.0))).unwrap();
    commands.send(Command::Run).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    commands.send(Command::Pause).unwrap();
    wait(&shared, 5, "the pause", |s| !s.running);
    take_rows(&shared, 1, &mut rows);
    let to = shared.lock().unwrap().edges;
    assert!(to > from + 100, "only {} edges at 1000 c/s", to - from);
    let missing = (from + 1..=to).filter(|e| !rows.contains_key(e)).count();
    assert_eq!(missing, 0, "{missing} edges of {from}..{to} without a row");
    check(&rows, "1000 c/s");
    // As fast as it goes.
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    commands.send(Command::Pause).unwrap();
    wait(&shared, 5, "the pause", |s| !s.running);
    take_rows(&shared, 1, &mut rows);
    check(&rows, "as fast as it goes");
    // Going back 20 cycles: the rows sent again are those sent before.
    let sent = rows.clone();
    rows.clear();
    let now = shared.lock().unwrap().cycles;
    commands.send(Command::Back(20)).unwrap();
    wait(&shared, 30, "going back", |s| {
        !s.running && s.cycles == now - 20
    });
    take_rows(&shared, 1, &mut rows);
    assert!(!rows.is_empty(), "going back sent no rows");
    let mut again = 0;
    for (edge, row) in &rows {
        if let Some(before) = sent.get(edge) {
            for (k, (&a, &b)) in row.iter().zip(before).enumerate() {
                if a != NOT_SAMPLED && b != NOT_SAMPLED {
                    assert_eq!(a, b, "wire {} after edge {edge}, again", wires[k]);
                    again += 1;
                }
            }
        }
    }
    assert!(again > 0, "nothing sent again to compare");
    check(&rows, "gone back");
    println!("{sampled} values sampled and checked, {not} not sampled, {again} sent again alike");
    assert!(sampled > 0);
}

/// What watching costs the run: life_full as fast as it goes, interleaving
/// unwatched and watched phases over repeated rounds so drift cancels.
/// Part 1 pairs nothing watched (A) and fresh nets only (B) on the default
/// program; Part 2 pairs all 64 nets (C) and nothing watched (D) on the
/// rebuilt program. Emits JSON metric lines and a summary line; asserts
/// rows come for B and C only and mean sampling cost is within 10%.
/// Two openings separate learning the fresh nets from timing, since a
/// library cache hit can switch programs before the watch is narrowed.
#[test]
#[ignore = "requires clang, the reference build of life_full in target/reference, the native image reader and a local Trellis database; minutes"]
fn watching_64_nets_costs_little() {
    use ecp5_emu::fast::NOT_SAMPLED;
    const ROUNDS: usize = 6;
    let mean_and_sd = |xs: &[f64]| -> (f64, f64) {
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
        (mean, var.sqrt())
    };

    let (shared, commands) = start("life_full", false);
    wait(&shared, 900, "ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    let s = shared.lock().unwrap().ready.clone().unwrap();
    let mut nets: Vec<u32> = s.net_names.keys().copied().collect();
    nets.sort_unstable();
    let wires: Vec<u32> = nets
        .iter()
        .filter_map(|&net| s.overlay.pieces_of(net).first())
        .map(|&p| s.overlay.pieces[p as usize].value)
        .take(64)
        .collect();
    assert_eq!(wires.len(), 64);
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();

    let settings = shared.lock().unwrap().fast.clone().expect("fast settings");
    // Learn freshness from the default program, even if the build has
    // already finished by the time we read its rows.
    commands
        .send(Command::Watch {
            id: 1,
            wires: wires.clone(),
            keep: 1 << 16,
        })
        .unwrap();
    let started = Instant::now();
    let fresh_wires: Vec<u32> = loop {
        let s = shared.lock().unwrap();
        assert!(s.error.is_none(), "{:?}", s.error);
        let mut rows = s
            .waves
            .iter()
            .filter(|r| r.id == 1 && r.edges() > 0)
            .flat_map(|r| r.rows.chunks_exact(r.width));
        if let Some(row) = rows.find(|row| row.contains(&NOT_SAMPLED)) {
            break wires
                .iter()
                .zip(row)
                .filter(|&(_, &v)| v != NOT_SAMPLED)
                .map(|(&w, _)| w)
                .collect();
        }
        assert!(
            !s.waves.iter().any(|r| r.id == 1 && r.edges() > 0)
                || s.watch_status.is_none(),
            "no default-program row with NOT_SAMPLED among watch 1 rows; cannot learn fresh nets after a watch build starts: {:?}",
            s.watch_status
        );
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "no default-program row with stale nets after 60 s: {:?}",
            s.watch_status
        );
        drop(s);
        std::thread::sleep(Duration::from_millis(20));
    };
    let fresh = fresh_wires.len();
    let stale = wires.len() - fresh;
    assert!(fresh > 0, "no fresh wires among the 64 watched nets");
    assert!(stale > 0, "no stale wires among the 64 watched nets");
    drop(commands);
    wait(&shared, 600, "first runner to stop", |_| {
        Arc::strong_count(&shared) == 1
    });

    let (shared, commands) = start("life_full", false);
    wait(&shared, 900, "ready again", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    {
        let s = shared.lock().unwrap();
        let current = s.fast.as_ref().expect("second opening's fast settings");
        assert_eq!(
            current.folding, settings.folding,
            "fresh nets depend on folding: first opening {settings:?}, second opening {current:?}"
        );
        assert_eq!(s.watch_status, None, "second opening's default program");
    }
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();

    let watch_id = std::cell::Cell::new(3u64);
    let next_id = || {
        let id = watch_id.get();
        watch_id.set(id + 1);
        id
    };
    let measure = |label: &str, round: usize, watch_wires: &[u32], expect_rows: bool| -> f64 {
        let id = next_id();
        let check = |sh: &Shared| {
            assert!(sh.error.is_none(), "{:?}", sh.error);
            if matches!(label, "A" | "B") {
                assert_eq!(
                    sh.watch_status, None,
                    "watch build during {label} round {round} on the default program"
                );
            }
            for run in sh.waves.iter().filter(|r| r.id == id) {
                assert!(
                    !run.rows.contains(&NOT_SAMPLED),
                    "NOT_SAMPLED in {label} round {round} (watch id {id})"
                );
            }
        };
        commands
            .send(Command::Watch {
                id,
                wires: watch_wires.to_vec(),
                keep: 8192,
            })
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let mut edges = 0u64;
        let (c0, t0) = {
            let mut sh = shared.lock().unwrap();
            check(&sh);
            sh.waves.clear();
            (sh.cycles, Instant::now())
        };
        while t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(100));
            let mut sh = shared.lock().unwrap();
            check(&sh);
            edges += std::mem::take(&mut sh.waves)
                .iter()
                .filter(|r| r.id == id)
                .map(|r| r.edges())
                .sum::<u64>();
        }
        let (c1, elapsed) = (shared.lock().unwrap().cycles, t0.elapsed());
        let khz = (c1 - c0) as f64 / elapsed.as_secs_f64() / 1e3;
        if expect_rows {
            assert!(
                edges > 0,
                "no rows came for {label} round {round} (watch id {id})"
            );
        } else {
            assert_eq!(
                edges, 0,
                "rows came for {label} round {round} (watch id {id})"
            );
        }
        println!(r#"{{"metric":"khz_{label}","value":{khz:.3},"step":{round}}}"#);
        khz
    };

    // Part 1, the default program: 6 rounds of (A, B) with flipping order.
    let (mut a_khz, mut b_khz, mut ab_costs) = (Vec::new(), Vec::new(), Vec::new());
    for round in 1..=ROUNDS {
        let (a, b) = if round % 2 == 1 {
            (
                measure("A", round, &[], false),
                measure("B", round, &fresh_wires, true),
            )
        } else {
            let b = measure("B", round, &fresh_wires, true);
            let a = measure("A", round, &[], false);
            (a, b)
        };
        a_khz.push(a);
        b_khz.push(b);
        ab_costs.push((1.0 - b / a) * 100.0);
    }

    // Watch all 64 and wait for the rebuilt program to run.
    let rebuild_id = next_id();
    commands
        .send(Command::Watch {
            id: rebuild_id,
            wires: wires.clone(),
            keep: 8192,
        })
        .unwrap();
    wait(&shared, 600, "new program running", |s| {
        s.watch_status
            .as_deref()
            .is_some_and(|t| t.starts_with("Running a program"))
    });

    // Part 2, the rebuilt program: 6 rounds of (C, D) with flipping order.
    let (mut c_khz, mut d_khz, mut cd_costs) = (Vec::new(), Vec::new(), Vec::new());
    for round in 1..=ROUNDS {
        let (c, d) = if round % 2 == 1 {
            (
                measure("C", round, &wires, true),
                measure("D", round, &[], false),
            )
        } else {
            let d = measure("D", round, &[], false);
            let c = measure("C", round, &wires, true);
            (c, d)
        };
        c_khz.push(c);
        d_khz.push(d);
        cd_costs.push((1.0 - c / d) * 100.0);
    }
    commands.send(Command::Pause).unwrap();

    let (mean_a, _) = mean_and_sd(&a_khz);
    let (mean_b, _) = mean_and_sd(&b_khz);
    let (mean_c, _) = mean_and_sd(&c_khz);
    let (mean_d, _) = mean_and_sd(&d_khz);
    let (sampling_cost_pct, sampling_cost_pct_sd) = mean_and_sd(&ab_costs);
    let (rebuilt_cost_pct, rebuilt_cost_pct_sd) = mean_and_sd(&cd_costs);
    let program_cost_pct = (1.0 - mean_d / mean_a) * 100.0;

    println!(
        r#"{{"metric":"sampling_cost_pct","value":{sampling_cost_pct:.3},"higher_is_better":false}}"#
    );
    println!(
        r#"{{"metric":"sampling_cost_pct_sd","value":{sampling_cost_pct_sd:.3},"higher_is_better":false}}"#
    );
    println!(
        r#"{{"metric":"rebuilt_sampling_cost_pct","value":{rebuilt_cost_pct:.3},"higher_is_better":false}}"#
    );
    println!(
        r#"{{"metric":"rebuilt_sampling_cost_pct_sd","value":{rebuilt_cost_pct_sd:.3},"higher_is_better":false}}"#
    );
    println!(
        r#"{{"metric":"program_cost_pct","value":{program_cost_pct:.3},"higher_is_better":false}}"#
    );

    let env_val = |name: &str| std::env::var(name).unwrap_or_else(|_| "unset".into());
    let fast_env = [
        "FAST_CHOOSE",
        "FAST_TUNE",
        "FAST_THREADS",
        "FAST_COLLAPSE",
        "FAST_FOLD",
    ]
    .into_iter()
    .map(|k| format!("{k}={}", env_val(k)))
    .collect::<Vec<_>>()
    .join(" ");
    println!(
        "life_full: A {mean_a:.1} kHz, B {mean_b:.1} kHz (sampling cost {sampling_cost_pct:+.2}% +/- {sampling_cost_pct_sd:.2}%), C {mean_c:.1} kHz, D {mean_d:.1} kHz (rebuilt sampling cost {rebuilt_cost_pct:+.2}% +/- {rebuilt_cost_pct_sd:.2}%); program cost {program_cost_pct:+.2}% (positive: slower; D vs A not interleaved: removing nets never switches back); {fresh} fresh, {stale} stale; {fast_env}"
    );
    assert!(
        sampling_cost_pct < 10.0,
        "mean sampling cost exceeded 10%: {sampling_cost_pct:+.2}%"
    );
}

fn check_watching_stale_nets(design: &str) {
    use ecp5_emu::fast::NOT_SAMPLED;
    let (shared_a, cmd_a) = start(design, false);
    let (shared_b, cmd_b) = start(design, false);
    wait(&shared_a, 300, "A ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    wait(&shared_b, 300, "B ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    let mut console_a = Vec::new();
    let mut console_b = Vec::new();
    let c0 = shared_b.lock().unwrap().cycles;
    assert_eq!(shared_a.lock().unwrap().cycles, c0);
    let _ = paused_at(&shared_a, &mut console_a, c0);
    let _ = paused_at(&shared_b, &mut console_b, c0);

    // Find nets that the default program folds/evaluates on demand by watching
    // the first named nets and keeping the columns that come NOT_SAMPLED.
    let s = shared_b.lock().unwrap().ready.clone().unwrap();
    let mut nets: Vec<u32> = s.net_names.keys().copied().collect();
    nets.sort_unstable();
    let probe_wires: Vec<u32> = nets
        .iter()
        .filter_map(|&net| s.overlay.nets.iter().position(|&n| n == net))
        .map(|v| v as u32)
        .take(64)
        .collect();
    assert!(!probe_wires.is_empty());

    cmd_b
        .send(Command::Watch {
            id: 1,
            wires: probe_wires.clone(),
            keep: 1 << 16,
        })
        .unwrap();
    cmd_b.send(Command::Step(1)).unwrap();
    wait(&shared_b, 30, "B first step", |s| {
        !s.running && s.cycles == c0 + 1
    });
    let mut rows = std::collections::BTreeMap::new();
    take_rows(&shared_b, 1, &mut rows);
    let edge0 = shared_b.lock().unwrap().edges;
    let first_row = rows.get(&edge0).expect("row after first step");
    let stale_wires: Vec<u32> = probe_wires
        .iter()
        .zip(first_row)
        .filter(|(_, &v)| v == NOT_SAMPLED)
        .map(|(&w, _)| w)
        .collect();
    assert!(
        !stale_wires.is_empty(),
        "{design}: no stale wires among the first {} named nets",
        probe_wires.len()
    );

    // Narrow to the formerly stale wires, wait until B's status says the new
    // program runs, and bring A to the same cycle.
    cmd_b
        .send(Command::Watch {
            id: 2,
            wires: stale_wires.clone(),
            keep: 1 << 16,
        })
        .unwrap();
    wait(&shared_b, 300, "new program running", |s| {
        s.watch_status
            .as_deref()
            .is_some_and(|t| t.starts_with("Running a program"))
    });
    cmd_a.send(Command::Step(1)).unwrap();
    let a0 = paused_at(&shared_a, &mut console_a, c0 + 1);
    let b0 = paused_at(&shared_b, &mut console_b, c0 + 1);
    same(&format!("{design}: at switch"), &a0, &b0);
    let switch_edge = shared_b.lock().unwrap().edges;
    rows.clear();
    take_rows(&shared_b, 2, &mut rows);

    let mut checked_pauses = 1usize;
    let mut checked_samples = 0usize;
    let mut verify_pause = |what: &str, cycle: u64| {
        let a = paused_at(&shared_a, &mut console_a, cycle);
        let b = paused_at(&shared_b, &mut console_b, cycle);
        same(what, &a, &b);
        checked_pauses += 1;
        take_rows(&shared_b, 2, &mut rows);
        let edge = shared_b.lock().unwrap().edges;
        assert_eq!(shared_a.lock().unwrap().edges, edge, "{what}: edges differ");
        assert_eq!(edge, cycle * 2, "{what}: expected even edge");
        let row = rows
            .get(&edge)
            .unwrap_or_else(|| panic!("{what}: no row for edge {edge}"));
        for (k, &w) in stale_wires.iter().enumerate() {
            assert_ne!(
                row[k], NOT_SAMPLED,
                "{what}: wire {w} still NOT_SAMPLED after edge {edge}"
            );
            assert_eq!(
                row[k], b.1[w as usize],
                "{what}: wire {w} after edge {edge}"
            );
            checked_samples += 1;
        }
    };

    // Steps of one cycle.
    for i in 1..=10 {
        let c = c0 + 1 + i;
        cmd_a.send(Command::Step(1)).unwrap();
        cmd_b.send(Command::Step(1)).unwrap();
        verify_pause(&format!("{design}: step {i}"), c);
    }

    // 1000 c/s for a while, then pause and land both on one cycle with GoTo.
    cmd_a.send(Command::Rate(Some(1000.0))).unwrap();
    cmd_b.send(Command::Rate(Some(1000.0))).unwrap();
    cmd_a.send(Command::Run).unwrap();
    cmd_b.send(Command::Run).unwrap();
    std::thread::sleep(Duration::from_millis(350));
    let pause_even = |shared: &Mutex<Shared>, cmd: &Sender<Command>| loop {
        let before = shared.lock().unwrap().published;
        cmd.send(Command::Pause).unwrap();
        wait(shared, 10, "pause", |s| !s.running && s.published > before);
        if shared.lock().unwrap().edges.is_multiple_of(2) {
            break;
        }
        let before = shared.lock().unwrap().published;
        cmd.send(Command::Run).unwrap();
        wait(shared, 10, "run", |s| s.running && s.published > before);
        std::thread::sleep(Duration::from_millis(2));
    };
    pause_even(&shared_a, &cmd_a);
    pause_even(&shared_b, &cmd_b);
    let target = shared_a
        .lock()
        .unwrap()
        .cycles
        .max(shared_b.lock().unwrap().cycles)
        + 10;
    cmd_a.send(Command::GoTo(target)).unwrap();
    cmd_b.send(Command::GoTo(target)).unwrap();
    verify_pause(&format!("{design}: after 1000 c/s at {target}"), target);

    let post_switch: Vec<(u64, &Vec<u8>)> = rows
        .iter()
        .filter(|(&e, _)| e > switch_edge)
        .map(|(&e, r)| (e, r))
        .collect();
    assert!(!post_switch.is_empty());
    for (edge, row) in &post_switch {
        for (k, &w) in stale_wires.iter().enumerate() {
            assert_ne!(
                row[k], NOT_SAMPLED,
                "{design}: wire {w} came NOT_SAMPLED after switch at edge {edge}"
            );
        }
    }
    println!(
        "{design}: {} stale of {} nets became fresh; {} post-switch edges, {} pauses and {} sampled values matched unwatched run ({:?})",
        stale_wires.len(),
        probe_wires.len(),
        post_switch.len(),
        checked_pauses,
        checked_samples,
        shared_b.lock().unwrap().watch_status.as_deref().unwrap_or("")
    );
}

/// Watching stale nets builds a program that keeps them in the background
/// and switches to it without losing state: from the switch on, the run
/// matches an unwatched run identically and the formerly stale nets are
/// sampled after every edge.
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn watching_stale_nets_rebuilds_and_matches_unwatched() {
    check_watching_stale_nets("blinky_cycle");
    check_watching_stale_nets("life");
}

/// Watching nets while a peripheral answers every clock edge (`sdram_test`)
/// sends every edge's row to `Shared::waves` (marking stale columns
/// `NOT_SAMPLED`), matches an unwatched run's console text, and keeps sending
/// rows when seeking back (`Command::GoTo`).
#[test]
#[ignore = "requires clang, the reference build in target/reference, the native image reader and a local Trellis database"]
fn watched_wires_while_sdram_answers_give_rows_and_console() {
    use ecp5_emu::fast::NOT_SAMPLED;
    let (shared_a, cmd_a) = start("sdram_test", false);
    let (shared_b, cmd_b) = start("sdram_test", false);
    wait(&shared_a, 300, "A ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });
    wait(&shared_b, 300, "B ready", |s| {
        s.ready.is_some() && !s.lit.is_empty()
    });

    let s = shared_b.lock().unwrap().ready.clone().unwrap();
    let mut nets: Vec<u32> = s.net_names.keys().copied().collect();
    nets.sort_unstable();
    let wires: Vec<u32> = nets
        .iter()
        .filter_map(|&net| s.overlay.nets.iter().position(|&n| n == net))
        .map(|v| v as u32)
        .take(16)
        .collect();
    assert!(wires.len() > 4, "only {} named nets", wires.len());

    let mut console_a = Vec::new();
    let mut console_b = Vec::new();
    let mut rows = std::collections::BTreeMap::new();
    let drain_console = |shared: &Mutex<Shared>, console: &mut Vec<u8>| {
        let mut s = shared.lock().unwrap();
        assert!(s.error.is_none(), "{}", s.error.as_ref().unwrap());
        if let Some(all) = s.console.take() {
            *console = all;
        }
        console.extend(std::mem::take(&mut s.text));
    };

    let from_edge = shared_b.lock().unwrap().edges;
    cmd_b
        .send(Command::Watch {
            id: 1,
            wires: wires.clone(),
            // This test collects the whole transaction and its replay, rather
            // than a viewport. Indexing a batch can outlast the small buffer.
            keep: 1 << 22,
        })
        .unwrap();
    cmd_a.send(Command::Rate(None)).unwrap();
    cmd_b.send(Command::Rate(None)).unwrap();
    cmd_a.send(Command::Run).unwrap();
    cmd_b.send(Command::Run).unwrap();

    let started = Instant::now();
    let line = "SDRAM W8000 E0000\r\n";
    loop {
        drain_console(&shared_a, &mut console_a);
        drain_console(&shared_b, &mut console_b);
        take_rows(&shared_b, 1, &mut rows);
        let a_done = String::from_utf8_lossy(&console_a).contains(line);
        let b_done = String::from_utf8_lossy(&console_b).contains(line);
        if a_done && b_done {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(180),
            "timed out waiting for SDRAM line: A={:?}, B={:?}",
            String::from_utf8_lossy(&console_a),
            String::from_utf8_lossy(&console_b)
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    cmd_a.send(Command::Pause).unwrap();
    cmd_b.send(Command::Pause).unwrap();
    wait(&shared_a, 60, "A pause", |s| !s.running);
    wait(&shared_b, 60, "B pause", |s| !s.running);
    drain_console(&shared_a, &mut console_a);
    drain_console(&shared_b, &mut console_b);
    take_rows(&shared_b, 1, &mut rows);

    let target = shared_a
        .lock()
        .unwrap()
        .cycles
        .min(shared_b.lock().unwrap().cycles);
    cmd_a.send(Command::GoTo(target)).unwrap();
    cmd_b.send(Command::GoTo(target)).unwrap();
    let a = paused_at(&shared_a, &mut console_a, target);
    let b = paused_at(&shared_b, &mut console_b, target);
    take_rows(&shared_b, 1, &mut rows);

    assert_eq!(
        String::from_utf8_lossy(&a.4),
        String::from_utf8_lossy(&b.4),
        "console text differs"
    );
    assert!(
        String::from_utf8_lossy(&b.4).contains(line),
        "expected {line:?} in {:?}",
        String::from_utf8_lossy(&b.4)
    );
    assert!(
        shared_b
            .lock()
            .unwrap()
            .peripherals
            .sdram
            .as_ref()
            .is_some_and(|sdram| sdram.initialised),
        "SDRAM peripheral did not run"
    );

    let to_edge = target * 2;
    let missing = (from_edge + 1..=to_edge)
        .filter(|e| !rows.contains_key(e))
        .count();
    assert_eq!(
        missing, 0,
        "{missing} edges of {from_edge}..={to_edge} without a row"
    );
    let sampled_values = (from_edge + 1..=to_edge)
        .flat_map(|e| rows[&e].iter())
        .filter(|&&v| v != NOT_SAMPLED)
        .count();
    assert!(sampled_values > 0, "every watched entry was NOT_SAMPLED");

    let check_pause_row =
        |edge: u64, lit: &[u8], rows: &std::collections::BTreeMap<u64, Vec<u8>>| {
            let row = rows
                .get(&edge)
                .unwrap_or_else(|| panic!("no row for edge {edge}"));
            let mut at_pause = 0usize;
            for (k, &w) in wires.iter().enumerate() {
                if row[k] != NOT_SAMPLED {
                    assert_eq!(row[k], lit[w as usize], "wire {w} after edge {edge}");
                    at_pause += 1;
                }
            }
            assert!(
                at_pause > 0,
                "all watched wires were NOT_SAMPLED at edge {edge}"
            );
            at_pause
        };
    let at_pause = check_pause_row(to_edge, &b.1, &rows);

    // Seeking back while the peripheral answers also delivers rows and keeps console identical.
    let seek_target = target.saturating_sub(200);
    rows.clear();
    cmd_a.send(Command::GoTo(seek_target)).unwrap();
    cmd_b.send(Command::GoTo(seek_target)).unwrap();
    let a_back = paused_at(&shared_a, &mut console_a, seek_target);
    let b_back = paused_at(&shared_b, &mut console_b, seek_target);
    take_rows(&shared_b, 1, &mut rows);
    assert_eq!(
        String::from_utf8_lossy(&a_back.4),
        String::from_utf8_lossy(&b_back.4),
        "console text differs after GoTo back"
    );
    let _ = check_pause_row(seek_target * 2, &b_back.1, &rows);

    println!(
        "sdram_test live watch: {} wires, {} edges checked ({sampled_values} sampled values, {at_pause} matched lit at cycle {target})",
        wires.len(),
        to_edge - from_edge
    );
}
