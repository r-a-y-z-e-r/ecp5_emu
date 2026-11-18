//! The DP16KD fixtures of tests/ebr_native.rs, which tests/fast_mode.rs runs
//! too: port A 18 bits WRITETHROUGH without output register, port B 18 bits
//! READBEFOREWRITE with OUTREG, CS decoders, inverted CLKA/CEA/CSA0, and WEA
//! inverted (`inverted_write`) or tied low (`read_only_a`). Each is built in
//! its own folder of target/ with yosys, nextpnr-ecp5 and ecppack from PATH
//! or their environment overrides.
#![allow(dead_code)]
#[path = "../common/tools.rs"]
mod tools;
use ecp5_emu::model::DeviceModel;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Runs `program` in `dir`, its output in `log`; panics when it fails.
pub fn run(dir: &Path, program: &str, args: &[&str], log: &str) {
    let out = tools::command(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("{program}: {e}"));
    fs::write(dir.join(log), [out.stdout, out.stderr].concat()).unwrap();
    assert!(
        out.status.success(),
        "{program}: see {}",
        dir.join(log).display()
    );
}

/// The word INIT gives `address`.
pub fn init(address: usize) -> usize {
    ((address * 181) ^ 0x2a155) & 0x3ffff
}

/// The fixture's folder.
pub fn dir(inverted_write: bool, read_only_a: bool) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if read_only_a {
        "target/ebr-readonly-a"
    } else if inverted_write {
        "target/ebr-inverted-wea"
    } else {
        "target/ebr-execution"
    })
}

/// The fixture's folder, built first if it lacks top.lpf, top.config or
/// top.bit (a fresh checkout).
pub fn ensure(inverted_write: bool, read_only_a: bool) -> PathBuf {
    let dir = dir(inverted_write, read_only_a);
    if ["top.lpf", "top.config", "top.bit"]
        .iter()
        .all(|f| dir.join(f).exists())
    {
        return dir;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let db = ecp5_emu::database::Database::discover(None).unwrap();
    let model = DeviceModel::load(&db, &root.join("docs/inventory/LFE5U-85F")).unwrap();
    build(&model, inverted_write, read_only_a).0
}

/// Builds the fixture: its folder and the ball of each top-level pin.
pub fn build(
    model: &DeviceModel,
    inverted_write: bool,
    read_only_a: bool,
) -> (PathBuf, BTreeMap<String, String>) {
    let dir = dir(inverted_write, read_only_a);
    fs::create_dir_all(&dir).unwrap();
    let mut pinmap = BTreeMap::from([
        ("clka".to_string(), "G2".to_string()),
        ("clkb".into(), "J4".into()),
    ]);
    let mut balls = model
        .package_pins()
        .filter(|p| {
            p.package == "CABGA381" && p.pio.is_some() && !["G2", "J4"].contains(&p.ball.as_str())
        })
        .map(|p| p.ball.clone())
        .collect::<Vec<_>>()
        .into_iter();
    for side in ['a', 'b'] {
        for prefix in ["ce", "oce", "we", "rst"] {
            pinmap.insert(format!("{prefix}{side}"), balls.next().unwrap());
        }
        for (prefix, width) in [("cs", 3), ("ad", 10), ("be", 2), ("di", 18), ("q", 18)] {
            for bit in 0..width {
                pinmap.insert(format!("{prefix}{side}[{bit}]"), balls.next().unwrap());
            }
        }
    }
    pinmap.insert("captured".into(), balls.next().unwrap());
    if read_only_a {
        pinmap.insert("wea_echo".into(), balls.next().unwrap());
    }
    let lpf=pinmap.iter().map(|(p,b)|format!("LOCATE COMP \"{p}\" SITE \"{b}\";\nIOBUF PORT \"{p}\" IO_TYPE=LVCMOS33 PULLMODE=NONE;\n")).collect::<String>();
    fs::write(dir.join("top.lpf"), lpf).unwrap();
    let mut params = vec![
        ".DATA_WIDTH_A(18)".into(),
        ".DATA_WIDTH_B(18)".into(),
        ".REGMODE_B(\"OUTREG\")".into(),
        ".WRITEMODE_A(\"WRITETHROUGH\")".into(),
        ".WRITEMODE_B(\"READBEFOREWRITE\")".into(),
        ".CLKAMUX(\"INV\")".into(),
        ".CEAMUX(\"INV\")".into(),
        ".CSA0MUX(\"INV\")".into(),
        ".CSDECODE_A(\"0b011\")".into(),
        ".CSDECODE_B(\"0b110\")".into(),
        ".GSR(\"ENABLED\")".into(),
    ];
    if inverted_write {
        params.push(".WEAMUX(\"INV\")".into());
    }
    for group in 0..64 {
        let hex = (0..16)
            .rev()
            .map(|word| format!("{:05x}", init(group * 16 + word)))
            .collect::<String>();
        params.push(format!(".INITVAL_{group:02X}(320'h{hex})"));
    }
    let mut connections = Vec::new();
    for side in ['A', 'B'] {
        let l = side.to_ascii_lowercase();
        for p in ["CLK", "CE", "OCE", "WE", "RST"] {
            connections.push(format!(
                ".{p}{side}({})",
                if read_only_a && side == 'A' && p == "WE" {
                    "1'b0".into()
                } else {
                    format!("{}{l}", p.to_ascii_lowercase())
                }
            ));
        }
        for bit in 0..3 {
            connections.push(format!(".CS{side}{bit}(cs{l}[{bit}])"));
        }
        for bit in 0..14 {
            connections.push(format!(
                ".AD{side}{bit}({})",
                if bit >= 4 {
                    format!("ad{l}[{}]", bit - 4)
                } else if bit < 2 {
                    format!("be{l}[{bit}]")
                } else {
                    "1'b0".into()
                }
            ));
        }
        for bit in 0..18 {
            connections.push(format!(".DI{side}{bit}(di{l}[{bit}])"));
            connections.push(format!(".DO{side}{bit}(q{l}[{bit}])"));
        }
    }
    let hdl=format!("module top(input clka,clkb,cea,ceb,ocea,oceb,wea,web,rsta,rstb,input [2:0] csa,csb,input [9:0] ada,adb,input [1:0] bea,beb,input [17:0] dia,dib,output [17:0] qa,qb,output captured);\nDP16KD #(\n{}\n) ram (\n{}\n);\nTRELLIS_FF #(.CLKMUX(\"INV\"),.GSR(\"ENABLED\")) observer(.CLK(clka),.LSR(rsta),.CE(1'b1),.DI(qa[0]),.Q(captured));\nendmodule\n",params.join(",\n"),connections.join(",\n"));
    let hdl = if read_only_a {
        hdl.replace(
            "output captured);",
            "output captured, output wea_echo);\nassign wea_echo=wea;",
        )
    } else {
        hdl
    };
    fs::write(dir.join("top.v"), hdl).unwrap();
    run(
        &dir,
        "yosys",
        &[
            "-p",
            "read_verilog top.v; synth_ecp5 -top top -json top.json",
        ],
        "yosys.log",
    );
    run(
        &dir,
        "nextpnr-ecp5",
        &[
            "--85k",
            "--package",
            "CABGA381",
            "--json",
            "top.json",
            "--lpf",
            "top.lpf",
            "--textcfg",
            "top.config",
            "--seed",
            "1",
            "--timing-allow-fail",
        ],
        "nextpnr.log",
    );
    run(
        &dir,
        "ecppack",
        &["--compress", "top.config", "top.bit"],
        "ecppack.log",
    );
    (dir, pinmap)
}
