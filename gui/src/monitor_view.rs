//! The board's peripherals as the window shows them: a monitor on the GPDI
//! (HDMI) connector and the OLED, from what the runner decodes from their
//! pads (ecp5_emu::peripherals, docs/PERIPHERALS.md). Each is a window of its
//! own. The OLED opens when first driven; the Monitor opens on its first
//! decoded DVI frame. Both can be opened from the Peripherals menu.
//!
//! The emulator does not run at the real pixel rate, so the monitor says how
//! fast frames come here against the rate they would come on the board.
//!
//! A record of the window (src/trace.rs) gets `# monitor` and `# oled` lines:
//! each picture the first time it is put on the screen, with its CRC and
//! where (points), and each change of signal. `monitor frame` is a frame
//! of the HDMI receiver: its number, size, the CRC-32 of its pixels (as the
//! design's own `F… C…` UART line has it), the refresh rate in emulated
//! time, and the rate here. `oled` is a new picture of the panel. `sd` is
//! a change of the SD card's state: the last command, the blocks read and
//! written, and the errors it answered. `sdram` is the SDRAM's counts,
//! at most once a second while they change: initialised or not, data beats
//! read and written, timing violations and refresh shortfalls. Audio notes
//! describe schematic-derived frequency and Vpp, throttled to one per second.
use ecp5_emu::{board, live::Command, peripherals::View};
use eframe::egui::{self, Color32, RichText};
use std::{collections::VecDeque, time::Instant};

/// What the record's `sdram` line says: initialised, data beats read and
/// written, timing violations, refresh shortfalls.
type SdramCounts = (bool, u64, u64, u64, u64);
type AudioReadings = [(Option<f64>, Option<f64>); 3];

/// A peripheral window. EDID is part of the HDMI Monitor, not a separate window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Monitor,
    Oled,
    Sd,
    Flash,
    Rtc,
    Audio,
    Adc,
    Sdram,
    Header,
}

impl Panel {
    pub const ALL: [Self; 9] = [
        Self::Monitor,
        Self::Oled,
        Self::Sd,
        Self::Flash,
        Self::Rtc,
        Self::Audio,
        Self::Adc,
        Self::Sdram,
        Self::Header,
    ];

    /// The name used by `panel NAME open|close` in window scripts.
    pub fn name(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Oled => "oled",
            Self::Sd => "sd",
            Self::Flash => "flash",
            Self::Rtc => "rtc",
            Self::Audio => "audio",
            Self::Adc => "adc",
            Self::Sdram => "sdram",
            Self::Header => "header",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.name() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Monitor => "Monitor (HDMI)",
            Self::Oled => "OLED (SSD1331)",
            Self::Sd => "SD card",
            Self::Flash => "SPI flash",
            Self::Rtc => "RTC (MCP7940N)",
            Self::Audio => "Audio (jack)",
            Self::Adc => "ADC (MAX11125)",
            Self::Sdram => "SDRAM",
            Self::Header => "Header J1/J2",
        }
    }
}

#[derive(Default)]
pub struct Monitor {
    hdmi_open: bool,
    oled_open: bool,
    /// Whether the model was seen, so its panel is available in the menu.
    hdmi_seen: bool,
    oled_seen: bool,
    /// Only a decoded frame triggers the Monitor's one automatic opening.
    hdmi_frame_seen: bool,
    hdmi_texture: Option<egui::TextureHandle>,
    /// The frame on the texture, and whether it was written to the record.
    hdmi_frame: Option<(u64, u32)>,
    hdmi_noted: Option<(u64, u32)>,
    /// The signal as last written to the record.
    signal_noted: Option<String>,
    edid_noted: Option<ecp5_emu::peripherals::edid::Status>,
    oled_texture: Option<egui::TextureHandle>,
    oled_version: Option<u64>,
    oled_noted: Option<u64>,
    audio_open: bool,
    audio_seen: bool,
    audio_noted: Option<(AudioReadings, Instant)>,
    rtc_open: bool,
    rtc_seen: bool,
    rtc_noted: Option<String>,
    flash_open: bool,
    flash_seen: bool,
    flash_noted: Option<u64>,
    flash_warning_noted: Option<u64>,
    sd_open: bool,
    sd_seen: bool,
    sd_noted: Option<u64>,
    adc_open: bool,
    adc_seen: bool,
    adc_noted: Option<([Option<u16>; 8], Instant)>,
    sdram_open: bool,
    sdram_seen: bool,
    /// The counts last written to the record, and when.
    sdram_noted: Option<(SdramCounts, Instant)>,
    /// Frames decoded, by when they reached the window, for the rate here.
    arrivals: VecDeque<(Instant, u64)>,
    notes: Vec<String>,
    header_open: bool,
    header_seen: bool,
    header_noted: Option<(String, Instant)>,
}

/// An image of `size` pixels as large as fits `room`, its aspect kept.
fn fit(room: egui::Vec2, size: [usize; 2]) -> egui::Vec2 {
    let (w, h) = (size[0] as f32, size[1] as f32);
    let scale = (room.x / w).min(room.y / h).max(0.05);
    egui::vec2(w * scale, h * scale)
}

impl Monitor {
    /// Menu order, availability (seen for this design), and current open state.
    pub fn panels(&self) -> Vec<(Panel, bool, bool)> {
        use Panel::*;
        vec![
            (Monitor, self.hdmi_seen, self.hdmi_open),
            (Oled, self.oled_seen, self.oled_open),
            (Sd, self.sd_seen, self.sd_open),
            (Flash, self.flash_seen, self.flash_open),
            (Rtc, self.rtc_seen, self.rtc_open),
            (Audio, self.audio_seen, self.audio_open),
            (Adc, self.adc_seen, self.adc_open),
            (Sdram, self.sdram_seen, self.sdram_open),
            (Header, self.header_seen, self.header_open),
        ]
    }

