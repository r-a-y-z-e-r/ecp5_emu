//! CLI adapter for folder-based project builds, emulator launch and SRAM load.
use anyhow::{bail, ensure, Context, Result};
use ecp5_emu::flow::{self, BuildOptions, Event, Project, Severity};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

/// The commands of `ecp5_emu`, each of which takes `--help`.
const COMMANDS: [&str; 8] = [
    "new", "build", "run", "load", "device", "emulate", "inspect", "decode",
];

/// `COMMAND --help` (or `-h`) for every command, and the project commands;
/// leaves legacy bitstream/config commands to the existing CLI parser.
/// `help` is the CLI's whole help text, from which a command's own lines
/// are taken.
pub fn dispatch(command: &str, args: &[String], help: &str) -> Option<Result<()>> {
    if COMMANDS.contains(&command) && args.first().is_some_and(|s| s == "--help" || s == "-h") {
        print!("{}", command_help(command, help));
        return Some(Ok(()));
    }
    if command == "new" {
        return Some(create_project(args));
    }
    if command == "build"
        || (["run", "load"].contains(&command)
            && args.first().is_some_and(|s| {
                let p = Path::new(s);
                p.is_dir() || p.file_name().is_some_and(|n| n == "ecp5.toml")
            }))
    {
        Some(execute(command, args))
    } else {
        None
    }
}
/// A command's help: its usage lines from the CLI's help; for the project
/// commands what a project is and their options; for the others (and the
/// .bit/.config forms of run and load) the options section.
pub fn command_help(command: &str, help: &str) -> String {
    let prefix = format!("  ecp5_emu {command} ");
    let usage: Vec<&str> = help.lines().filter(|l| l.starts_with(&prefix)).collect();
    let mut text = format!("Usage:\n{}\n", usage.join("\n"));
    if command == "new" {
        text.push_str("  ecp5_emu new DIR --from-demo ulx3s/NAME|gallery/NAME [--name NAME]\n  ecp5_emu new --list-demos [--dev]\n");
        text.push_str(
            "\nCreate a generated project or copy a bundled demo into DIR; no build is started.
DIR must be absent or an empty directory. The suggested parent is ~/ecp5-projects.
  --name NAME       Display name (default: DIR's leaf)
  --resources LIST  Comma-separated resources (default: clock,leds); replaces defaults
                    Names: clock,leds,buttons,switches,uart,oled,hdmi,sd,audio,
                    sdram,adc,rtc,flash,header (HDMI uses GPDI; header inputs only)
                    Clock is required; buttons/switches require LEDs.
                    Initial demonstrations require LEDs or UART; peripherals start idle.
  --from-demo DEMO  Copy any ulx3s/NAME or gallery/NAME, including firmware and notices
                    Cannot be combined with --resources
  --list-demos      Print showcase demo names, one per line; no DIR is needed
  --dev             With --list-demos, include every demo (Dev mode)
                    Showcase selection: examples/showcase.toml; absent means all demos
",
        );
        return text;
    }
    if ["build", "run", "load"].contains(&command) {
        text.push_str(
            "\nPROJECT_DIR is a folder with Verilog sources, one .lpf and optionally ecp5.toml
(its format: src/flow/project.rs). The build runs yosys, nextpnr-ecp5 and ecppack
into PROJECT_DIR/build and skips the steps whose inputs are unchanged.
  --out DIR    Build into DIR instead of PROJECT_DIR/build
  --force      Run every step even when its inputs are unchanged
  --verbose    Print every line of the tools' output
",
        );
    }
    match command {
        "run" => text.push_str(
            "  run PROJECT_DIR builds the project and opens it in the window (ecp5_gui --project).\n",
        ),
        "load" => text.push_str(
            "  --wait       Wait while another run holds the board (out/board.lock)
  load PROJECT_DIR writes the bitstream into the ULX3S's SRAM with openFPGALoader,
  never into its flash.
",
        ),
        _ => {}
    }
    if command != "build" {
        if let Some(at) = help.find("\nOptions:") {
            text.push_str(&help[at..]);
        }
    }
    text
}
fn parse_new(args: &[String]) -> Result<(flow::new_project::Options, Option<String>)> {
    let mut args = args.iter();
    let dir = args
        .next()
        .filter(|s| !s.starts_with('-'))
        .context("missing project folder")?;
    let mut options = flow::new_project::Options::new(dir);
    let mut resources = None;
    let mut demo = None;
    let mut name = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--name" => {
                ensure!(name.is_none(), "duplicate --name");
                name = Some(args.next().context("missing value for --name")?.clone());
            }
            "--resources" => {
                ensure!(resources.is_none(), "duplicate --resources");
                resources = Some(args.next().context("missing value for --resources")?);
            }
            "--from-demo" => {
                ensure!(demo.is_none(), "duplicate --from-demo");
                demo = Some(args.next().context("missing value for --from-demo")?);
            }
            _ => bail!("unknown new option {option}"),
        }
    }
    ensure!(
        resources.is_none() || demo.is_none(),
        "--from-demo and --resources are mutually exclusive"
    );
    if let Some(name) = name {
        options.name = name;
    }
    if let Some(resources) = resources {
        options.resources = resources
            .split(',')
            .map(flow::new_project::Resource::parse)
            .collect::<Result<_>>()?;
    }
    options.validate()?;
    Ok((options, demo.cloned()))
}

