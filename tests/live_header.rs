//! Recorded external header positions, pad levels, shared ADC sources and replay.
mod common;
use ecp5_emu::{
    board,
    live::{self, Command, Design, RunOptions, Shared},
};
use std::{
    sync::{mpsc::Sender, Arc, Mutex},
    time::{Duration, Instant},
};
fn start(design: &str, header: board::HeaderLevels) -> (Arc<Mutex<Shared>>, Sender<Command>) {
    let out = common::reference::build(design);
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn_with(
        Design {
            config: out.join("top.config"),
            names: Some(out.join("top_routed.json")),
            run: false,
        },
        RunOptions::default().header(header),
        Arc::clone(&shared),
        || {},
    );
    wait(&shared, |s| s.ready.is_some());
    (shared, commands)
}
fn wait(shared: &Mutex<Shared>, done: impl Fn(&Shared) -> bool) {
    let at = Instant::now();
    loop {
        let s = shared.lock().unwrap();
        assert!(s.error.is_none(), "{:?}", s.error);
        if done(&s) {
            break;
        }
        assert!(
            at.elapsed() < Duration::from_secs(240),
            "{} {}",
            s.stage,
            s.cycles
        );
        drop(s);
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn step(s: &Mutex<Shared>, c: &Sender<Command>, cycles: u64) {
    let target = s.lock().unwrap().cycles + cycles;
    c.send(Command::Step(cycles)).unwrap();
    wait(s, |s| !s.running && s.cycles == target);
}
fn text(s: &Shared) -> Vec<u8> {
    s.console.iter().flatten().chain(&s.text).copied().collect()
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn external_header_levels_rewind_and_replay() {
    let mut levels = [None; 56];
    levels[0] = Some(false);
    let (s, c) = start("examples/ulx3s/header_probe/driven", levels);
    step(&s, &c, 26_000_000);
    {
        let s = s.lock().unwrap();
        assert_eq!(s.header_levels[0], Some(false));
        assert_eq!(s.leds[0], Some(false));
        assert!(text(&s).ends_with(b" E0\r\n"));
        assert_eq!(s.ready.as_ref().unwrap().header.len(), 56);
        for index in [1, 3, 5, 7] {
            assert!(s
                .ready
                .as_ref()
                .unwrap()
                .header
                .contains(&(index, board::Direction::Bidirectional)));
        }
        assert!(String::from_utf8_lossy(&text(&s)).contains(" P 000003E8/005F5E10"));
    }
    c.send(Command::HeaderLevel(0, Some(true))).unwrap();
    step(&s, &c, 400_000);
    let after = {
        let s = s.lock().unwrap();
        assert_eq!(s.header_levels[0], Some(true));
        assert_eq!(s.leds[0], Some(true));
        assert!(text(&s).ends_with(b" E1\r\n"));
        text(&s)
    };
    c.send(Command::GoTo(26_000_000)).unwrap();
    wait(&s, |s| !s.running && s.cycles == 26_000_000);
    assert_eq!(s.lock().unwrap().header_levels[0], Some(false));
    assert_eq!(s.lock().unwrap().leds[0], Some(false));
    step(&s, &c, 400_000);
    assert_eq!(text(&s.lock().unwrap()), after);
    c.send(Command::HeaderLevel(0, None)).unwrap();
    step(&s, &c, 1);
    {
        let s = s.lock().unwrap();
        assert_eq!(s.header_levels[0], None);
        assert_eq!(s.leds[0], Some(true));
    }
}
#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn shared_ain_header_wins_then_restores_adc_source() {
    let (s, c) = start("adc_read", [None; 56]);
    c.send(Command::AdcVolts(0, 1.25)).unwrap();
    step(&s, &c, 2);
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        1.25
    );
    // GN14 is AIN0, HEADER index 28.
    c.send(Command::HeaderLevel(28, Some(true))).unwrap();
    step(&s, &c, 2);
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        3.3
    );
    c.send(Command::AdcVolts(0, 0.75)).unwrap();
    step(&s, &c, 2);
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        3.3
    );
    c.send(Command::HeaderLevel(28, None)).unwrap();
    step(&s, &c, 2);
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        0.75
    );
    c.send(Command::GoTo(4)).unwrap();
    wait(&s, |s| !s.running && s.cycles == 4);
    assert_eq!(s.lock().unwrap().header_levels[28], Some(true));
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        3.3
    );
    step(&s, &c, 4);
    assert_eq!(s.lock().unwrap().header_levels[28], None);
    assert_eq!(
        s.lock().unwrap().peripherals.adc.as_ref().unwrap().volts[0],
        0.75
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn bidirectional_release_uses_the_real_tristate_pad_path() {
    use std::{fs, path::PathBuf};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = root.join("examples/ulx3s/header_probe/driven");
    let prepared = root
        .join("target")
        .join(format!("header-bidirectional-{}", std::process::id()));
    fs::create_dir_all(&prepared).unwrap();
    for file in ["header_logic.v", "uart_tx.v"] {
        fs::copy(source.join(file), prepared.join(file)).unwrap();
    }
    let top = fs::read_to_string(source.join("top.v"))
        .unwrap()
        .replace("input [27:0] gn", "input [27:1] gn, inout gn0")
        .replace(".gn(gn)", ".gn({gn,gn0})")
        .replace(
            "  wire [3:0] pwm;",
            "  wire [3:0] pwm;\n  assign gn0 = gn[1] ? 1'bz : 1'b0;",
        );
    fs::write(prepared.join("top.v"), top).unwrap();
    fs::write(
        prepared.join("top.lpf"),
        fs::read_to_string(source.join("top.lpf"))
            .unwrap()
            .replace("\"gn[0]\"", "\"gn0\""),
    )
    .unwrap();
    let (s, c) = start(prepared.to_str().unwrap(), [None; 56]);
    let level = |s: &Shared| {
        s.peripherals
            .header
            .as_ref()
            .unwrap()
            .pins
            .iter()
            .find(|p| p.index == 0)
            .unwrap()
            .level
    };
    step(&s, &c, 2);
    assert_eq!(level(&s.lock().unwrap()), 1);
    assert!(s
        .lock()
        .unwrap()
        .ready
        .as_ref()
        .unwrap()
        .header
        .contains(&(0, board::Direction::Bidirectional)));
    c.send(Command::HeaderLevel(0, Some(false))).unwrap();
    step(&s, &c, 2);
    assert_eq!(level(&s.lock().unwrap()), 0);
    c.send(Command::HeaderLevel(0, None)).unwrap();
    step(&s, &c, 2);
    assert_eq!(level(&s.lock().unwrap()), 1);
    assert_eq!(s.lock().unwrap().header_levels[0], None);
}
