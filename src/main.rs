use anyhow::{bail, ensure, Context, Result};
use ecp5_emu::{
    chip::{coverage, expand_groups, Chip},
    config::Config,
    database::Database,
    decode::{Catalog, Cram},
    image::ConfigurationImage,
    logic::Logic,
    model::DeviceModel,
    prepared::{load_catalogue, ConfiguredDevice},
    state::DeviceState,
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::{BufWriter, Write},
    path::PathBuf,
    sync::Arc,
};
mod command_emulate;
mod command_flow;

const HELP: &str = "ecp5_emu — headless ECP5-85F digital emulator

Usage:
  ecp5_emu new DIR [--name NAME] [--resources clock,leds,buttons,switches,uart]  Create a new ULX3S project
  ecp5_emu build PROJECT_DIR [--out DIR] [--force] [--verbose]  Build Verilog + .lpf + optional ecp5.toml
  ecp5_emu run PROJECT_DIR   Build a project and open the emulator
  ecp5_emu load PROJECT_DIR  Build a project and load ULX3S SRAM, never flash
  ecp5_emu device [--db PATH] [--architecture PATH] [--output model-summary.json]
  ecp5_emu load DESIGN.bit [--architecture PATH] [--image-reader PATH] [--output snapshot-summary.json]
  ecp5_emu emulate DESIGN.bit --startup configured --scope observed --clock pin:G2 --probe pin:F1 [--vcd signals.vcd]
  ecp5_emu emulate DESIGN.bit --names DESIGN.json --startup configured --scope observed --clock net:clk_25mhz --probe net:count
  ecp5_emu emulate DESIGN.bit --fast --startup configured --scope observed --clock pin:G2 --period-ns 40 --cycles 25000000 --probe pin:L4 --vcd uart.vcd
  ecp5_emu emulate DESIGN.bit --fast --startup configured --scope observed --clock pin:G2 --period-ns 40 --console-out pin:L4 --console-in pin:M1
  ecp5_emu inspect DESIGN.config [--db PATH]
  ecp5_emu inspect DESIGN.bit [--db PATH]
  ecp5_emu decode DESIGN.bit --resource LFE5U-85F/R22C105/EBR0 --output decoded.json
  ecp5_emu run DESIGN.bit --clock pin:G2 --cycles 16 --probe R38C2:Q3
  ecp5_emu run DESIGN.config --stimulus events.json --vcd signals.vcd