    /// Reopen or close a seen panel; leave an unavailable panel unchanged.
    pub fn set_open(&mut self, panel: Panel, open: bool) -> anyhow::Result<()> {
        let (seen, state) = match panel {
            Panel::Monitor => (self.hdmi_seen, &mut self.hdmi_open),
            Panel::Oled => (self.oled_seen, &mut self.oled_open),
            Panel::Sd => (self.sd_seen, &mut self.sd_open),
            Panel::Flash => (self.flash_seen, &mut self.flash_open),
            Panel::Rtc => (self.rtc_seen, &mut self.rtc_open),
            Panel::Audio => (self.audio_seen, &mut self.audio_open),
            Panel::Adc => (self.adc_seen, &mut self.adc_open),
            Panel::Sdram => (self.sdram_seen, &mut self.sdram_open),
            Panel::Header => (self.header_seen, &mut self.header_open),
        };
        anyhow::ensure!(
            seen,
            "{} panel is unavailable: its model has not been seen for this design",
            panel.label()
        );
        *state = open;
        Ok(())
    }

    /// The record's lines since the last call.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    pub fn show(&mut self, ctx: &egui::Context, view: &View) -> Vec<Command> {
        self.hdmi(ctx, view);
        self.oled(ctx, view);
        self.sd(ctx, view);
        self.flash(ctx, view);
        self.rtc(ctx, view);
        self.audio(ctx, view);
        self.sdram(ctx, view);
        let mut commands: Vec<_> = self
            .adc(ctx, view)
            .into_iter()
            .map(|(channel, volts)| Command::AdcVolts(channel, volts))
            .collect();
        commands.extend(self.header(ctx, view));
        commands
    }

    fn header(&mut self, ctx: &egui::Context, view: &View) -> Vec<Command> {
        let mut commands = Vec::new();
        let Some(status) = &view.header else {
            return commands;
        };
        if !self.header_seen {
            self.header_seen = true;
            self.header_open = true;
        }
        let note = status
            .pins
            .iter()
            .map(|p| {
                let hz = p.hz.map_or("--".into(), |v| format!("{v:.3}"));
                let duty = p.duty.map_or("--".into(), |v| format!("{:.3}", v * 100.0));
                let external = p.external.map_or("z", |b| if b { "1" } else { "0" });
                format!(
                    "{}={} {hz}Hz/{duty}% ext={external}",
                    board::HEADER[p.index].name,
                    ["0", "1", "X", "Z"][p.level.min(3) as usize]
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        if self
            .header_noted
            .as_ref()
            .is_none_or(|(old, at)| *old != note && at.elapsed().as_secs_f64() >= 1.0)
        {
            self.notes.push(format!("header {note}"));
            self.header_noted = Some((note, Instant::now()));
        }
        let mut open = self.header_open;
        egui::Window::new("Header J1/J2")
            .vscroll(true)
            .open(&mut open)
            .default_pos([200.0, 100.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    for connector in [1, 2] {
                        ui.vertical(|ui| {
                            ui.heading(format!("J{connector}"));
                            egui::Grid::new(("header connector", connector))
                                .num_columns(2)
                                .show(ui, |ui| {
                                    for row in 0..20 {
                                        for pad in [row * 2 + 1, row * 2 + 2] {
                                            if let Some((index, pin)) =
                                                board::HEADER.iter().enumerate().find(|(_, p)| {
                                                    p.connector == connector && p.pad == pad
                                                })
                                            {
                                                ui.vertical(|ui| {
                                                    ui.label(format!(
                                                        "{pad}: {} ({})",
                                                        pin.name, pin.ball
                                                    ));
                                                    if let Some(p) = status
                                                        .pins
                                                        .iter()
                                                        .find(|p| p.index == index)
                                                    {
                                                        ui.horizontal(|ui| {
                                                            ui.label(
                                                                ["0", "1", "X", "Z"]
                                                                    [p.level.min(3) as usize],
                                                            );
                                                            if p.direction.receives() {
                                                                for (label, level) in [
                                                                    ("0", Some(false)),
                                                                    ("1", Some(true)),
                                                                    ("released", None),
                                                                ] {
                                                                    if ui
                                                                        .selectable_label(
                                                                            p.external == level,
                                                                            label,
                                                                        )
                                                                        .clicked()
                                                                    {
                                                                        commands.push(
                                                                            Command::HeaderLevel(
                                                                                index, level,
                                                                            ),
                                                                        );
                                                                    }
                                                                }
                                                            }
                                                        });
                                                        if p.direction.drives() {
                                                            ui.label(format!(
                                                                "{} Hz / {}%",
                                                                p.hz.map_or(
                                                                    "--".into(),
                                                                    |n| format!("{n:.3}")
                                                                ),
                                                                p.duty.map_or(
                                                                    "--".into(),
                                                                    |n| format!("{:.2}", n * 100.0)
                                                                )
                                                            ));
                                                        }
                                                    } else {
                                                        ui.weak("unused");
                                                    }
                                                });
                                            } else {
                                                let name = if [3, 4, 21, 22, 37, 38].contains(&pad)
                                                {
                                                    "GND"
                                                } else if connector == 1 {
                                                    "2V5_3V3"
                                                } else if pad == 39 {
                                                    "IN5V"
                                                } else if pad == 40 {
                                                    "OUT5V"
                                                } else {
                                                    "+3V3"
                                                };
                                                ui.add_enabled(
                                                    false,
                                                    egui::Label::new(format!("{pad}: {name}")),
                                                );
                                            }
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    }
                });
            });
        self.header_open = open;
        commands
    }

    fn audio(&mut self, ctx: &egui::Context, view: &View) {
        let Some(s) = &view.audio else {
            return;
        };
        if !self.audio_seen {
            self.audio_seen = true;
            self.audio_open = true;
        }
        let readings = std::array::from_fn(|i| (s.channels[i].hz, s.channels[i].vpp));
        let changed = |old: &AudioReadings| {
            let notable = |a: Option<f64>, b: Option<f64>, floor: f64, relative: f64| match (a, b) {
                (Some(a), Some(b)) => (a - b).abs() >= floor.max(a.abs() * relative),
                (None, None) => false,
                _ => true,
            };
            old.iter()
                .zip(readings)
                .any(|(&(hz, vpp), (h, v))| notable(hz, h, 1.0, 0.01) || notable(vpp, v, 0.01, 0.0))
        };
        if self
            .audio_noted
            .as_ref()
            .is_none_or(|(was, at)| at.elapsed().as_secs_f64() >= 1.0 && changed(was))
        {
            self.audio_noted = Some((readings, Instant::now()));
            let n = |v: Option<f64>| v.map_or("--".into(), |v| format!("{v:.3}"));
            self.notes.push(format!("audio modelled open-circuit l_hz={} l_vpp={} r_hz={} r_vpp={} v_hz={} v_vpp={} at_ps={}",
                n(readings[0].0), n(readings[0].1), n(readings[1].0), n(readings[1].1),
                n(readings[2].0), n(readings[2].1), s.time_ps));
        }
        let mut open = self.audio_open;
        egui::Window::new("Audio (jack)")
            .open(&mut open)
            .default_pos([620.0, 200.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label("Modelled from the PCB, not measured. Open circuit (jack unplugged).");
                for (i, name) in ["L", "R", "V"].into_iter().enumerate() {
                    let c = &s.channels[i];
                    if !c.attached {
                        ui.label(format!("{name}: no driven DAC bits"));
                        continue;
                    }
                    let hz = c.hz.map_or("--".into(), |v| format!("{v:.2}"));
                    let vpp = c.vpp.map_or("--".into(), |v| format!("{v:.3}"));
                    let volts = c.volts.map_or("--".into(), |v| format!("{v:.3}"));
                    ui.label(format!(
                        "{name}: {hz} Hz   {vpp} Vpp (last 1 s)   {}   {volts} V",
                        c.code_text()
                    ));
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().max(100.0), 86.0),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter_at(rect);
                    let color = [
                        Color32::LIGHT_GREEN,
                        Color32::LIGHT_BLUE,
                        Color32::LIGHT_RED,
                    ][i];
                    let graph = rect.shrink2(egui::vec2(3.0, 12.0));
                    for (fraction, text) in [(0.0, "3.3 V"), (1.0, "0 V")] {
                        let y = graph.top() + fraction * graph.height();
                        painter.line_segment(
                            [egui::pos2(graph.left(), y), egui::pos2(graph.right(), y)],
                            egui::Stroke::new(0.5, ui.visuals().weak_text_color()),
                        );
                        painter.text(
                            egui::pos2(graph.left(), y),
                            egui::Align2::LEFT_BOTTOM,
                            text,
                            egui::FontId::monospace(10.0),
                            ui.visuals().weak_text_color(),
                        );
                    }
                    let start = s.time_ps.saturating_sub(50_000_000_000);
                    let mut points: Vec<egui::Pos2> = Vec::new();
                    let flush = |points: &mut Vec<egui::Pos2>| {
                        if points.len() >= 2 {
                            painter.add(egui::Shape::line(
                                std::mem::take(points),
                                egui::Stroke::new(1.2, color),
                            ));
                        } else {
                            points.clear();
                        }
                    };
                    for sample in s.scope.iter().filter(|p| p.ps >= start) {
                        if let Some(volts) = sample.volts[i] {
                            let x = graph.right()
                                - ((s.time_ps - sample.ps) as f64 / 50_000_000_000.0) as f32
                                    * graph.width();
                            let y = graph.bottom() - volts / 3.3 * graph.height();
                            if let Some(p) = points.last().copied() {
                                points.push(egui::pos2(x, p.y));
                            }
                            points.push(egui::pos2(x, y));
                        } else {
                            flush(&mut points);
                        }
                    }
                    flush(&mut points);
                    painter.text(
                        rect.left_bottom(),
                        egui::Align2::LEFT_BOTTOM,
                        "-50 ms",
                        egui::FontId::monospace(10.0),
                        ui.visuals().weak_text_color(),
                    );
                    painter.text(
                        rect.right_bottom(),
                        egui::Align2::RIGHT_BOTTOM,
                        "now (emulated time)",
                        egui::FontId::monospace(10.0),
                        ui.visuals().weak_text_color(),
                    );
                }
            });
        self.audio_open = open;
    }

    fn rtc(&mut self, ctx: &egui::Context, view: &View) {
        let Some(s) = &view.rtc else {
            return;
        };
        if !self.rtc_seen {
            self.rtc_seen = true;
            self.rtc_open = true;
        }
        let [year, month, date, hour, minute, second] = s.time;
        let time = format!("{year:04}-{month:02}-{date:02} {hour:02}:{minute:02}:{second:02}");
        let access = format!(
            "bus {} {} register {} read {} written {} NACKs {}",
            s.last_bus_address
                .map_or("--".into(), |a| format!("{a:02X}")),
            if s.last_read { "read" } else { "write" },
            s.last_address.map_or("--".into(), |a| format!("{a:02X}")),
            s.bytes_read,
            s.bytes_written,
            s.nacks
        );
        let line = format!(
            "rtc {time} ST {} OSCRUN {} {access}",
            u8::from(s.st),
            u8::from(s.oscrun)
        );
        if self.rtc_noted.as_ref() != Some(&line) {
            self.notes.push(line.clone());
            self.rtc_noted = Some(line);
        }
        let mut open = self.rtc_open;
        egui::Window::new("RTC (MCP7940N)")
            .open(&mut open)
            .default_pos([620.0, 580.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(time);
                ui.label(format!(
                    "ST {}   OSCRUN {} (emulated time)",
                    u8::from(s.st),
                    u8::from(s.oscrun)
                ));
                ui.label(access);
            });
        self.rtc_open = open;
    }

    fn adc(&mut self, ctx: &egui::Context, view: &View) -> Vec<(usize, f64)> {
        let mut changes = Vec::new();
        let Some(s) = &view.adc else {
            return changes;
        };
        if !self.adc_seen {
            self.adc_seen = true;
            self.adc_open = true;
        }
        if self
            .adc_noted
            .is_none_or(|(codes, at)| codes != s.codes && at.elapsed().as_secs_f64() >= 1.0)
        {
            self.adc_noted = Some((s.codes, Instant::now()));
            let codes = s
                .codes
                .iter()
                .map(|c| c.map_or("----".into(), |c| format!("{c:04X}")))
                .collect::<Vec<_>>()
                .join(" ");
            self.notes.push(format!(
                "adc {codes} mode {} frames {}",
                s.mode_name(),
                s.frames
            ));
        }
        let mut open = self.adc_open;
        egui::Window::new("ADC (MAX11125)")
            .open(&mut open)
            .default_pos([620.0, 420.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{}; {} frames; VREF 3.3 V",
                    s.mode_name(),
                    s.frames
                ));
                egui::Grid::new("adc channels")
                    .num_columns(3)
                    .show(ui, |ui| {
                        for channel in 0..8 {
                            ui.label(format!("AIN{channel}"));
                            if s.driven[channel] {
                                ui.label(format!("{:.3} V (FPGA)", s.volts[channel]));
                            } else {
                                let mut volts = s.volts[channel];
                                if ui
                                    .add(egui::Slider::new(&mut volts, 0.0..=3.3).suffix(" V"))
                                    .changed()
                                {
                                    changes.push((channel, volts));
                                }
                            }
                            ui.label(
                                s.codes[channel]
                                    .map_or("----".into(), |code| format!("{code:04X} ({code})")),
                            );
                            ui.end_row();
                        }
                    });
            });
        self.adc_open = open;
        changes
    }

    fn sdram(&mut self, ctx: &egui::Context, view: &View) {
        let Some(s) = &view.sdram else {
            return;
        };
        if !self.sdram_seen {
            self.sdram_seen = true;
            self.sdram_open = true;
        }
        let counts = (
            s.initialised,
            s.reads,
            s.writes,
            s.timing_violations,
            s.refresh_shortfalls,
        );
        let due = self
            .sdram_noted
            .is_none_or(|(was, at)| was != counts && at.elapsed().as_secs_f64() >= 1.0);
        if due {
            self.sdram_noted = Some((counts, Instant::now()));
            self.notes.push(format!(
                "sdram init {} reads {} writes {} violations {} refresh shortfalls {}",
                if s.initialised { "yes" } else { "no" },
                s.reads,
                s.writes,
                s.timing_violations,
                s.refresh_shortfalls
            ));
        }
        let mut open = self.sdram_open;
        egui::Window::new("SDRAM (48LC16M16A2)")
            .open(&mut open)
            .default_pos([1060.0, 420.0])
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("sdram").num_columns(2).show(ui, |ui| {
                    ui.label("initialised");
                    ui.label(if s.initialised {
                        "yes"
                    } else {
                        "no (power-up sequence not seen)"
                    });
                    ui.end_row();
                    ui.label("mode");
                    ui.label(format!(
                        "CL {}, burst {}{}",
                        s.mode.cas_latency,
                        s.mode.burst_length,
                        if s.mode.interleaved {
                            " interleaved"
                        } else {
                            ""
                        }
                    ));
                    ui.end_row();
                    ui.label("data beats read / written");
                    ui.label(format!("{} / {}", s.reads, s.writes));
                    ui.end_row();
                    ui.label("open rows");
                    ui.label(
                        s.open_rows
                            .iter()
                            .map(|r| r.map_or("-".to_string(), |r| r.to_string()))
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                    ui.end_row();
                    ui.label("bytes touched");
                    ui.label(s.bytes_touched.to_string());
                    ui.end_row();
                    ui.label("timing violations");
                    ui.label(if s.timing_violations == 0 {
                        RichText::new("0")
                    } else {
                        RichText::new(s.timing_violations.to_string()).color(Color32::YELLOW)
                    });
                    ui.end_row();
                    ui.label("refresh shortfalls");
                    ui.label(s.refresh_shortfalls.to_string());
                    ui.end_row();
                });
                for m in s.messages.iter().take(6) {
                    ui.label(RichText::new(m).small().color(Color32::YELLOW));
                }
            });
        self.sdram_open = open;
    }

    fn flash(&mut self, ctx: &egui::Context, view: &View) {
        let Some(s) = &view.flash else {
            return;
        };
        if !self.flash_seen {
            self.flash_seen = true;
            self.flash_open = true;
        }
        let command = s.last_command.map_or("none".into(), |c| format!("{c:02X}"));
        let address = s.last_address.map_or("-".into(), |a| format!("{a:06X}"));
        let id = s
            .jedec
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>();
        if self.flash_noted != Some(s.version) {
            self.flash_noted = Some(s.version);
            self.notes.push(format!("flash JEDEC {id} last {command} address {address} read {} write command ignored {} programs {} erases {} protected {} busy ignored {} config writes {}",
                s.bytes_read, s.ignored_writes, s.programs, s.erases, s.protected, s.busy_ignored, s.config_writes));
        }
        if self.flash_warning_noted != Some(s.config_writes) {
            self.flash_warning_noted = Some(s.config_writes);
            if let Some(warning) = &s.first_config_write {
                self.notes.push(format!(
                    "flash warning: {warning}; first event, config_writes={}",
                    s.config_writes
                ));
            }
        }
        let mut open = self.flash_open;
        egui::Window::new("SPI flash")
            .open(&mut open)
            .default_pos([1060.0, 820.0])
            .default_width(440.0)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("spi flash").num_columns(2).show(ui, |ui| {
                    ui.label("JEDEC");
                    ui.label(&id);
                    ui.end_row();
                    ui.label("last command / address");
                    ui.label(format!("{command} / {address}"));
                    ui.end_row();
                    ui.label("bytes read");
                    ui.label(s.bytes_read.to_string());
                    ui.end_row();
                    ui.label("rejected write commands");
                    ui.label(s.ignored_writes.to_string());
                    ui.end_row();
                    ui.label("programs / erases");
                    ui.label(format!("{} / {}", s.programs, s.erases));
                    ui.end_row();
                    ui.label("protected / busy ignored");
                    ui.label(format!("{} / {}", s.protected, s.busy_ignored));
                    ui.end_row();
                });
                if let Some(warning) = &s.first_config_write {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "WARNING ({} attempts; first event): {warning}",
                                s.config_writes
                            ))
                            .color(Color32::YELLOW),
                        )
                        .wrap(),
                    );
                }
            });
        self.flash_open = open;
    }

    fn sd(&mut self, ctx: &egui::Context, view: &View) {
        let Some(sd) = &view.sd else {
            return;
        };
        if !self.sd_seen {
            self.sd_seen = true;
            self.sd_open = true;
        }
        let command = sd.last_command.map_or("none yet".to_string(), |c| {
            format!("CMD{c} {:08X}", sd.last_argument)
        });
        if self.sd_noted != Some(sd.version) {
            self.sd_noted = Some(sd.version);
            self.notes.push(format!(
                "sd {:?} last {command} read {} written {} crc errors {} illegal {} mode {} card_state {} rca {:04X} width {} commands {} responses {} dat3_pullup {}",
                sd.state, sd.blocks_read, sd.blocks_written, sd.crc_errors,
                sd.illegal_commands, sd.mode, sd.native_state.map_or("SPI", |s| s.name()),
                sd.rca, sd.bus_width, sd.commands, sd.responses, u8::from(sd.dat3_pullup)
            ));
        }
        let mut open = self.sd_open;
        egui::Window::new("SD card")
            .open(&mut open)
            .default_pos([1060.0, 660.0])
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("sd card").num_columns(2).show(ui, |ui| {
                    ui.label("state");
                    ui.label(format!("{:?}", sd.state));
                    ui.end_row();
                    ui.label("mode / card state");
                    ui.label(format!(
                        "{} / {}",
                        sd.mode,
                        sd.native_state.map_or("SPI", |s| s.name())
                    ));
                    ui.end_row();
                    ui.label("RCA / bus width");
                    ui.label(format!("{:04X} / {}", sd.rca, sd.bus_width));
                    ui.end_row();
                    ui.label("commands / responses");
                    ui.label(format!("{} / {}", sd.commands, sd.responses));
                    ui.end_row();
                    ui.label("internal DAT3 pull-up");
                    ui.label(if sd.dat3_pullup {
                        "connected"
                    } else {
                        "disconnected"
                    });
                    ui.end_row();
                    ui.label("last command");
                    ui.label(&command);
                    ui.end_row();
                    ui.label("blocks read / written");
                    ui.label(format!("{} / {}", sd.blocks_read, sd.blocks_written));
                    ui.end_row();
                    ui.label("CRC errors / illegal commands");
                    ui.label(format!("{} / {}", sd.crc_errors, sd.illegal_commands));
                    ui.end_row();
                });
                ui.label(
                    RichText::new("image: ECP5_SD_IMAGE, else a demo card with a boot sector")
                        .small()
                        .weak(),
                );
            });
        self.sd_open = open;
    }

    fn hdmi(&mut self, ctx: &egui::Context, view: &View) {
        if let Some(edid) = &view.edid {
            if self.edid_noted.as_ref() != Some(edid) {
                self.notes.push(format!("edid present={} reads={} bytes_read={} last_pointer={:02X} nacks={} ignored_writes={}",
                    if edid.present { "yes" } else { "no" }, edid.reads, edid.bytes_read,
                    edid.last_pointer, edid.nacks, edid.ignored_writes));
                self.edid_noted = Some(edid.clone());
            }
        }
        let Some(video) = &view.video else {
            return;
        };
        self.hdmi_seen = true;
        if !self.hdmi_frame_seen && video.frames_decoded > 0 {
            self.hdmi_frame_seen = true;
            self.hdmi_open = true;
        }
        // Frames a second here, over the last two seconds.
        let now = Instant::now();
        if self
            .arrivals
            .back()
            .is_none_or(|a| a.1 != video.frames_decoded)
        {
            self.arrivals.push_back((now, video.frames_decoded));
        }
        while self.arrivals.len() > 2 && now.duration_since(self.arrivals[0].0).as_secs_f64() > 2.0
        {
            self.arrivals.pop_front();
        }
        let here = match (self.arrivals.front(), self.arrivals.back()) {
            (Some(a), Some(b)) if b.1 > a.1 => {
                Some((b.1 - a.1) as f64 / now.duration_since(a.0).as_secs_f64().max(1e-3))
            }
            _ => None,
        };
        let no_frame = (video.frames_decoded == 0).then(|| {
            format!(
                "TMDS symbol lock: {}; invalid symbols: {}; pixel clock {:.3} MHz{}",
                if video.locked {
                    "all 3 data lanes"
                } else {
                    "not acquired"
                },
                video.invalid_symbols,
                video.pixel_clock_hz / 1e6,
                if video.note.is_empty() {
                    String::new()
                } else {
                    format!("; {}", video.note)
                },
            )
        });
        let signal = if let Some(details) = &no_frame {
            format!("GPDI lanes active, no DVI frame yet; {details}")
        } else if video.locked {
            match &video.timing {
                Some(t) => format!("{}x{} {:.2} Hz", t.width, t.height, t.refresh_hz),
                None => "locked".to_string(),
            }
        } else {
            format!("no signal: {}", video.note)
        };
        if self.signal_noted.as_ref() != Some(&signal) {
            self.notes.push(format!("monitor signal {signal}"));
            self.signal_noted = Some(signal);
        }
        if let Some(frame) = &video.last_frame {
            let key = (frame.number, frame.crc32);
            if self.hdmi_frame != Some(key) {
                let image = egui::ColorImage::from_rgb([frame.width, frame.height], &frame.rgb);
                match &mut self.hdmi_texture {
                    Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.hdmi_texture = Some(ctx.load_texture(
                            "hdmi frame",
                            image,
                            egui::TextureOptions::LINEAR,
                        ))
                    }
                }
                self.hdmi_frame = Some(key);
            }
        }
        let mut open = self.hdmi_open;
        // Beside the board panel (600 points wide at the start), over the die.
        egui::Window::new("Monitor (HDMI)")
            .open(&mut open)
            .default_pos([620.0, 80.0])
            .default_size([660.0, 560.0])
            .show(ctx, |ui| {
                match (&no_frame, &video.timing, video.locked) {
                    (Some(details), _, _) => {
                        ui.label("GPDI lanes active, no DVI frame yet");
                        ui.label(details);
                    }
                    (None, Some(t), true) => {
                        ui.label(format!(
                            "{}×{} at {:.2} Hz, pixel clock {:.3} MHz, syncs {}{}/{}{} (emulated time)",
                            t.width,
                            t.height,
                            t.refresh_hz,
                            t.pixel_clock_hz / 1e6,
                            if t.hsync_positive { "+" } else { "−" },
                            t.hsync_width,
                            if t.vsync_positive { "+" } else { "−" },
                            t.vsync_width,
                        ));
                    }
                    (None, _, true) => {
                        ui.label(format!(
                            "locked, pixel clock {:.3} MHz; waiting for a whole frame",
                            video.pixel_clock_hz / 1e6
                        ));
                    }
                    _ => {
                        ui.label(RichText::new(format!("No signal: {}", video.note)).color(Color32::YELLOW));
                    }
                }
                if let Some(edid) = &view.edid {
                    ui.label(if edid.present { format!("EDID: present, {} bytes read", edid.bytes_read) }
                        else { "EDID: disconnected".into() });
                }
                let refresh = video.timing.as_ref().map(|t| t.refresh_hz);
                ui.label(match (here, refresh) {
                    (Some(fps), Some(hz)) => format!(
                        "{} frames received; here {fps:.2} frames/s, {:.1}% of the board's rate",
                        video.frames_decoded,
                        100.0 * fps / hz
                    ),
                    _ => format!("{} frames received", video.frames_decoded),
                });
                let room = ui.available_size() - egui::vec2(0.0, 4.0);
                match (&self.hdmi_texture, &video.last_frame) {
                    (Some(texture), Some(frame)) => {
                        let size = fit(room, [frame.width, frame.height]);
                        let response = ui.add(egui::Image::new((texture.id(), size)));
                        let rect = response.rect;
                        if self.hdmi_noted != self.hdmi_frame && ui.is_rect_visible(rect) {
                            self.hdmi_noted = self.hdmi_frame;
                            self.notes.push(format!(
                                "monitor frame {} {}x{} crc {:08X} refresh {:.2} here {} at {:.0},{:.0} {:.0}x{:.0}",
                                frame.number,
                                frame.width,
                                frame.height,
                                frame.crc32,
                                frame.timing.refresh_hz,
                                here.map_or("-".into(), |f| format!("{f:.2}")),
                                rect.min.x,
                                rect.min.y,
                                rect.width(),
                                rect.height()
                            ));
                        }
                        response.on_hover_text(format!(
                            "frame {}: CRC-32 {:08X} of its pixels (R, G, B in raster order)",
                            frame.number, frame.crc32
                        ));
                    }
                    _ => {
                        let size = fit(room, [640, 480]);
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                    }
                }
            });
        self.hdmi_open = open;
    }

    fn oled(&mut self, ctx: &egui::Context, view: &View) {
        let (Some(oled), Some((width, height, rgba, serial))) = (&view.oled, view.oled_screen())
        else {
            return;
        };
        if !self.oled_seen {
            self.oled_seen = true;
            self.oled_open = true;
        }
        if self.oled_version != Some(serial) {
            let image =
                egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba);
            match &mut self.oled_texture {
                Some(t) => t.set(image, egui::TextureOptions::NEAREST),
                None => {
                    self.oled_texture =
                        Some(ctx.load_texture("oled", image, egui::TextureOptions::NEAREST))
                }
            }
            self.oled_version = Some(serial);
        }
        let mut open = self.oled_open;
        egui::Window::new("OLED (SSD1331)")
            .open(&mut open)
            .default_pos([620.0, 660.0])
            .default_size([420.0, 330.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{}, {} frames, last CRC {:08X}; {} commands, {} data bytes{}",
                    oled.note,
                    oled.frames,
                    oled.crc32,
                    oled.commands,
                    oled.data_bytes,
                    if oled.errors > 0 {
                        format!(", {} errors", oled.errors)
                    } else {
                        String::new()
                    }
                ));
                if let Some(texture) = &self.oled_texture {
                    let size = fit(ui.available_size() - egui::vec2(0.0, 4.0), [width as usize, height as usize]);
                    let response = ui.add(egui::Image::new((texture.id(), size)));
                    let rect = response.rect;
                    if self.oled_noted != self.oled_version && ui.is_rect_visible(rect) {
                        self.oled_noted = self.oled_version;
                        // The CRC of the picture shown, not of the bytes sent.
                        let shown = ecp5_emu::peripherals::video::crc32(
                            &rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect::<Vec<u8>>(),
                        );
                        self.notes.push(format!(
                            "oled picture {serial} {} frames {} data crc {:08X} picture crc {shown:08X} at {:.0},{:.0} {:.0}x{:.0}",
                            if oled.display_on { "on" } else { "off" },
                            oled.frames,
                            oled.crc32,
                            rect.min.x,
                            rect.min.y,
                            rect.width(),
                            rect.height()
                        ));
                    }
                }
            });
        self.oled_open = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecp5_emu::peripherals::adc::Adc;
    use std::time::Duration;

    fn decoded_frame(number: u64) -> ecp5_emu::peripherals::video::VideoStatus {
        use ecp5_emu::peripherals::video::{crc32, Frame, Timing, VideoStatus};
        let timing = Timing {
            width: 640,
            height: 480,
            htotal: 800,
            vtotal: 525,
            pixel_clock_hz: 25e6,
            refresh_hz: 25e6 / (800.0 * 525.0),
            ..Timing::default()
        };
        let rgb = vec![0; 640 * 480 * 3];
        VideoStatus {
            locked: true,
            pixel_clock_hz: timing.pixel_clock_hz,
            timing: Some(timing.clone()),
            frames_decoded: number,
            last_frame: Some(std::sync::Arc::new(Frame {
                number,
                width: 640,
                height: 480,
                crc32: crc32(&rgb),
                rgb,
                timing,
                end_ps: 0,
            })),
            ..VideoStatus::default()
        }
    }

    #[test]
    fn control_only_gpdi_stays_closed_but_manual_open_explains_the_signal() {
        use ecp5_emu::peripherals::video::VideoStatus;
        // ddr_pattern repeats control 00 on all three data lanes at a
        // 5 MHz pixel clock: symbol lock, zero invalid symbols, no frames.
        let mut view = View {
            video: Some(VideoStatus {
                locked: true,
                pixel_clock_hz: 5e6,
                ..VideoStatus::default()
            }),
            ..View::default()
        };
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let paint = |monitor: &mut Monitor, view: &View| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1600.0, 1200.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    monitor.show(ui.ctx(), view);
                },
            )
        };
        paint(&mut monitor, &view);
        assert_eq!(monitor.panels()[0], (Panel::Monitor, true, false));
        let details =
            "TMDS symbol lock: all 3 data lanes; invalid symbols: 0; pixel clock 5.000 MHz";
        assert_eq!(
            monitor.take_notes(),
            [format!(
                "monitor signal GPDI lanes active, no DVI frame yet; {details}"
            )]
        );
        // This is the API used by both the menu and `panel monitor open`.
        monitor.set_open(Panel::Monitor, true).unwrap();
        paint(&mut monitor, &view);
        let out = paint(&mut monitor, &view);
        let text = |out: &egui::FullOutput| {
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let shown = text(&out);
        assert!(
            shown
                .iter()
                .any(|t| t == "GPDI lanes active, no DVI frame yet"),
            "{shown:?}"
        );
        assert!(shown.iter().any(|t| t == details), "{shown:?}");
        assert!(
            monitor.take_notes().is_empty(),
            "unchanged signal is not recorded twice"
        );
        let status = view.video.as_mut().unwrap();
        status.locked = false;
        status.invalid_symbols = 7;
        status.note = "searching for control symbols".into();
        let shown = text(&paint(&mut monitor, &view));
        assert!(shown.iter().any(|t| t
            .contains("TMDS symbol lock: not acquired; invalid symbols: 7")
            && t.contains("searching for control symbols")));
        assert!(
            monitor.take_notes()[0].contains("no DVI frame yet; TMDS symbol lock: not acquired")
        );
    }

    #[test]
    fn first_decoded_dvi_frame_auto_opens_once_even_after_rewind() {
        use ecp5_emu::peripherals::video::VideoStatus;
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut view = View {
            video: Some(VideoStatus::default()),
            ..View::default()
        };
        let paint = |monitor: &mut Monitor, view: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                monitor.show(ui.ctx(), view);
            });
        };
        paint(&mut monitor, &view);
        assert!(!monitor.hdmi_open);
        view.video = Some(decoded_frame(1));
        paint(&mut monitor, &view);
        assert!(monitor.hdmi_frame_seen && monitor.hdmi_open);
        assert!(monitor
            .take_notes()
            .iter()
            .any(|n| n == "monitor signal 640x480 59.52 Hz"));
        monitor.set_open(Panel::Monitor, false).unwrap();
        for number in [1, 2, 0, 1] {
            view.video = Some(if number == 0 {
                VideoStatus::default()
            } else {
                decoded_frame(number)
            });
            paint(&mut monitor, &view);
            assert_eq!(monitor.panels()[0], (Panel::Monitor, true, false));
        }
        monitor.set_open(Panel::Monitor, true).unwrap();
        paint(&mut monitor, &view);
        assert!(monitor.hdmi_open);
    }

    #[test]
    fn all_seen_panels_can_close_and_reopen_without_auto_opening_again() {
        use ecp5_emu::peripherals::{
            audio::Audio, header::Header, oled, rtc::Rtc, sdcard::SdCard, sdram::Sdram,
            spiflash::SpiFlash,
        };
        let mut oled = oled::Oled::new();
        let view = View {
            video: Some(decoded_frame(1)),
            oled: Some(oled.status()),
            oled_rgba: Some((vec![0; oled::WIDTH * oled::HEIGHT * 4].into(), 0)),
            sd: Some(SdCard::demo().status()),
            flash: Some(SpiFlash::new(Vec::new()).status()),
            rtc: Some(Rtc::default().status()),
            audio: Some(Audio::new([true; 3]).status()),
            adc: Some(Adc::new().status()),
            sdram: Some(Sdram::new().status()),
            header: Header::new(&[("C11".into(), board::Direction::Input)].into())
                .map(|h| h.status()),
            ..View::default()
        };
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let paint = |monitor: &mut Monitor| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                monitor.show(ui.ctx(), &view);
            });
        };
        paint(&mut monitor);
        assert_eq!(
            monitor.panels(),
            Panel::ALL.map(|p| (p, true, true)),
            "all panels open once their automatic opening condition is met"
        );
        for panel in Panel::ALL {
            monitor.set_open(panel, false).unwrap();
            paint(&mut monitor);
            assert_eq!(
                monitor.panels(),
                Panel::ALL.map(|p| (p, true, p != panel)),
                "closed panel must stay closed: {panel:?}"
            );
            monitor.set_open(panel, true).unwrap();
            paint(&mut monitor);
            assert!(monitor
                .panels()
                .iter()
                .all(|&(_, available, open)| available && open));
        }
        // A model seen earlier in this design stays available while absent from a view.
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            monitor.show(ui.ctx(), &View::default());
        });
        assert!(monitor.panels().iter().all(|&(_, available, _)| available));
    }

    #[test]
    fn unavailable_panels_are_noops_and_a_new_design_starts_unavailable() {
        let mut monitor = Monitor::default();
        let initial = monitor.panels();
        assert_eq!(initial, Panel::ALL.map(|p| (p, false, false)));
        for panel in Panel::ALL {
            for open in [false, true] {
                let error = monitor.set_open(panel, open).unwrap_err().to_string();
                assert!(error.contains(panel.label()) && error.contains("unavailable"));
                assert_eq!(monitor.panels(), initial);
            }
        }
    }

    #[test]
    fn header_records_levels_and_throttles_changes() {
        let mut h = ecp5_emu::peripherals::header::Header::new(
            &[("C11".into(), board::Direction::Input)].into(),
        )
        .unwrap();
        h.feed(1_000, [1].into_iter());
        let mut view = View {
            header: Some(h.status()),
            ..View::default()
        };
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let paint = |monitor: &mut Monitor, view: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(monitor.show(ui.ctx(), view).is_empty());
            });
        };
        paint(&mut monitor, &view);
        assert!(monitor.take_notes()[0].starts_with("header GN0=1"));
        h.set_level(0, Some(false));
        h.feed(2_000, [0].into_iter());
        view.header = Some(h.status());
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().is_empty());
        monitor.header_noted.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        paint(&mut monitor, &view);
        assert!(monitor.take_notes()[0].contains("ext=0"));
    }
    #[test]
    fn edid_records_presence_and_counts_without_identifying_bytes() {
        use ecp5_emu::peripherals::{edid::Edid, video::VideoDecoder};
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut view = View {
            video: Some(VideoDecoder::new().status()),
            edid: Some(Edid::default().status()),
            ..View::default()
        };
        let paint = |m: &mut Monitor, v: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                m.show(ui.ctx(), v);
            });
        };
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().iter().any(|n| n
            == "edid present=yes reads=0 bytes_read=0 last_pointer=00 nacks=0 ignored_writes=0"));
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().is_empty());
        let e = view.edid.as_mut().unwrap();
        e.reads = 1;
        e.bytes_read = 128;
        e.last_pointer = 127;
        e.nacks = 1;
        paint(&mut monitor, &view);
        assert_eq!(
            monitor.take_notes(),
            ["edid present=yes reads=1 bytes_read=128 last_pointer=7F nacks=1 ignored_writes=0"]
        );
        view.edid.as_mut().unwrap().present = false;
        paint(&mut monitor, &view);
        assert!(monitor.take_notes()[0].starts_with("edid present=no"));
    }

    #[test]
    fn flash_configuration_warning_is_recorded_and_survives_unchanged_paints() {
        use ecp5_emu::peripherals::spiflash::{ConfigWrite, SpiFlash};
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut status = SpiFlash::new(Vec::new()).status();
        status.config_writes = 1;
        status.first_config_write = Some(std::sync::Arc::new(ConfigWrite {
            command: 0x02,
            start: 0,
            end: 1,
        }));
        status.version = 1;
        let view = View {
            flash: Some(status),
            ..View::default()
        };
        let paint = |m: &mut Monitor| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                m.show(ui.ctx(), &view);
            });
        };
        paint(&mut monitor);
        assert!(monitor.flash_open && monitor.flash_seen);
        let notes = monitor.take_notes();
        assert!(notes.iter().any(|s| s.starts_with("flash warning: command 02 range [000000,000001): this would overwrite the board's FPGA configuration (bitstream) on a real ULX3S")));
        paint(&mut monitor);
        assert!(monitor.take_notes().is_empty());
    }

    #[test]
    fn audio_scope_opens_and_records_only_notable_changes_once_a_second() {
        use ecp5_emu::peripherals::audio::Audio;
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut audio = Audio::new([true; 3]);
        audio.feed(0, [[0; 4]; 3]);
        audio.feed(5_000_000_000, [[1; 4]; 3]);
        let mut view = View {
            audio: Some(audio.status()),
            ..View::default()
        };
        let paint = |m: &mut Monitor, v: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                m.show(ui.ctx(), v);
            });
        };
        paint(&mut monitor, &view);
        assert!(monitor.audio_seen && monitor.audio_open);
        assert!(monitor.take_notes()[0].starts_with("audio modelled open-circuit"));
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().is_empty());
        view.audio.as_mut().unwrap().channels[0].hz = Some(440.0);
        paint(&mut monitor, &view);
        assert!(
            monitor.take_notes().is_empty(),
            "throttled even for a new estimate"
        );
        monitor.audio_noted.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        paint(&mut monitor, &view);
        assert!(monitor.take_notes()[0].contains("l_hz=440.000"));
        monitor.audio_noted.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        view.audio.as_mut().unwrap().channels[0].hz = Some(440.1);
        paint(&mut monitor, &view);
        assert!(
            monitor.take_notes().is_empty(),
            "small estimate variation is not a change"
        );
        view.audio.as_mut().unwrap().channels[0].vpp = Some(3.0);
        paint(&mut monitor, &view);
        assert!(monitor.take_notes()[0].contains("l_vpp=3.000"));
    }

    #[test]
    fn rtc_record_changes_with_time_and_access() {
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut view = View {
            rtc: Some(ecp5_emu::peripherals::rtc::Rtc::default().status()),
            ..View::default()
        };
        let paint = |m: &mut Monitor, v: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                m.show(ui.ctx(), v);
            });
        };
        paint(&mut monitor, &view);
        assert_eq!(monitor.take_notes(), ["rtc 2026-01-01 00:00:00 ST 1 OSCRUN 1 bus -- write register -- read 0 written 0 NACKs 0"]);
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().is_empty());
        let s = view.rtc.as_mut().unwrap();
        s.time[5] = 1;
        s.bytes_read = 96;
        s.last_bus_address = Some(0x6f);
        s.last_address = Some(0x5f);
        s.last_read = true;
        paint(&mut monitor, &view);
        assert_eq!(monitor.take_notes(), ["rtc 2026-01-01 00:00:01 ST 1 OSCRUN 1 bus 6F read register 5F read 96 written 0 NACKs 0"]);
    }

    #[test]
    fn adc_record_tracks_codes_and_limits_changed_records_to_one_second() {
        let ctx = egui::Context::default();
        let mut monitor = Monitor::default();
        let mut view = View {
            adc: Some(Adc::new().status()),
            ..View::default()
        };
        let paint = |monitor: &mut Monitor, view: &View| {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                assert!(monitor.show(ui.ctx(), view).is_empty());
            });
        };
        paint(&mut monitor, &view);
        assert_eq!(
            monitor.take_notes(),
            ["adc ---- ---- ---- ---- ---- ---- ---- ---- mode Manual frames 0"]
        );
        view.adc.as_mut().unwrap().codes = [Some(4095); 8];
        view.adc.as_mut().unwrap().frames = 8;
        paint(&mut monitor, &view);
        assert!(monitor.take_notes().is_empty());
        monitor.adc_noted.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        paint(&mut monitor, &view);
        assert_eq!(
            monitor.take_notes(),
            ["adc 0FFF 0FFF 0FFF 0FFF 0FFF 0FFF 0FFF 0FFF mode Manual frames 8"]
        );
        monitor.adc_noted.as_mut().unwrap().1 = Instant::now() - Duration::from_secs(2);
        view.adc.as_mut().unwrap().frames = 16;
        paint(&mut monitor, &view);
        assert!(
            monitor.take_notes().is_empty(),
            "frames alone do not change the record"
        );
    }
}
