//! `ecp5_gui DESIGN.config [--names top_routed.json]`: the chip and the
//! ULX3S around it; `ecp5_gui` alone opens on a start screen to open a design
//! from (a file, the recent ones, or the demos built from examples/ulx3s,
//! src/start.rs); `ecp5_gui --empty` shows the die's whole fabric with no
//! design. The design runs in fast mode; the window shows the die's fabric
//! with the design's wires, LUTs and flip-flops live (`--grid` opens on the
//! grid, the tiles and sites without the wires; `--net clk_25mhz` shows that signal's net
//! whole, from where it starts to what it feeds), and left of it the ULX3S
//! in 3D with its LEDs lit and its buttons to press, over a UART console. A click on a site or a part of a slice opens
//! it in the inspector (src/inspector.rs): how it works, its pins with their
//! values, its settings and its insides. The routed JSON next to the configuration is
//! taken for HDL names when `--names` is not given; `--run` starts running
//! as soon as the design is loaded; `--focus R40C9 --cell 200` opens the map
//! on that tile at 200 pixels per tile (`--inside 0.8,0.4` centres the
//! empty fabric on that point of the tile, as shares of its width and
//! height); `--rate 2` runs two cycles per
//! second (`max` as fast as it can; the default is the board's 25 MHz).
//!
//! `ecp5_gui --board-picture FILE.png [--board-view 3d|top|bottom]
//! [--board-lit 10110000] [--board-pressed 0100000] [--size 1200x744]` draws
//! the window's 3D ULX3S without a window (src/board_view.rs): the LEDs
//! D0..D7 and buttons B0..B6 given as 0/1 from the first.
mod app;
mod board_model;
mod board_view;
mod changes;
mod fabric_view;
mod gpu_map;
mod inspector;
mod monitor_view;
mod new_project;
mod project;
mod start;
#[cfg(test)]
mod test_temp;
mod theme;
mod trace;
mod waves;
mod window;

use anyhow::{bail, Context as _, Result};
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str =
    "Usage: ecp5_gui --tour\n       ecp5_gui --project DIR [--out BUILD_DIR] [--run] [--rate CYCLES_PER_S|max] [--focus RxCy] [--cell PIXELS]\n       ecp5_gui [DESIGN.config] [--names top_routed.json] [--run] [--grid] [--net HDL_NAME] [--rate CYCLES_PER_S|max] [--focus RxCy] [--cell PIXELS]\n       ecp5_gui --empty [--focus RxCy] [--inside X,Y] [--cell PIXELS]\n       ecp5_gui --board-picture FILE.png [--board-view 3d|top|bottom] [--board-lit BITS] [--board-pressed BITS] [--size WxH]\n       any of them: [--architecture DIR] (the architecture catalogues; also $ECP5_ARCHITECTURE)";

