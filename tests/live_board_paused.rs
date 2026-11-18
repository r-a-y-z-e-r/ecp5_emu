//! The board state and publication observed after paused navigation.
mod common;

use ecp5_emu::live::{self, Command, Design, Shared};
use std::{
    sync::{mpsc::Sender, Arc, Mutex},
    time::{Duration, Instant},
};

fn wait(shared: &Mutex<Shared>, done: impl Fn(&Shared) -> bool) {
    let started = Instant::now();
    loop {
        {
            let state = shared.lock().unwrap();
            assert!(state.error.is_none(), "{:?}", state.error);
            if done(&state) {
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(120),
                "stage {}, cycle {}, publication {}",
                state.stage,
                state.cycles,
                state.published
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn navigate(shared: &Mutex<Shared>, commands: &Sender<Command>, command: Command, cycle: u64) {
    let before = shared.lock().unwrap().published;
    commands.send(command).unwrap();
    wait(shared, |state| {
        !state.running && state.cycles == cycle && state.published > before
    });
    let state = shared.lock().unwrap();
    assert_eq!(state.edges, cycle * 2);
    assert_eq!(state.stage, "Paused");
    assert_eq!(
        state.leds,
        std::array::from_fn(|i| Some(cycle & (1 << i) != 0)),
        "published LEDs at cycle {cycle} (publication {})",
        state.published
    );
    assert!(!state.values.is_empty() && !state.lit.is_empty());
    assert!(state.text.is_empty() && state.console.as_ref().is_none_or(Vec::is_empty));
    assert!(state.header_levels.iter().all(Option::is_none));
    assert!(state.peripherals.header.is_none());
}

#[test]
#[ignore = "headless board regression requires the reference builder, Trellis and clang"]
fn paused_step_back_and_goto_publish_blinky_leds() {
    let root = common::reference::design("blinky_cycle");
    let shared = Arc::new(Mutex::new(Shared::default()));
    let commands = live::spawn(
        Design {
            config: root.join("top.config"),
            names: Some(root.join("top_routed.json")),
            run: false,
        },
        Arc::clone(&shared),
        || {},
    );
    wait(&shared, |state| {
        state.ready.is_some() && state.published > 0
    });
    assert_eq!(shared.lock().unwrap().cycles, 0);
    for cycle in 1..=20 {
        navigate(&shared, &commands, Command::Step(1), cycle);
    }
    for (command, target) in [
        (Command::Back(7), 13),
        (Command::GoTo(4), 4),
        (Command::GoTo(18), 18),
        (Command::Step(1), 19),
        (Command::GoTo(40), 40),
        (Command::Back(39), 1),
        (Command::GoTo(40), 40),
        (Command::GoTo(40), 40),
        (Command::Step(0), 40),
        (Command::GoTo(254), 254),
        (Command::Step(1), 255),
        (Command::Step(1), 256),
        (Command::Step(1), 257),
        (Command::Back(1), 256),
        (Command::GoTo(255), 255),
    ] {
        navigate(&shared, &commands, command, target);
    }
}
