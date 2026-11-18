//! Real Yosys/nextpnr ranges through CLI stimulus, probes and both executors.
#[path = "common/tools.rs"]
mod tools;
use std::{collections::BTreeMap, fs, path::PathBuf, process::Command};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn waves(text: &str) -> BTreeMap<String, Vec<(u64, char)>> {
    let mut names = BTreeMap::new();
    let mut values = BTreeMap::<String, Vec<(u64, char)>>::new();
    let mut time = 0;
    for line in text.lines() {
        if line.starts_with("$var ") {
            let words: Vec<_> = line.split_whitespace().collect();
            names.insert(words[3], words[4].trim_start_matches('\\'));
        } else if let Some(t) = line.strip_prefix('#') {
            time = t.parse().unwrap();
        } else if let Some(v @ ('0' | '1' | 'x' | 'z')) = line.chars().next() {
            values
                .entry(names[&line[1..]].into())
                .or_default()
                .push((time, v));
        }
    }
    values
}

#[test]
#[ignore = "requires Yosys, nextpnr, clang, the native image reader and a local Trellis database"]
fn offset_and_upto_ports_work_in_cli_stimuli_and_probes() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("ecp5-names-ranges-{}", std::process::id())));
    let dir = &scratch.0;
    fs::create_dir(dir).unwrap();
    fs::write(
        dir.join("top.v"),
        r#"module top(input clk, input [1:1] btn,
  input [7:4] data, input [0:3] asc, output reg [7:4] led = 0);
  always @(posedge clk) led <= data ^ asc ^ {4{btn[1]}};
endmodule
"#,
    )
    .unwrap();
    let mut lpf = String::new();
    for (name, ball) in [
        ("clk", "G2"),
        ("btn[1]", "R1"),
        ("data[4]", "E8"),
        ("data[5]", "D8"),
        ("data[6]", "D7"),
        ("data[7]", "E7"),
        ("asc[0]", "D6"),
        ("asc[1]", "T1"),
        ("asc[2]", "R18"),
        ("asc[3]", "V1"),
        ("led[4]", "B2"),
        ("led[5]", "C2"),
        ("led[6]", "C1"),
        ("led[7]", "D2"),
    ] {
        lpf += &format!("LOCATE COMP \"{name}\" SITE \"{ball}\";\nIOBUF PORT \"{name}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n");
    }
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    for (program, args) in [
        (
            "yosys",
            vec![
                "-q",
                "-p",
                "read_verilog top.v; synth_ecp5 -top top -json top.json",
            ],
        ),
        (
            "nextpnr-ecp5",
            vec![
                "--85k",
                "--package",
                "CABGA381",
                "--json",
                "top.json",
                "--lpf",
                "top.lpf",
                "--textcfg",
                "top.config",
                "--write",
                "routed.json",
                "--seed",
                "1",
                "-q",
            ],
        ),
    ] {
        let output = tools::command(program)
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fs::write(dir.join("press.json"), r#"[
      {"time":0,"set":{"net:btn[1]":"0","net:data[4]":"0","net:data[5]":"0","net:data[6]":"0","net:data[7]":"0","net:asc[0]":"0","net:asc[1]":"0","net:asc[2]":"0","net:asc[3]":"0"}},
      {"time":50,"set":{"net:btn[1]":"1","net:data[4]":"1","net:data[6]":"1"}},
      {"time":130,"set":{"net:btn[1]":"0","net:data[4]":"0","net:data[5]":"1","net:data[6]":"0","net:data[7]":"1","net:asc[0]":"1"}}
    ]"#).unwrap();
    for fast in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ecp5_emu"));
        command
            .current_dir(dir)
            .args([
                "emulate",
                "top.config",
                "--names",
                "routed.json",
                "--startup",
                "configured",
                "--scope",
                "observed",
                "--clock",
                "net:clk",
                "--period-ns",
                "40",
                "--cycles",
                "8",
                "--stimulus",
                "press.json",
                "--probe",
                "net:btn[1]",
                "--probe",
                "net:data",
                "--probe",
                "net:asc",
                "--probe",
                "net:led",
                "--vcd",
                "waves.vcd",
            ])
            .env_remove("ECP5_SWITCHES")
            .env("FAST_CHOOSE", "0")
            .env("FAST_TUNE", "0");
        if fast {
            command.arg("--fast");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "fast={fast}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let waves = waves(&fs::read_to_string(dir.join("waves.vcd")).unwrap());
        let value = |name: &str, time: u64| {
            waves[name]
                .iter()
                .rev()
                .find(|(t, _)| *t <= time)
                .unwrap()
                .1
        };
        assert_eq!(waves.len(), 13, "canonical probes only: {:?}", waves.keys());
        for (time, button, data, asc, led) in [
            (40, '0', 0, 0, 0),
            (100, '1', 5, 0, 10),
            (180, '0', 10, 1, 2),
            (300, '0', 10, 1, 2),
        ] {
            assert_eq!(value("btn[1]", time), button, "fast={fast} at {time}");
            for (bus, offset, expected) in [("data", 4, data), ("asc", 0, asc), ("led", 4, led)] {
                for i in 0..4 {
                    assert_eq!(
                        value(&format!("{bus}[{}]", offset + i), time),
                        if expected & (1 << i) == 0 { '0' } else { '1' },
                        "fast={fast} {bus} bit {i} at {time}"
                    );
                }
            }
        }
    }
}
