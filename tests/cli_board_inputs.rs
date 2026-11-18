//! Board idle inputs in both CLI executors, including an unpulled button.
mod common;
use ecp5_emu::board;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        OnceLock,
    },
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn prepared(name: &str, verilog: &str, lpf: &str) -> PathBuf {
    let source = root()
        .join("target/cli_board_inputs")
        .join(std::process::id().to_string())
        .join(name);
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("top.v"), verilog).unwrap();
    std::fs::write(source.join("top.lpf"), lpf).unwrap();
    common::reference::build(source)
}
fn debounce_none() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let source = root().join("examples/ulx3s/debounce");
        let lpf = std::fs::read_to_string(source.join("top.lpf")).unwrap();
        let original = "IOBUF PORT \"fire1\" PULLMODE=DOWN";
        assert!(lpf.contains(original));
        prepared(
            "debounce-none",
            &std::fs::read_to_string(source.join("top.v")).unwrap(),
            &lpf.replace(original, "IOBUF PORT \"fire1\" PULLMODE=NONE"),
        )
    })
}
fn controls() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let verilog=r#"module top(input clk_25mhz, input [6:0] btn, input [3:0] sw,
  input ftdi_txd, output ftdi_rxd, output [7:0] led);
  reg seen = 0;
  always @(posedge clk_25mhz) seen <= btn[2];
  assign led = {seen ^ (^btn[6:3]), sw, ftdi_txd, btn[1:0]};
  assign ftdi_rxd = 1'b1;
