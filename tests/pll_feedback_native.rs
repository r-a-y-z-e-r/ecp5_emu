//! HC08 packed A-H: silicon frequency, full event/fast traces and refusal boundary.
#[path = "common/tools.rs"]
mod tools;
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, SignalId, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn checked(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Fixture {
    scratch: Scratch,
    model: Arc<DeviceModel>,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let scratch =
            Scratch(std::env::temp_dir().join(format!("hc08-pll-feedback-{}", std::process::id())));
        fs::create_dir(&scratch.0).unwrap();
        checked(
            Command::new("python3")
                .arg("-B")
                .arg(root.join("docs/inventory/LFE5U-85F/hc08_pll_analog/build.py"))
                .arg(scratch.0.join("build"))
                .args(["--profiles", "A", "B", "C", "D", "E", "F", "G", "H"]),
        );
        let db = Database::discover(None).unwrap();
        let model =
            Arc::new(DeviceModel::load(&db, &root.join("docs/inventory/LFE5U-85F")).unwrap());
        Self { scratch, model }
    }
    fn directory(&self, profile: char) -> PathBuf {
        self.scratch.0.join("build").join(profile.to_string())
    }
    fn compile(
        &self,
        bit: &Path,
        names: &Names,
        control: Option<&str>,
        raw_outputs: bool,
    ) -> anyhow::Result<(Compiled, Vec<SignalId>)> {
        let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let image = Arc::new(ConfigurationImage::load(
            bit,
            &self.model.database().root,
            None,
        )?);
        let device = Arc::new(ConfiguredDevice::bind(
            Arc::clone(&self.model),
            image,
            load_catalogue(&self.model, &evidence)?,
        )?);
        let mut probes: Vec<_> = ["clkop", "locked"]
            .into_iter()
            .flat_map(|name| names.expand(name).unwrap().into_iter().map(|(_, r)| r))
            .collect();
        let pll = self
            .model
            .resources()
            .find(|(_, r)| r.key.ends_with("/EHXPLL_UL"))
            .unwrap()
            .0;
        let port = |name: &str| {
            self.model
                .resource_ports(pll)
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .wire
                .clone()
        };
        if raw_outputs {
            probes.extend(["CLKOS", "CLKOS2", "CLKOS3"].map(|p| Reference::Wire(port(p))));
        }
        let mut drives = vec![(Reference::Pin("G2".into()), Logic::Zero)];
        if let Some(control) = control {
            drives.push((Reference::Wire(port(control)), Logic::Zero));
        }
        let references = probes.clone();
        let c = compile::compile(
            device,
            Request {
                package: "CABGA381".into(),
                scope: Scope::Observed,
                startup: Startup::Configured,
                drives,
                probes,
                options: Options {
                    trace_capacity: 0,
                    ..Options::default()
                },
            },
        )?;
        let signals = references.iter().map(|r| c.probes[&r.label()]).collect();
        Ok((c, signals))
    }
    fn changed(&self, name: &str, config: &str) -> PathBuf {
        let d = self.scratch.0.join(name);
        fs::create_dir(&d).unwrap();
        fs::write(d.join("top.config"), config).unwrap();
        checked(
            tools::command("ecppack")
                .current_dir(&d)
                .args(["top.config", "top.bit"]),
        );
        d.join("top.bit")
    }
}

/// Sample every ns, not just G2's 20 ns edges (which alias 100 MHz).
fn traces(mut c: Compiled, signals: Vec<SignalId>, expected_periods_ps: &[u64]) {
    let clock = c.drivers["pin:G2"];
    let lock = signals[1];
    let outputs: Vec<_> = signals
        .into_iter()
        .enumerate()
        .filter_map(|(i, s)| (i != 1).then_some(s))
        .collect();
    assert_eq!(outputs.len(), expected_periods_ps.len());
    assert_eq!(c.kernel.value(lock).unwrap(), Logic::One);
    let mut fast = FastSim::new(&c.kernel).unwrap();
    fast.clock(clock, Time::from_ps(40_000)).unwrap();
    let mut previous = vec![Logic::Zero; outputs.len()];
    let mut rising = vec![Vec::new(); outputs.len()];
    for ns in 1..=3000_u64 {
        let at = Time::from_ps(ns * 1000);
        if ns % 20 == 0 {
            c.kernel
                .schedule(at, clock, Logic::from_bool(ns % 40 == 20))
                .unwrap();
        }
        c.kernel.run_until(at).unwrap();
        fast.run_until(at).unwrap();
        assert_eq!(c.kernel.value(lock).unwrap(), Logic::One);
        assert_eq!(fast.value(lock), Some(Logic::One), "LOCK at {ns} ns");
        for (i, &output) in outputs.iter().enumerate() {
            let value = c.kernel.value(output).unwrap();
            assert_eq!(fast.value(output), Some(value), "output {i} at {ns} ns");
            if previous[i] == Logic::Zero && value == Logic::One {
                rising[i].push(ns * 1000);
            }
            previous[i] = value;
        }
    }
    for (edges, &period) in rising.iter().zip(expected_periods_ps) {
        assert!(edges.len() > 20, "too few CLKOP/CLKOS edges");
        assert!(
            edges.windows(2).all(|e| e[1] - e[0] == period),
            "expected {period} ps, got {edges:?}"
        );
    }
}

