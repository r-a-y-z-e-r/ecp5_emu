//! The ULX3S DIP inputs, their pad pulls and their recorded positions.
mod common;
use ecp5_emu::{
    board,
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

fn build(pullup: bool) -> PathBuf {
    common::reference::build(if pullup {
        "examples/ulx3s/switches/pullup"
    } else {
        "examples/ulx3s/switches"
    })
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn pure_input_off_levels_use_the_configured_pull_in_fast_mode() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let pin = |b: &str| Reference::Pin(b.into());
    for pullup in [false, true] {
        let out = build(pullup);
        let image = Arc::new(ConfigurationImage::load(&out.join("top.config"), &db, None).unwrap());
        let device = Arc::new(
            ConfiguredDevice::bind(
                Arc::clone(&model),
                image,
                load_catalogue(&model, &evidence).unwrap(),
            )
            .unwrap(),
        );
        for ball in board::SWITCHES {
            assert_eq!(
                board::direction(&device, ball).unwrap(),
                Some(board::Direction::Input)
            );
        }
        for (ball, direction) in board::used_balls(&device).unwrap() {
            assert!(
                !direction.drives()
                    || board::LEDS.contains(&ball.as_str())
                    || ball == board::UART_TX,
                "unexpected output {ball}"
            );
        }
        let refs: Vec<_> = board::SWITCHES
            .into_iter()
            .map(pin)
            .chain(board::LEDS[..4].iter().map(|b| pin(b)))
            .collect();
        let mut compiled = compile::compile(
            device,
            Request {
                package: board::PACKAGE.into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives: std::iter::once((pin(board::CLOCK), Logic::Zero))
                    .chain(board::SWITCHES.map(|b| (pin(b), Logic::One)))
                    .collect(),
                probes: refs.clone(),
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )
        .unwrap();
        let mut fast = FastSim::new(&compiled.kernel).unwrap();
        let drivers: Vec<_> = board::SWITCHES
            .map(|b| compiled.drivers[&pin(b).label()])
            .into_iter()
            .collect();
        let levels: Vec<_> = drivers.iter().map(|&d| (d, None)).collect();
        let at = Time::from_ps(1_000);
        let kernel_levels: Vec<_> = drivers.iter().map(|&d| (d, Logic::Z)).collect();
        compiled
            .kernel
            .schedule_batch(at, &kernel_levels, None)
            .unwrap();
        compiled.kernel.run_until(at).unwrap();
        for r in &refs {
            assert_eq!(
                compiled.kernel.value(compiled.probes[&r.label()]).unwrap(),
                Logic::from_bool(pullup)
            );
        }
        let released = match fast.set_at_levels(at, &levels) {
            Ok(()) => {
                for r in &refs {
                    assert_eq!(
                        fast.value(compiled.probes[&r.label()]),
                        Some(Logic::from_bool(pullup))
                    );
                }
                "supported"
            }
            Err(error) => {
                assert!(error.to_string().contains("cannot be set to Z"));
                "rejected"
            }
        };
        assert_eq!(
            board::input_pull(compiled.kernel.device(), board::CLOCK).unwrap(),
            None
        );
        let off = board::input_pull(compiled.kernel.device(), board::SWITCHES[0])
            .unwrap()
            .unwrap_or(false);
        assert_eq!(off, pullup);
        fast.set_at_levels(
            Time::from_ps(2_000),
            &drivers.iter().map(|&d| (d, Some(off))).collect::<Vec<_>>(),
        )
        .unwrap();
        for r in &refs {
            assert_eq!(
                fast.value(compiled.probes[&r.label()]),
                Some(Logic::from_bool(pullup))
            );
        }
        fast.set_at_levels(
            Time::from_ps(3_000),
            &drivers.iter().map(|&d| (d, Some(true))).collect::<Vec<_>>(),
        )
        .unwrap();
        for r in &refs {
            assert_eq!(fast.value(compiled.probes[&r.label()]), Some(Logic::One));
        }
        println!(
            "PULLMODE={}: pure-input Z {released}; model OFF reads {}, ON reads 1",
            if pullup { "UP" } else { "DOWN" },
            u8::from(pullup)
        );
    }
}

use ecp5_emu::live::{self, Command, Design, RunOptions, Shared};
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

fn start(pullup: bool, switches: [bool; 4]) -> (Arc<Mutex<Shared>>, Sender<Command>) {
    let out = build(pullup);
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn_with(
        Design {
            config: out.join("top.config"),
            names: Some(out.join("top_routed.json")),
            run: false,
        },
        RunOptions {
            switches: Some(switches),
        },
        Arc::clone(&shared),
        || {},
    );
    (shared, commands)
}
fn wait(shared: &Mutex<Shared>, what: &str, done: impl Fn(&Shared) -> bool) {
    let start = Instant::now();
    loop {
        {
            let s = shared.lock().unwrap();
            assert!(s.error.is_none(), "{:?}", s.error);
            if done(&s) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(180),
                "no {what}: {} at {}, {:?}",
                s.stage,
                s.cycles,
                s.switches
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn step(shared: &Mutex<Shared>, commands: &Sender<Command>, cycles: u64) {
    let target = shared.lock().unwrap().cycles + cycles;
    commands.send(Command::Step(cycles)).unwrap();
    wait(shared, "step", |s| {
        !s.running && s.stage == "Paused" && s.cycles == target
    });
}
fn go_to(shared: &Mutex<Shared>, commands: &Sender<Command>, cycle: u64) {
    commands.send(Command::GoTo(cycle)).unwrap();
    wait(shared, "seek", |s| {
        !s.running && s.stage == "Paused" && s.cycles == cycle
    });
}
fn console(s: &Shared) -> Vec<u8> {
    // Combine restored and new bytes before finding the last complete report.
    // A periodic report may still be in flight after an arbitrary Pause/Step.
    let mut text: Vec<_> = s.console.iter().flatten().chain(&s.text).copied().collect();
    let end = text.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    text.truncate(end);
    text
}

#[test]
fn console_ignores_an_unterminated_switch_report() {
    let mut shared = Shared {
        console: Some(b"SW 1010\r\nSW 11".to_vec()),
        text: b"10\r\nSW 10".to_vec(),
        ..Shared::default()
    };
    assert_eq!(console(&shared), b"SW 1010\r\nSW 1110\r\n");
    shared.text = b"10\r".to_vec();
    assert_eq!(console(&shared), b"SW 1010\r\n");
    shared.text.push(b'\n');
    assert_eq!(console(&shared), b"SW 1010\r\nSW 1110\r\n");
    shared.console = None;
    shared.text = b"SW 1010".to_vec();
    assert!(console(&shared).is_empty());
}
fn leds(s: &Shared, expected: [bool; 4]) {
    assert_eq!(&s.leds[..4], &expected.map(Some));
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn dip_positions_debounce_and_replay_after_going_back() {
    let (shared, commands) = start(false, [true, false, true, false]);
    wait(&shared, "ready", |s| s.ready.is_some());
    assert_eq!(
        shared.lock().unwrap().ready.as_ref().unwrap().switches,
        [true; 4]
    );
    step(&shared, &commands, 400_000);
    {
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [true, false, true, false]);
        leds(&s, s.switches);
        assert_eq!(console(&s), b"SW 1010\r\n");
    }
    commands.send(Command::Switch(1, true)).unwrap();
    step(&shared, &commands, 400_000);
    let text = {
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [true, true, true, false]);
        leds(&s, s.switches);
        assert_eq!(console(&s), b"SW 1010\r\nSW 1110\r\n");
        console(&s)
    };
    go_to(&shared, &commands, 400_000);
    assert_eq!(shared.lock().unwrap().switches, [true, false, true, false]);
    go_to(&shared, &commands, 300_000);
    {
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [true, false, true, false]);
        leds(&s, s.switches);
    }
    step(&shared, &commands, 500_000);
    {
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [true, true, true, false]);
        leds(&s, s.switches);
        assert_eq!(console(&s), text);
    }
    // A checkpoint after a flip restores its positions, too; the next
    // one-second UART report does not require another input change.
    step(&shared, &commands, 24_700_000);
    assert_eq!(
        console(&shared.lock().unwrap()),
        b"SW 1010\r\nSW 1110\r\nSW 1110\r\n"
    );
    go_to(&shared, &commands, 450_000);
    assert_eq!(shared.lock().unwrap().switches, [true, true, true, false]);
    go_to(&shared, &commands, 300_000);
    commands.send(Command::Rate(None)).unwrap();
    commands.send(Command::Run).unwrap();
    wait(&shared, "running replay", |s| {
        s.cycles >= 800_000 && console(s).starts_with(&text)
    });
    commands.send(Command::Pause).unwrap();
    wait(&shared, "pause", |s| !s.running && s.stage == "Paused");
    commands.send(Command::Switch(1, false)).unwrap();
    step(&shared, &commands, 400_000);
    let s = shared.lock().unwrap();
    assert_eq!(s.switches, [true, false, true, false]);
    leds(&s, s.switches);
    assert!(console(&s).ends_with(b"SW 1010\r\n"));
    println!("DOWN: SW 1010 -> SW 1110; rewind restores positions/LEDs and both Step/Run replay the flip; OFF returns SW 1010");
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn pullup_reads_all_ones_with_all_switches_off() {
    let (shared, commands) = start(true, [false; 4]);
    wait(&shared, "ready", |s| s.ready.is_some());
    step(&shared, &commands, 400_000);
    {
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [false; 4]);
        leds(&s, [true; 4]);
        assert_eq!(console(&s), b"SW 1111\r\n");
    }
    commands.send(Command::Switch(0, true)).unwrap();
    step(&shared, &commands, 1);
    assert_eq!(shared.lock().unwrap().switches, [true, false, false, false]);
    commands.send(Command::Switch(0, false)).unwrap();
    step(&shared, &commands, 1);
    let s = shared.lock().unwrap();
    assert_eq!(s.switches, [false; 4]);
    leds(&s, [true; 4]);
    println!(
        "UP: all OFF prints SW 1111; physical ON/OFF positions remain distinct while both read 1"
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn the_board_all_on_read_reproduces_for_both_pulls() {
    for pullup in [false, true] {
        let (shared, commands) = start(pullup, [true; 4]);
        wait(&shared, "ready", |s| s.ready.is_some());
        step(&shared, &commands, 400_000);
        let s = shared.lock().unwrap();
        assert_eq!(s.switches, [true; 4]);
        leds(&s, [true; 4]);
        assert_eq!(console(&s), b"SW 1111\r\n");
        println!("HR10 {}: SW 1111", if pullup { "UP" } else { "DOWN" });
    }
}

#[test]
fn pullup_variant_copies_the_logic_and_only_changes_switch_pulls() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/ulx3s/switches");
    assert_eq!(
        std::fs::read(root.join("top.v")).unwrap(),
        std::fs::read(root.join("pullup/top.v")).unwrap()
    );
    let down = std::fs::read_to_string(root.join("top.lpf")).unwrap();
    assert_eq!(down.matches("PULLMODE=DOWN").count(), 4);
    let up = down.replace("PULLMODE=DOWN", "PULLMODE=UP");
    assert_eq!(
        std::fs::read_to_string(root.join("top_pullup.lpf")).unwrap(),
        up
    );
    assert_eq!(
        std::fs::read_to_string(root.join("pullup/top.lpf")).unwrap(),
        up
    );
}