endmodule
"#;
        let mut lpf=String::from("LOCATE COMP \"clk_25mhz\" SITE \"G2\";\nIOBUF PORT \"clk_25mhz\" PULLMODE=NONE IO_TYPE=LVCMOS33;\nFREQUENCY PORT \"clk_25mhz\" 25 MHZ;\nLOCATE COMP \"ftdi_txd\" SITE \"M1\";\nIOBUF PORT \"ftdi_txd\" PULLMODE=NONE IO_TYPE=LVCMOS33;\nLOCATE COMP \"ftdi_rxd\" SITE \"L4\";\nIOBUF PORT \"ftdi_rxd\" PULLMODE=NONE IO_TYPE=LVCMOS33;\n");
        for (i,b) in board::BUTTONS.iter().enumerate() {
            lpf.push_str(&format!("LOCATE COMP \"btn[{i}]\" SITE \"{}\";\nIOBUF PORT \"btn[{i}]\" PULLMODE=NONE IO_TYPE=LVCMOS33;\n",b.ball));
        }
        for (i,b) in board::SWITCHES.iter().enumerate() {
            let pull=if i==1 {"UP"} else if i==2 {"NONE"} else {"DOWN"};
            lpf.push_str(&format!("LOCATE COMP \"sw[{i}]\" SITE \"{b}\";\nIOBUF PORT \"sw[{i}]\" PULLMODE={pull} IO_TYPE=LVCMOS33;\n"));
        }
        for (i,b) in board::LEDS.iter().enumerate() {
            lpf.push_str(&format!("LOCATE COMP \"led[{i}]\" SITE \"{b}\";\nIOBUF PORT \"led[{i}]\" PULLMODE=NONE IO_TYPE=LVCMOS33;\n"));
        }
        prepared("board-controls",verilog,&lpf)
    })
}
fn output_path(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = root()
        .join("target/cli_board_inputs")
        .join(std::process::id().to_string())
        .join("runs");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{}-{name}", NEXT.fetch_add(1, Ordering::Relaxed)))
}
fn command(
    build: &Path,
    fast: bool,
    peripherals: bool,
    cycles: u64,
    probes: &[&str],
    vcd: &Path,
) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
    c.arg("emulate")
        .arg(build.join("top.config"))
        .arg("--names")
        .arg(build.join("top_routed.json"))
        .args([
            "--startup",
            "configured",
            "--scope",
            "observed",
            "--clock",
            "pin:G2",
            "--period-ns",
            "40",
            "--cycles",
        ])
        .arg(cycles.to_string())
        .arg("--vcd")
        .arg(vcd)
        .env_remove("ECP5_SWITCHES")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0")
        .stdout(Stdio::null());
    for p in probes {
        c.arg("--probe").arg(p);
    }
    if fast {
        c.arg("--fast");
    }
    if peripherals {
        c.arg("--peripherals");
    }
    c
}
fn success(c: &mut Command) {
    let Output { status, stderr, .. } = c.output().unwrap();
    assert!(
        status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&stderr)
    );
}
fn waves(vcd: &str) -> BTreeMap<String, Vec<(u64, char)>> {
    let mut names = BTreeMap::new();
    let mut values = BTreeMap::<String, Vec<(u64, char)>>::new();
    let mut time = 0;
    for line in vcd.lines() {
        if line.starts_with("$var ") {
            let words: Vec<_> = line.split_whitespace().collect();
            names.insert(
                words[3].to_string(),
                words[4].trim_start_matches('\\').to_string(),
            );
        } else if let Some(t) = line.strip_prefix('#') {
            time = t.parse().unwrap();
        } else if let Some(v @ ('0' | '1' | 'x' | 'z')) = line.chars().next() {
            let name = names.get(&line[1..]).expect("declared VCD signal");
            values.entry(name.clone()).or_default().push((time, v));
        }
    }
    values
}
fn button_run(
    build: &Path,
    fast: bool,
    peripherals: bool,
    set: Option<&str>,
    stimulus: &Path,
) -> String {
    let vcd = output_path("button.vcd");
    let mut c = command(
        build,
        fast,
        peripherals,
        70_000,
        &["pin:R1", "net:led"],
        &vcd,
    );
    c.arg("--stimulus").arg(stimulus);
    if let Some(set) = set {
        c.arg("--set").arg(set);
    }
    success(&mut c);
    std::fs::read_to_string(vcd).unwrap()
}
fn one_press(vcd: &str) {
    let w = waves(vcd);
    for i in 0..8 {
        assert_eq!(
            w[&format!("led[{i}]")].last().unwrap().1,
            if i == 0 { '1' } else { '0' }
        );
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn an_unpulled_button_with_future_stimulus_matches_both_executors() {
    let stimulus = root().join("examples/ulx3s/debounce/press.json");
    let kernel = button_run(debounce_none(), false, false, None, &stimulus);
    assert_eq!(
        waves(&kernel)["pin:R1"],
        [(0, '0'), (1000, '1'), (1300, '0'), (1700, '1')]
    );
    one_press(&kernel);
    for peripheral in [false, true] {
        let fast = button_run(debounce_none(), true, peripheral, None, &stimulus);
        assert_eq!(fast, kernel, "--peripherals={peripheral}: exact VCD");
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn explicit_button_sets_and_time_zero_stimulus_override_idle() {
    let press = root().join("examples/ulx3s/debounce/press.json");
    let zero = output_path("time-zero.json");
    let mut events: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(&press).unwrap()).unwrap();
    events.insert(0, serde_json::json!({"time":0,"set":{"net:fire1":"1"}}));
    std::fs::write(&zero, serde_json::to_vec(&events).unwrap()).unwrap();
    for (set, stimulus) in [
        (Some("pin:R1=1"), press.as_path()),
        (Some("net:fire1=1"), press.as_path()),
        (None, zero.as_path()),
    ] {
        let kernel = button_run(debounce_none(), false, false, set, stimulus);
        assert_eq!(waves(&kernel)["pin:R1"][0], (0, '1'));
        one_press(&kernel);
        for peripheral in [false, true] {
            assert_eq!(
                button_run(debounce_none(), true, peripheral, set, stimulus),
                kernel
            );
        }
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn unpatched_debounce_keeps_the_same_output() {
    let original = common::reference::build("debounce");
    let stimulus = root().join("examples/ulx3s/debounce/press.json");
    let released = button_run(&original, false, false, Some("pin:R1=0"), &stimulus);
    one_press(&released);
    assert_eq!(
        button_run(&original, false, false, None, &stimulus),
        released
    );
    assert_eq!(
        button_run(&original, true, false, None, &stimulus),
        released
    );
    assert_eq!(
        button_run(debounce_none(), true, false, None, &stimulus),
        released
    );
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn buttons_uart_and_switches_get_the_board_levels_in_both_executors() {
    let probes: Vec<_> = board::BUTTONS
        .iter()
        .map(|b| format!("pin:{}", b.ball))
        .chain(std::iter::once(format!("pin:{}", board::UART_RX)))
        .chain(board::SWITCHES.map(|b| format!("pin:{b}")))
        .collect();
    let refs: Vec<_> = probes.iter().map(String::as_str).collect();
    for setting in [None, Some("1010")] {
        let mut previous = None;
        for fast in [false, true] {
            let vcd = output_path("controls.vcd");
            let mut c = command(controls(), fast, false, 4, &refs, &vcd);
            if let Some(value) = setting {
                c.env("ECP5_SWITCHES", value);
            }
            success(&mut c);
            let text = std::fs::read_to_string(vcd).unwrap();
            let w = waves(&text);
            for b in board::BUTTONS {
                assert_eq!(
                    w[&format!("pin:{}", b.ball)][0],
                    (0, if b.idle { '1' } else { '0' })
                );
            }
            assert_eq!(w["pin:M1"][0], (0, '1'));
            let expected = if setting.is_some() {
                ['1', '1', '1', '0']
            } else {
                ['0', '1', '0', '0']
            };
            for (b, level) in board::SWITCHES.into_iter().zip(expected) {
                assert_eq!(w[&format!("pin:{b}")][0], (0, level));
            }
            if let Some(previous) = &previous {
                assert_eq!(&text, previous);
            } else {
                previous = Some(text);
            }
        }
    }
}

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn console_and_clock_inputs_keep_their_own_initial_levels() {
    let vcd = output_path("console.vcd");
    success(
        command(controls(), true, false, 4, &["pin:R1"], &vcd).args([
            "--console-out",
            "pin:L4",
            "--console-in",
            "net:btn[1]",
        ]),
    );
    assert_eq!(
        waves(&std::fs::read_to_string(vcd).unwrap())["pin:R1"][0],
        (0, '1')
    );
    let vcd = output_path("clock.vcd");
    let c = command(controls(), true, false, 4, &["pin:D6"], &vcd);
    // Replace the regular clock option with a clock on PWR (whose idle is 1).
    let args: Vec<_> = c.get_args().map(|a| a.to_os_string()).collect();
    let mut replacement = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
    let mut after_clock = false;
    for arg in args {
        replacement.arg(if after_clock {
            std::ffi::OsString::from("pin:D6")
        } else {
            arg.clone()
        });
        after_clock = arg == "--clock";
    }
    replacement
        .args(["--set", "pin:G2=0"])
        .env_remove("ECP5_SWITCHES")
        .env("FAST_CHOOSE", "0")
        .env("FAST_TUNE", "0")
        .stdout(Stdio::null());
    success(&mut replacement);
    assert_eq!(
        waves(&std::fs::read_to_string(vcd).unwrap())["pin:D6"][0],
        (0, '0')
    );
}

#[test]
#[ignore = "requires yosys/nextpnr and a local Trellis database"]
fn invalid_switch_positions_are_rejected_by_the_cli() {
    let vcd = output_path("invalid.vcd");
    let result = command(controls(), false, false, 1, &["pin:M1"], &vcd)
        .env("ECP5_SWITCHES", "1020")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("ECP5_SWITCHES must be exactly four 0/1")
    );
}