fn create_project(args: &[String]) -> Result<()> {
    if args
        .first()
        .is_some_and(|arg| arg == "--list-demos" || arg == "--dev")
    {
        let mut list = false;
        let mut dev = false;
        for arg in args {
            match arg.as_str() {
                "--list-demos" if !list => list = true,
                "--dev" if !dev => dev = true,
                _ => bail!("unknown or duplicate demo listing option {arg}"),
            }
        }
        ensure!(list, "--dev requires --list-demos");
        let catalogue = flow::demo_copy::catalogue()?;
        for demo in catalogue.visible(dev) {
            println!("{}", demo.name);
        }
        for (demo, reason) in catalogue.excluded {
            eprintln!("Excluded {demo}: {reason}");
        }
        for demo in catalogue.unknown_showcase {
            eprintln!("Unknown showcase demo: {demo}");
        }
        return Ok(());
    }
    let (options, demo) = parse_new(args)?;
    let project = if let Some(demo) = &demo {
        flow::demo_copy::create(&flow::demo_copy::Options {
            demo: demo.clone(),
            dir: options.dir,
            name: options.name,
        })?
    } else {
        flow::new_project::create(&options)?
    };
    // Shell quoting keeps the printed next steps usable with spaces and quotes.
    let dir = format!(
        "'{}'",
        project.root.display().to_string().replace('\'', "'\\''")
    );
    println!(
        "Created {}\nNext steps:\n  ecp5_emu build {dir}\n  ecp5_emu run {dir}",
        project.root.display()
    );
    if let Some(demo) = demo {
        println!("Copied demo: {demo}");
        if let Some(note) = flow::demo_copy::runtime_inputs(&demo) {
            println!("{note}");
        }
    }
    Ok(())
}
fn execute(command: &str, args: &[String]) -> Result<()> {
    let mut args = args.iter();
    let path = args.next().context("missing project folder")?;
    let mut project = Project::open(path)?;
    let mut options = BuildOptions::default();
    let mut verbose = false;
    let mut wait = false;
    let mut out = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--out" => {
                let dir = args.next().context("missing value for --out")?;
                project.set_build_dir(dir)?;
                out = Some(dir);
            }
            "--force" => options.force = true,
            "--verbose" => verbose = true,
            "--wait" if command == "load" => wait = true,
            _ => bail!("unknown {command} option {option}"),
        }
    }
    println!(
        "Project: {} ({}) — top {}, {}/{}",
        project.name,
        project.root.display(),
        project.top,
        project.device.part,
        project.device.package
    );
    let total = if project.memory_images.is_empty() {
        println!("Memory images (ecpbram): none listed — $readmemh files are built in by Yosys");
        3
    } else {
        4
    };
    let mut index = 0;
    let mut progress_open = false;
    let cwd = std::env::current_dir()?;
    let outcome = flow::build(&project, &options, &mut |event| match event {
        Event::Started { step, command } => {
            index += 1;
            if verbose {
                println!("[{index}/{total}] {step} …\n{command}");
            } else {
                print!("[{index}/{total}] {step} … ");
                let _ = io::stdout().flush();
                progress_open = true;
            }
        }
        Event::Skipped { step } => {
            index += 1;
            println!("[{index}/{total}] {step} … skipped: inputs unchanged");
        }
        Event::Output { line, .. } if verbose => println!("{line}"),
        Event::Message(mut message)
            if matches!(message.severity, Severity::Warning | Severity::Error) =>
        {
            if progress_open {
                println!();
                let _ = io::stdout().flush();
                progress_open = false;
            }
            if let Some(file) = &message.file {
                message.file = Some(file.strip_prefix(&cwd).unwrap_or(file).into());
            }
            eprintln!("{message}");
        }
        Event::Finished { step, ok, seconds } => {
            if !progress_open {
                print!("[{index}/{total}] {step} … ");
            }
            if ok {
                println!("done in {seconds:.1} s");
            } else {
                println!("failed");
            }
            progress_open = false;
        }
        _ => {}
    });
    ensure!(!outcome.cancelled, "build cancelled");
    ensure!(outcome.ok, "project build failed; see diagnostics above");
    if let Some(report) = &outcome.report {
        if let Some(frontend) = &report.frontend {
            println!("Frontend: {}", frontend.summary());
        }
        for note in &report.notes {
            println!("Note: {note}");
        }
        println!("Utilisation: {}", report.summary());
        for line in report.timing_lines() {
            println!("Timing: {line}");
        }
    }
    println!(
        "Emulator: {}\nBoard: {}\nHDL names: {}",
        outcome.outputs.config.display(),
        outcome.outputs.bit.display(),
        outcome.outputs.routed_json.display()
    );
    if command == "load" {
        let lock = flow::board::lock_path();
        // With --wait, a board held by another run is waited for (the lock
        // is taken again here only to see that it is free; load takes it).
        if wait {
            let mut said = false;
            loop {
                match flow::board::BoardLock::take(&lock, "ecp5_emu-wait") {
                    Ok(free) => break drop(free),
                    Err(busy) if !said => {
                        eprintln!("{busy:#}; waiting …");
                        said = true;
                    }
                    Err(_) => {}
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
        flow::board::load(
            &outcome.outputs.bit,
            &options.tools,
            Some(&lock),
            &mut |line| println!("{line}"),
        )?;
        println!("Loaded {} into the ULX3S's SRAM (not flash): it runs until power-off or the next load.", outcome.outputs.bit.display());
    } else if command == "run" {
        project.emulatable().map_err(anyhow::Error::msg)?;
        let gui = gui_program()?;
        let mut child = Command::new(gui);
        child.args(["--project", path]);
        if let Some(out) = out {
            child.args(["--out", out]);
        }
        let status = child.arg("--run").status().map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "ecp5_gui not found; run cargo build --release -p ecp5_gui or set ECP5_GUI"
                )
            } else {
                e.into()
            }
        })?;
        ensure!(status.success(), "ecp5_gui failed ({status})");
    }
    Ok(())
}
fn gui_program() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ECP5_GUI") {
        return Ok(path.into());
    }
    let sibling = std::env::current_exe()?
        .parent()
        .context("executable has no parent")?
        .join(if cfg!(windows) {
            "ecp5_gui.exe"
        } else {
            "ecp5_gui"
        });
    Ok(if sibling.is_file() {
        sibling
    } else {
        PathBuf::from("ecp5_gui")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use flow::new_project::Resource;
    fn new_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn new_defaults_and_explicit_resources() {
        let folder = std::env::temp_dir().join(format!("new CLI {}", std::process::id()));
        let dir = folder.join("folder with 'quotes'");
        let dir = dir.to_str().unwrap();
        let (options, demo) = parse_new(&new_args(&[dir])).unwrap();
        assert!(demo.is_none());
        assert_eq!(options.name, "folder with 'quotes'");
        assert_eq!(options.resources, [Resource::Clock, Resource::Leds].into());
        let (options, _) = parse_new(&new_args(&[
            dir,
            "--name",
            "My \"project\"",
            "--resources",
            "uart,clock",
        ]))
        .unwrap();
        assert_eq!(options.name, "My \"project\"");
        assert_eq!(options.resources, [Resource::Clock, Resource::Uart].into());
        assert!(!folder.exists());
    }
    #[test]
    fn new_rejects_bad_arguments_and_mixed_demo_resources() {
        let dir = std::env::temp_dir().join(format!("new CLI errors {}", std::process::id()));
        let dir = dir.to_str().unwrap();
        for tail in [
            vec!["--name"],
            vec!["--resources"],
            vec!["--from-demo"],
            vec!["--build"],
            vec!["--resources", "leds"],
            vec!["--resources", "clock"],
            vec!["--resources", "clock,buttons"],
            vec!["--resources", "clock,leds,unknown"],
            vec!["--resources", "clock,leds,"],
            vec!["--resources", ""],
            vec!["--name", "  "],
            vec!["--name", "a", "--name", "b"],
            vec!["--resources", "clock,uart", "--resources", "clock,leds"],
            vec!["--from-demo", "a", "--from-demo", "b"],
        ] {
            let mut args = vec![dir];
            args.extend(tail);
            assert!(parse_new(&new_args(&args)).is_err(), "{args:?}");
        }
        assert!(parse_new(&[]).is_err());
        let (_, demo) = parse_new(&new_args(&[dir, "--from-demo", "ulx3s/uart_hello"])).unwrap();
        assert_eq!(demo.as_deref(), Some("ulx3s/uart_hello"));
        let error = parse_new(&new_args(&[
            dir,
            "--from-demo",
            "ulx3s/uart_hello",
            "--resources",
            "clock,leds",
        ]))
        .unwrap_err();
        assert!(error.to_string().contains("mutually exclusive"));
        assert!(!Path::new(dir).exists());
    }
    #[test]
    fn legacy_files_are_not_projects() {
        assert!(dispatch("run", &["foo.bit".into()], "").is_none());
        assert!(dispatch("load", &["foo.config".into()], "").is_none());
    }
}