Options:
  --db PATH          Trellis database root (or ECP5_DB)
  --catalog PATH     Resource decode catalogue (decode command)
  --architecture PATH  Pinned evidence directory (device/load/emulate)
  --image-reader PATH  Native image reader (load/emulate; or ECP5_IMAGE_READER)
  --resource ID      Select a resource in decode output (repeatable)
  --output FILE      Save the command report as JSON
  --startup PROFILE  Required: ready-unknown, configured (flip-flops at REGSET, as observed
                     on the board) or wake-phases (explicit sys:step phases after loading)
  --scope SCOPE      emulate: all-plc (default) or observed
  --package NAME     Physical package, default CABGA381
  --clock pin:PIN    Clock input for a simple cycle run
  --cycles N         Number of rising edges, default 16 (with a console: until Ctrl-C)
  --period-ns N      Clock period in ns, even, default 2 (40 for the 25 MHz board clock)
  --fast             emulate: run the plan compiled to C (clang on PATH or ECP5_CLANG;
                     wrapper for zig cc): two-valued, no trace;
                     prints the values at the start and at the end, --vcd gets every edge.
                     Covers LUT, carry, FF, LUTRAM, PIO, IOLOGIC, EBR, PLL, OSC and MULT18
  --peripherals      emulate --fast: attach ULX3S peripherals from the configured balls;
                     use ECP5_SD_IMAGE, ECP5_FLASH_IMAGE, ECP5_RTC_IMAGE, ECP5_ADC_VOLTS, ECP5_EDID_IMAGE.
                     EDID is present with the HDMI model; ECP5_EDID=off disconnects its EEPROM.
                     No tuning while tracing; final stderr lines: peripheral: MODEL key=value ...
                     Models: sdram, sd, flash, adc, rtc, oled, hdmi, edid, header, audio; decimal counters,
                     uppercase hex CRCs; SDRAM diagnostics: peripheral: sdram message: TEXT.
                     Reports (N decimal, H hex, - unavailable; see docs/PERIPHERALS.md):
                     peripheral: sdram init=yes|no reads=N writes=N timing_violations=N unknown_samples=N refresh_shortfalls=N
                     peripheral: sd state=STATE last_command=N last_argument=H blocks_read=N blocks_written=N crc_errors=N illegal_commands=N mode=spi|sd1|sd4 card_state=STATE rca=HHHH bus_width=1|4 commands=N responses=N dat3_pullup=0|1
                     peripheral: flash jedec=H last_command=H last_address=H bytes_read=N ignored_writes=N programs=N erases=N protected=N busy_ignored=N config_writes=N
                     Flash writes change memory only; ignored_writes counts rejected writes (unsupported,
                     malformed, no WEL, protected or busy), not accepted WREN/WRDI/program/erase/WRSR.
                     Configuration-area attempts add: peripheral: flash warning: TEXT.
                     This would overwrite the board's FPGA configuration (bitstream) on a real ULX3S.
                     The board rule remains SRAM loads only: never program or erase its flash.
                     peripheral: adc frames=N unsupported_frames=N mode=H codes=H,H,H,H,H,H,H,H
                     peripheral: rtc time=YYYY-MM-DDTHH:MM:SS st=0|1 oscrun=0|1 last_bus_address=H last_address=H bytes_read=N bytes_written=N nacks=N
                     peripheral: oled frames=N crc32=H commands=N data_bytes=N errors=N unknown_commands=N display_on=yes|no
                     peripheral: hdmi frames=N width=N height=N crc32=H invalid_symbols=N relocks=N
                     peripheral: edid present=yes|no reads=N bytes_read=N last_pointer=HH nacks=N
                     peripheral: header NAME=FHz/D% ... (outputs in J1/J2 pad order; F Hz, D percent, 3 decimals; -- unavailable)
                     peripheral: audio l_hz=F l_vpp=F r_hz=F r_vpp=F v_hz=F v_vpp=F transitions=N
                     Audio F: decimal Hz (3 places) or volts (6 places), -- if unavailable.
  ECP5_HEADER        GP/GN input defaults, e.g. GP0=1,GN3=0; values 0,1,z (released).
                     --set and time-0 stimulus win. Released input = pull (NONE -> 0); bidirectional = Z.
  --set pin:PIN=V    Initial input value: 0, 1, x, z (repeatable)
                     emulate defaults for configured board inputs: buttons released (PWR=1,
                     others=0), UART RX (M1)=1, DIP switches from ECP5_SWITCHES (default OFF);
                     ON=1, OFF=pad pull (NONE/unknown=0), in both kernel and fast mode.
                     --set pin:R1=1 or time-0 stimulus overrides a default; later stimulus
                     changes it at its time. --clock, --console-in and peripheral answers win
  --probe REFERENCE  Package pin, internal wire, or net:NAME (repeatable)
  --names FILE       nextpnr's routed JSON (nextpnr --write FILE), so that net:NAME means
                     the design's own signal; a bus name without [i] expands to its bits
  --stimulus FILE    JSON array of {time, set: {\"pin:G2\": \"1\"}}
  --vcd FILE         Write observed signals as VCD, timescale 1ns
  --osc-hz N         OSCG base frequency before the divider, default the nominal 310000000;
                     each chip differs (one ULX3S measured 311.8 MHz)
  --console-out REF  emulate --fast: a UART terminal on this design output (8N1): its bytes
                     go to stdout, and the status lines to stderr
  --console-in REF   The design input that the terminal's keys drive, idle high; on a
                     terminal, Enter sends CR and Ctrl-C ends the run
  --baud N           Console baud rate, default 115200

inspect reports unsupported features. run rejects active unmodelled resources.
Current logic models: LUT4, CCU2, LUT5/6/7 muxes, FFs, basic PIO, static global clocks.
emulate uses the new directed event kernel; run remains the earlier prototype.
";

#[derive(Deserialize)]
struct Event {
    time: u64,
    #[serde(default)]
    set: BTreeMap<String, String>,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        print!("{HELP}");
        return Ok(());
    };
    if ["--help", "-h", "help"].contains(&command.as_str()) {
        print!("{HELP}");
        return Ok(());
    }
    let remaining: Vec<String> = args.collect();
    if let Some(result) = command_flow::dispatch(&command, &remaining, HELP) {
        return result;
    }
    let mut args = remaining.into_iter();
    if command == "device" {
        let mut database = None;
        let mut architecture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F");
        let mut output = None;
        while let Some(option) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("missing value for {option}"))?;
            match option.as_str() {
                "--db" => database = Some(PathBuf::from(value)),
                "--architecture" => architecture = value.into(),
                "--output" => output = Some(PathBuf::from(value)),
                _ => bail!("unknown device option {option}"),
            }
        }
        let model = DeviceModel::load(&Database::discover(database.as_deref())?, &architecture)?;
        let summary = model.summary();
        println!(
            "Device: {}; tiles: {}; resources: {}",
            summary.device, summary.tiles, summary.resources
        );
        println!(
            "Site references: {}; bindings: {:?}",
            summary.site_references, summary.site_bindings
        );
        println!(
            "Route descriptions: {} (directed, configuration-independent)",
            summary.placed_route_descriptions
        );
        println!(
            "Package balls: {:?}; bonded GPIO references: {}",
            summary.package_balls, summary.bonded_pio_references
        );
        println!("Architecture loaded; emulate uses the new event kernel, run uses the earlier prototype.");
        if let Some(path) = output {
            let mut file = BufWriter::new(std::fs::File::create(path)?);
            serde_json::to_writer_pretty(&mut file, &summary)?;
            writeln!(file)?;
            file.flush()?;
        }
        return Ok(());
    }
    ensure!(
        ["inspect", "run", "decode", "load", "emulate"].contains(&command.as_str()),
        "unknown command {command}; use --help"
    );
    let path = PathBuf::from(args.next().context("missing design file")?);
    let (mut db_path, mut vcd_path, mut stimulus) = (None, None, None);
    let mut package = "CABGA381".to_string();
    let (mut clock, mut cycles) = (None, None);
    let (mut probes, mut initial) = (Vec::new(), Vec::new());
    let (mut catalog_path, mut output_path) = (None, None);
    let (mut architecture_path, mut image_reader) = (None, None);
    let mut resources = Vec::new();
    let (mut startup, mut scope) = (None, None);
    let (mut oscillator_hz, mut names) = (None, None);
    let (mut fast, mut period_ns) = (false, 2_u64);
    let mut peripherals = false;
    let (mut console_out, mut console_in, mut baud) = (None, None, None);
    while let Some(option) = args.next() {
        if option == "--fast" {
            fast = true;
            continue;
        }
        if option == "--peripherals" {
            peripherals = true;
            continue;
        }
        let value = args
            .next()
            .with_context(|| format!("missing value for {option}"))?;
        match option.as_str() {
            "--db" => db_path = Some(PathBuf::from(value)),
            "--catalog" => catalog_path = Some(PathBuf::from(value)),
            "--architecture" => architecture_path = Some(PathBuf::from(value)),
            "--image-reader" => image_reader = Some(PathBuf::from(value)),
            "--startup" => startup = Some(value),
            "--scope" => scope = Some(value),
            "--output" => output_path = Some(PathBuf::from(value)),
            "--resource" => resources.push(value),
            "--package" => package = value,
            "--clock" => clock = Some(value),
            "--cycles" => cycles = Some(value.parse().context("invalid cycle count")?),
            "--probe" => probes.push(value),
            "--set" => {
                let (pin, v) = value
                    .split_once('=')
                    .context("expected --set pin:PIN=VALUE")?;
                initial.push((pin.into(), v.parse::<Logic>()?));
            }
            "--vcd" => vcd_path = Some(PathBuf::from(value)),
            "--stimulus" => stimulus = Some(PathBuf::from(value)),
            "--names" => names = Some(PathBuf::from(value)),
            "--period-ns" => period_ns = value.parse().context("invalid clock period")?,
            "--console-out" => console_out = Some(value),
            "--console-in" => console_in = Some(value),
            "--baud" => baud = Some(value.parse().context("invalid baud rate")?),
            "--osc-hz" => {
                oscillator_hz = Some(value.parse().context("invalid oscillator frequency")?)
            }
            other => bail!("unknown option {other}"),
        }
    }
    let db_path = Database::discover(db_path.as_deref())?;
    if command == "emulate" {
        let startup=match startup.as_deref() {
            Some("ready-unknown")=>ecp5_emu::compile::Startup::ReadyUnknown,
            Some("configured")=>ecp5_emu::compile::Startup::Configured,
            Some("wake-phases")=>ecp5_emu::compile::Startup::WakePhases,
            _=>bail!("emulate requires --startup ready-unknown, configured or wake-phases; POR/transport/physical START timing are not implemented"),
        };
        ensure!(
            catalog_path.is_none() && resources.is_empty(),
            "emulate does not use --catalog/--resource; --output includes the new event trace"
        );
        let scope = match scope.as_deref().unwrap_or("all-plc") {
            "all-plc" => ecp5_emu::compile::Scope::AllPlcRegisters,
            "observed" => ecp5_emu::compile::Scope::Observed,
            _ => bail!("expected --scope all-plc or observed"),
        };
        let architecture = architecture_path.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F")
        });
        let model = Arc::new(DeviceModel::load(&db_path, &architecture)?);
        let image = Arc::new(ConfigurationImage::load(
            &path,
            &db_path,
            image_reader.as_deref(),
        )?);
        let catalogue = load_catalogue(&model, &architecture)?;
        let device = Arc::new(ConfiguredDevice::bind(model, image, catalogue)?);
        return command_emulate::run(
            device,
            command_emulate::Options {
                startup,
                package,
                scope,
                initial,
                probes,
                clock,
                cycles,
                stimulus,
                output: output_path,
                vcd: vcd_path,
                oscillator_hz,
                names,
                fast,
                peripherals,
                period_ns,
                console: match (console_out, console_in) {
                    (Some(output), input) => Some(command_emulate::ConsoleOptions {
                        output,
                        input,
                        baud: baud.unwrap_or(115_200),
                    }),
                    (None, None) if baud.is_none() => None,
                    _ => bail!("--console-in and --baud need --console-out"),
                },
            },
        );
    }
    ensure!(
        console_out.is_none() && console_in.is_none() && baud.is_none(),
        "--console-out, --console-in and --baud belong to emulate"
    );
    ensure!(!peripherals, "--peripherals belongs to emulate --fast");
    let cycles = cycles.unwrap_or(16);
    ensure!(
        !fast && period_ns == 2,
        "--fast and --period-ns belong to emulate"
    );
    ensure!(
        startup.is_none() && scope.is_none(),
        "--startup/--scope require emulate"
    );
    if command == "load" {
        ensure!(
            catalog_path.is_none()
                && resources.is_empty()
                && clock.is_none()
                && stimulus.is_none()
                && vcd_path.is_none()
                && probes.is_empty()
                && initial.is_empty(),
            "load prepares a storage snapshot; decode/run options are not applied"
        );
        let architecture = architecture_path.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/inventory/LFE5U-85F")
        });
        let model = Arc::new(DeviceModel::load(&db_path, &architecture)?);
        let image = Arc::new(ConfigurationImage::load(
            &path,
            &db_path,
            image_reader.as_deref(),
        )?);
        let catalogue = load_catalogue(&model, &architecture)?;
        let prepared = ConfiguredDevice::bind(model, image, catalogue)?;
        let state = DeviceState::new(&prepared)?;
        let config = prepared.summary();
        let storage = state.summary();
        println!(
            "Device: {}; source: {:?}",
            prepared.model().database().device,
            prepared.image().source_kind()
        );
        println!(
            "CRAM: {}; EBR INIT images: {}",
            prepared.image().cram().fingerprint(),
            config.init_image_count
        );
        println!(
            "Composed resource fields: {}; states: {:?}",
            config.composed_fields, config.field_statuses
        );
        println!(
            "Live storage: {} LUT arrays, {} FFs, {} EBR arrays",
            storage.lut_arrays, storage.flip_flops, storage.ebr_arrays
        );
        println!(
            "Service registers: {:?}; parser defaults may apply: {}",
            prepared.image().service().native_registers,
            prepared.image().service().parser_defaults_may_apply
        );
        println!("Snapshot prepared; event execution and startup have not run.");
        if let Some(path) = output_path {
            let mut file = BufWriter::new(std::fs::File::create(path)?);
            serde_json::to_writer_pretty(
                &mut file,
                &serde_json::json!({"schema_version":1,
                "architecture_manifest_sha256":prepared.model().manifest_sha256(),
                "image":prepared.image().summary(),"configuration":config,"state":storage}),
            )?;
            writeln!(file)?;
            file.flush()?;
        }
        return Ok(());
    }
    ensure!(
        architecture_path.is_none() && image_reader.is_none(),
        "--architecture/--image-reader require load or device"
    );
    if command == "decode" {
        let catalog_path = catalog_path.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("docs/inventory/LFE5U-85F/decode_catalog.json")
        });
        let catalog = Catalog::load(&catalog_path)?;
        let cram = Cram::load(&path, &db_path, &catalog_path)?;
        let report = catalog.decode(&cram, &resources)?;
        println!(
            "Device: {}; CRAM: {}",
            report.device, report.cram_fingerprint
        );
        println!(
            "Decoded resources: {}; INIT images: {}",
            report.resources.len(),
            report.images.len()
        );
        println!("Field states: {:?}", report.field_status_counts);
        if !resources.is_empty() {
            for resource in &report.resources {
                println!("{} ({})", resource.id, resource.family);
                for field in &resource.fields {
                    println!(
                        "  {}: {} [{}]",
                        field.name,
                        field.value.as_deref().unwrap_or("?"),
                        field.status
                    );
                }
                if let Some(memory) = &resource.memory {
                    println!("  INIT: {} (WID {:?})", memory.status, memory.wid);
                }
            }
        }
        if let Some(path) = output_path {
            let mut file = BufWriter::new(std::fs::File::create(path)?);
            serde_json::to_writer_pretty(&mut file, &report)?;
            writeln!(file)?;
            file.flush()?;
        }
        return Ok(());
    }
    ensure!(
        catalog_path.is_none() && output_path.is_none() && resources.is_empty(),
        "--catalog/--resource/--output require decode"
    );
    let config = Config::load(&path, &db_path)?;
    let db = Database::load(&db_path, &config.device)?;
    let expanded = expand_groups(&db, &config)?;
    let report = coverage(&db, &expanded);
    if command == "inspect" {
        println!("Device: {}", config.device);
        println!("Database: {}", db_path.display());
        println!(
            "Physical tiles: {}; PLC2: {}; LUT4/FF capacity: {}",
            db.tiles.len(),
            report.logic_tiles,
            report.logic_tiles * 8
        );
        println!(
            "Configured tiles: {}; selected arcs: {}; BRAM initialisation blocks: {}",
            expanded.tiles.len(),
            expanded.tiles.values().map(|t| t.arcs.len()).sum::<usize>(),
            config.bram.len()
        );
        println!(
            "Unknown configuration bits retained: {}",
            report.unknown_bits
        );
        if report.unsupported.is_empty() {
            println!("Active configuration is within the implemented digital subset.");
        } else {
            println!("Unmodelled active features (simulation will refuse these):");
            for feature in report.unsupported {
                println!("  {feature}");
            }
        }
        return Ok(());
    }
    ensure!(
        clock.is_none() || stimulus.is_none(),
        "choose --clock or --stimulus"
    );
    let mut chip = Chip::compile(&db, &expanded, &package)?;
    eprintln!(
        "{}: {} routing wires, {} fixed connections, {} LUT/mux models, {} FFs",
        config.device,
        chip.routing.wire_count(),
        chip.routing.fixed_connections,
        chip.simulation.gate_count(),
        chip.simulation.register_count()
    );
    if probes.is_empty() {
        probes.extend(
            chip.ports
                .iter()
                .filter(|(_, p)| p.direction == "OUTPUT")
                .map(|(name, _)| format!("pin:{name}")),
        );
    }
    let observed = probes
        .iter()
        .map(|p| chip.probe_net(p))
        .collect::<Result<Vec<_>>>()?;
    if let Some(ref pin) = clock {
        ensure!(
            !initial.iter().any(|(name, _)| name == pin),
            "clock input is already set with --set"
        );
        initial.push((pin.clone(), Logic::Zero));
    }
    chip.drive_pins(&initial)?;
    let mut vcd = vcd_path
        .map(|path| -> Result<_> {
            let mut output = BufWriter::new(std::fs::File::create(path)?);
            writeln!(
                output,
                "$version ecp5_emu $end\n$timescale 1ns $end\n$scope module ecp5 $end"
            )?;
            for (i, name) in probes.iter().enumerate() {
                writeln!(output, "$var wire 1 s{i} \\{name} $end")?;
            }
            writeln!(output, "$upscope $end\n$enddefinitions $end")?;
            Ok(output)
        })
        .transpose()?;
    println!("time,{}", probes.join(","));
    let mut sample = |time: u64, chip: &Chip| -> Result<()> {
        let values: Vec<_> = observed.iter().map(|&n| chip.simulation.value(n)).collect();
        println!(
            "{time},{}",
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        if let Some(ref mut output) = vcd {
            writeln!(output, "#{time}")?;
            for (i, value) in values.iter().enumerate() {
                writeln!(output, "{value}s{i}")?;
            }
        }
        Ok(())
    };
    sample(0, &chip)?;
    if let Some(pin) = clock {
        ensure!(cycles <= (u64::MAX - 1) / 2, "cycle count too large");
        for cycle in 0..cycles {
            chip.drive_pins(&[(pin.clone(), Logic::One)])?;
            sample(cycle * 2 + 1, &chip)?;
            chip.drive_pins(&[(pin.clone(), Logic::Zero)])?;
            sample(cycle * 2 + 2, &chip)?;
        }
    } else if let Some(path) = stimulus {
        let events: Vec<Event> = serde_json::from_slice(&std::fs::read(path)?)?;
        let mut previous = None;
        for event in events {
            ensure!(previous.is_none_or(|p| event.time > p), "stimulus timestamps must strictly increase; combine simultaneous inputs in one set");
            let values = event
                .set
                .iter()
                .map(|(k, v)| Ok((k.clone(), v.parse::<Logic>()?)))
                .collect::<Result<Vec<_>>>()?;
            chip.drive_pins(&values)?;
            sample(event.time, &chip)?;
            previous = Some(event.time);
        }
    }
    if let Some(ref mut output) = vcd {
        output.flush()?;
    }
    Ok(())
}