fn window_icon() -> Arc<eframe::egui::IconData> {
    // Linux, Windows and cargo run use this; macOS .app Dock icons use the bundled .icns.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!(
        "../../assets/icon/hicolor/256x256/apps/ecp5_emu.png"
    ))
    .unwrap_or_else(|error| {
        eprintln!("ecp5_gui: could not decode window icon: {error}");
        // An empty IconData requests the OS default, without a custom icon.
        eframe::egui::IconData::default()
    });
    Arc::new(icon)
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut config = None;
    let mut project = None;
    let mut out = None;
    let mut names = None;
    let mut run = false;
    let mut tour = false;
    let mut focus = None;
    let mut cell = None;
    let mut rate = None;
    let mut empty = false;
    let mut grid = false;
    let mut net = None;
    let mut inside = (0.5, 0.5);
    let mut picture = None;
    let mut board_view = "3d".to_string();
    let mut lit = [false; 8];
    let mut pressed = [false; 7];
    let mut size = (1200u32, 744u32);
    let bits = |s: String| -> Vec<bool> { s.chars().map(|c| c == '1').collect() };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--board-picture" => {
                picture = Some(PathBuf::from(
                    args.next().context("--board-picture needs a file")?,
                ))
            }
            "--board-view" => {
                board_view = args
                    .next()
                    .context("--board-view needs 3d, top or bottom")?
            }
            "--board-lit" => {
                for (i, b) in bits(args.next().context("--board-lit needs 0/1 for D0..D7")?)
                    .into_iter()
                    .take(8)
                    .enumerate()
                {
                    lit[i] = b;
                }
            }
            "--board-pressed" => {
                for (i, b) in bits(
                    args.next()
                        .context("--board-pressed needs 0/1 for B0..B6")?,
                )
                .into_iter()
                .take(7)
                .enumerate()
                {
                    pressed[i] = b;
                }
            }
            "--size" => {
                let at = args.next().context("--size needs WxH")?;
                let (w, h) = at
                    .split_once('x')
                    .context("--size needs WxH, like 1200x744")?;
                size = (w.parse()?, h.parse()?);
            }
            "--project" => {
                project = Some(PathBuf::from(
                    args.next().context("--project needs a folder")?,
                ))
            }
            "--out" => out = Some(PathBuf::from(args.next().context("--out needs a folder")?)),
            "--names" => names = Some(PathBuf::from(args.next().context("--names needs a file")?)),
            // Where the architecture catalogues are (src/live.rs
            // `architecture`), as a packaged app has them; set before any
            // thread starts.
            "--architecture" => {
                let dir = args.next().context("--architecture needs a folder")?;
                std::env::set_var("ECP5_ARCHITECTURE", dir);
            }
            "--tour" => tour = true,
            "--run" => run = true,
            "--empty" => empty = true,
            "--grid" => grid = true,
            "--net" => net = Some(args.next().context("--net needs an HDL name")?),
            "--focus" => {
                let at = args.next().context("--focus needs RxCy")?;
                let (r, c) = at
                    .strip_prefix('R')
                    .and_then(|s| s.split_once('C'))
                    .context("--focus needs RxCy, like R40C9")?;
                focus = Some((r.parse::<u16>()?, c.parse::<u16>()?));
            }
            "--inside" => {
                let at = args.next().context("--inside needs X,Y")?;
                let (x, y) = at
                    .split_once(',')
                    .context("--inside needs X,Y, like 0.8,0.4")?;
                inside = (x.parse::<f32>()?, y.parse::<f32>()?);
            }
            "--cell" => cell = Some(args.next().context("--cell needs pixels")?.parse::<f32>()?),
            "--rate" => {
                let r = args
                    .next()
                    .context("--rate needs cycles per second or max")?;
                rate = Some(if r == "max" {
                    None
                } else {
                    Some(r.parse::<f64>()?)
                });
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ if config.is_none() => config = Some(PathBuf::from(arg)),
            other => bail!("unexpected argument {other}"),
        }
    }
    let tour_project = if tour {
        let found = project::tour::TourProject::find()?;
        project = Some(found.root.clone());
        out = None;
        run = false;
        Some(found)
    } else {
        None
    };
    if let Some(out) = picture {
        return board_view::picture(&out, &board_view, lit, pressed, size.0, size.1);
    }
    if empty {
        let options = eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1500.0, 950.0])
                .with_icon(window_icon())
                .with_title("ecp5_emu — LFE5U-85F fabric"),
            ..Default::default()
        };
        let options = gpu_room(options);
        return eframe::run_native(
            "ecp5_emu",
            options,
            Box::new(move |cc| {
                Ok(Box::new(fabric_view::FabricApp::new(
                    cc, focus, cell, inside,
                )))
            }),
        )
        .map_err(|e| anyhow::anyhow!("{e}"));
    }
    // A project (src/project.rs): its sources, build and report, and the
    // design built from it running in the same window; `--run` builds it
    // and opens the design at once.
    if let Some(dir) = project {
        let options = gpu_room(eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1500.0, 950.0])
                .with_icon(window_icon())
                .with_title("ecp5_emu — project"),
            ..Default::default()
        });
        return eframe::run_native(
            "ecp5_emu",
            options,
            Box::new(move |cc| {
                let mut window = project::ProjectApp::new(cc, dir, out, run, tour)?;
                if let Some(rate) = rate {
                    window.rate(rate);
                }
                let app = window.app();
                app.view(focus, cell);
                if let Some(path) = std::env::var_os("ECP5_GUI_TRACE") {
                    app.trace(std::path::Path::new(&path))?;
                }
                if !tour {
                    if let Some(path) = std::env::var_os("ECP5_GUI_SCRIPT") {
                        app.script(std::path::Path::new(&path))?;
                    }
                }
                if let Some(project) = tour_project {
                    window.start_tour(project)?;
                }
                Ok(Box::new(window::Window::project(window)))
            }),
        )
        .map_err(|e| anyhow::anyhow!("{e}"));
    }
    let design = config.map(|config| {
        let names = names.or_else(|| {
            let routed = config.with_file_name("top_routed.json");
            routed.exists().then_some(routed)
        });
        ecp5_emu::live::Design { config, names, run }
    });
    let title = design.as_ref().map_or_else(
        || "Open a design".to_string(),
        |d| app::design_title(&d.config),
    );
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 950.0])
            .with_icon(window_icon())
            .with_title(format!("ecp5_emu — {title}")),
        ..Default::default()
    };
    let options = gpu_room(options);
    eframe::run_native(
        "ecp5_emu",
        options,
        Box::new(move |cc| {
            let mut app = app::App::start(cc, design);
            app.view(focus, cell);
            if grid {
                app.grid();
            }
            if let Some(net) = net {
                app.show_net(net);
            }
            if let Some(rate) = rate {
                app.rate(rate);
            }
            // A record of what the map draws, and a script to drive the
            // window (src/trace.rs).
            if let Some(path) = std::env::var_os("ECP5_GUI_TRACE") {
                app.trace(std::path::Path::new(&path))?;
            }
            if let Some(path) = std::env::var_os("ECP5_GUI_SCRIPT") {
                app.script(std::path::Path::new(&path))?;
            }
            Ok(Box::new(window::Window::plain(app)?))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}

/// The GPU device asked for buffers as large as the GPU takes: the map reads
/// a big design's pieces of wire (5.8 million, 153 MB, on the full chip)
/// from one buffer, past wgpu's default 128 MB.
fn gpu_room(mut options: eframe::NativeOptions) -> eframe::NativeOptions {
    use eframe::egui_wgpu::WgpuSetup;
    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        let base = std::sync::Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = std::sync::Arc::new(move |adapter| {
            let mut descriptor = base(adapter);
            descriptor.required_features |=
                adapter.features() & eframe::wgpu::Features::TIMESTAMP_QUERY;
            let most = adapter.limits();
            let limits = &mut descriptor.required_limits;
            limits.max_storage_buffer_binding_size = limits
                .max_storage_buffer_binding_size
                .max(most.max_storage_buffer_binding_size);
            limits.max_buffer_size = limits.max_buffer_size.max(most.max_buffer_size);
            descriptor
        });
    }
    options
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_icon_decodes_embedded_rgba_png() {
        let decoded = image::load_from_memory(include_bytes!(
            "../../assets/icon/hicolor/256x256/apps/ecp5_emu.png"
        ))
        .expect("embedded window icon must decode");
        assert_eq!(decoded.color(), image::ColorType::Rgba8);
        let rgba = decoded.into_rgba8();
        assert_eq!(rgba.dimensions(), (256, 256));
        assert_eq!(rgba.as_raw().len(), 256 * 256 * 4);
        assert!(rgba.pixels().any(|pixel| pixel[3] != 0));

        let icon = super::window_icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert_eq!(&icon.rgba, rgba.as_raw());
    }
}