#[test]
#[ignore = "requires yosys, nextpnr-ecp5, ecppack, clang, native image reader and Trellis database"]
fn hc08_packed_feedback_matches_board_and_refuses_unmeasured_combinations() {
    let f = Fixture::new();
    for profile in 'A'..='H' {
        let d = f.directory(profile);
        let names = Names::load(&d.join("routed.json"), &f.model, "CABGA381").unwrap();
        let raw = matches!(profile, 'C' | 'D' | 'F' | 'G' | 'H');
        let (c, signals) = f.compile(&d.join("top.bit"), &names, None, raw).unwrap();
        assert!(c
            .report
            .resources
            .iter()
            .any(|r| r.family == "PLL" && r.rule.contains("HC08")));
        let periods = if raw {
            let os3 = match profile {
                'F' | 'H' => 30_000,
                'G' => 20_000,
                _ => 80_000,
            };
            vec![60_000, 80_000, 80_000, os3]
        } else {
            vec![10_000]
        };
        traces(c, signals, &periods);
        println!(
            "HC08 {profile}: kernel = fast, CLKOP {} ps, LOCK=1",
            periods[0]
        );
    }
    let d = f.directory('C');
    let base = fs::read_to_string(d.join("top.config")).unwrap();
    let names = Names::load(&d.join("routed.json"), &f.model, "CABGA381").unwrap();
    let refusal = |name: &str, config: String, control: Option<&str>| {
        let bit = f.changed(name, &config);
        let error = match f.compile(&bit, &names, control, false) {
            Err(e) => format!("{e:#}"),
            Ok(_) => panic!("accepted {name}"),
        };
        for word in ["FEEDBK_PATH", "CLKFB", "HC08"] {
            assert!(error.contains(word), "{name}: {error}");
        }
        println!("{name}: {error}");
    };
    for entry in 1..=3 {
        refusal(
            &format!("os2-fabric-{entry}"),
            base.replace("enum: FEEDBK_PATH INT_OP", "enum: FEEDBK_PATH INT_OS2")
                .replace(
                    "arc: E1_CLKFB E1_JCLKFB3",
                    &format!("arc: E1_CLKFB E1_JCLKFB{entry}"),
                ),
            None,
        );
    }
    refusal(
        "unknown-selector",
        base.replace("enum: FEEDBK_PATH INT_OP", "enum: FEEDBK_PATH USERCLOCK")
            .replace(
                "arc: E1_CLKFB E1_JCLKFB3",
                "arc: E1_CLKFB E1_CLKINTFB\nunknown: F3B1",
            ),
        None,
    );
    assert!(base.contains("arc: JCLK0 V00B0000"));
    refusal(
        "different-return",
        base.replace("arc: JCLK0 V00B0000", "arc: JCLK0 G_HPBX0100"),
        None,
    );
    for (control, enable) in [("RST", "PLLRST_ENA"), ("STDBY", "STDBY_ENABLE")] {
        refusal(
            &format!("live-{control}"),
            base.replace(
                &format!("enum: {enable} DISABLED"),
                &format!("enum: {enable} ENABLED"),
            ),
            Some(control),
        );
    }
    // A canonical INT_OS3 sets both mux bits too, but must remain output feedback.
    let e = f.directory('E');
    let control = fs::read_to_string(e.join("top.config"))
        .unwrap()
        .replace("enum: FEEDBK_PATH INT_OP", "enum: FEEDBK_PATH INT_OS3");
    let bit = f.changed("canonical-os3", &control);
    let names = Names::load(&e.join("routed.json"), &f.model, "CABGA381").unwrap();
    let (mut c, _) = f.compile(&bit, &names, None, false).unwrap();
    let clock = c.drivers["pin:G2"];
    let output = c.probes[&names.expand("clkop").unwrap()[0].1.label()];
    let mut edges = Vec::new();
    let mut old = Logic::Zero;
    // OS3 divider 8 -> VCO 800 MHz -> CLKOP period 7.5 ns.
    for half_ns in 1..=2000 {
        let at = Time::from_ps(half_ns * 500);
        if half_ns % 40 == 0 {
            c.kernel
                .schedule(at, clock, Logic::from_bool(half_ns % 80 == 40))
                .unwrap();
        }
        c.kernel.run_until(at).unwrap();
        let value = c.kernel.value(output).unwrap();
        if old == Logic::Zero && value == Logic::One {
            edges.push(half_ns * 500);
        }
        old = value;
    }
    assert!(edges.len() > 50 && edges.windows(2).all(|e| e[1] - e[0] == 7500));
}
