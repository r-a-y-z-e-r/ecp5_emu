//! The ULX3S's chips around the FPGA, emulated from what a design drives on
//! their pads (docs/PERIPHERALS.md). The models take pad samples with their
//! times and do not depend on either executor; `Attached` is what a runner
//! holds: the models the design's pins call for, the balls to trace for
//! them (`FastSim::trace`), and a snapshot of what they show for a window.
//! A peripheral that answers (the SD card on its MISO pad, or the SDRAM on
//! its DQ pads) also gives the inputs it drives, how many edges a run may go
//! before it must be asked again (`horizon`), and the levels it drives now
//! (`respond`).
pub mod adc;
pub mod audio;
pub mod edid;
pub mod header;
pub mod oled;
pub mod rtc;
pub mod sdcard;
pub mod sdram;
pub mod spiflash;
pub mod tmds;
pub mod video;
use crate::{board, fast::Trace, logic::Logic};
use adc::Adc;
use audio::Audio;
use edid::Edid;
use oled::{Oled, OledPins, OledStatus};
use rtc::Rtc;
use sdcard::{SdCard, SdPins, SdStatus};
use sdram::{Drive, Sdram, SdramPins};
use spiflash::SpiFlash;
use std::{collections::BTreeMap, sync::Arc};
use video::{VideoDecoder, VideoStatus};

/// What the peripherals show now, cheap to clone: the pictures are shared.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub video: Option<VideoStatus>,
    pub oled: Option<OledStatus>,
    /// The OLED picture as RGBA8 (for a texture), and a serial that changes
    /// only when a pixel may have changed.
    pub oled_rgba: Option<(Arc<[u8]>, u64)>,
    pub sd: Option<SdStatus>,
    pub sdram: Option<sdram::Status>,
    pub flash: Option<spiflash::Status>,
    pub adc: Option<adc::Status>,
    pub rtc: Option<rtc::Status>,
    pub audio: Option<audio::Status>,
    pub edid: Option<edid::Status>,
    pub header: Option<header::Status>,
}

impl View {
    /// The OLED as its glass shows it, held pins-up: width, height, RGBA8 in
    /// row order, and the serial. Some whenever the design drives the OLED.
    pub fn oled_screen(&self) -> Option<(u32, u32, Arc<[u8]>, u64)> {
        self.oled_rgba.as_ref().map(|(rgba, serial)| {
            (
                oled::WIDTH as u32,
                oled::HEIGHT as u32,
                Arc::clone(rgba),
                *serial,
            )
        })
    }
    /// The HDMI mode in a few words, "640x480 59.52 Hz" once the receiver
    /// has a frame, else "signal"; none when the design does not drive the
    /// GPDI pads.
    pub fn hdmi_mode(&self) -> Option<String> {
        let video = self.video.as_ref()?;
        Some(match (&video.timing, video.locked) {
            (Some(t), true) => format!("{}x{} {:.2} Hz", t.width, t.height, t.refresh_hz),
            _ => "signal".into(),
        })
    }
}

/// What going back to a point of the run needs of the peripherals: the
/// OLED's controller and RAM, the SD card's state and written blocks (its
/// image is shared), the flash's protocol state and shared image, the ADC's
/// protocol and voltage sources, and the SDRAM's state and shared rows.
/// The RTC keeps its registers, protocol, fractional second and last trace time.
/// Audio keeps its voltage history, scope ring and resampling phase.
/// EDID keeps its read-only image, pointer, protocol and counters.
/// The HDMI receiver locks again instead.
#[derive(Clone)]
pub struct Snapshot {
    oled: Option<Oled>,
    audio: Option<Audio>,
    edid: Option<Edid>,
    header: Option<header::Header>,
    sd: Option<(SdCard, u8, Vec<Option<bool>>)>,
    flash: Option<(SpiFlash, u8)>,
    adc: Option<(Adc, u8)>,
    rtc: Option<(Rtc, Option<bool>)>,
    sdram: Option<(Sdram, Drive, [Option<bool>; 16])>,
}

/// How many edges a run may go between two looks at an SD card that is not
/// selected: it answers only after a whole 6-byte command, at least 96
/// edges after CS falls even with SCLK at half the board clock.
const SD_IDLE_EDGES: u64 = 64;

fn logic(v: u8) -> Logic {
    match v {
        0 => Logic::Zero,
        1 => Logic::One,
        3 => Logic::Z,
        _ => Logic::X,
    }
}

// A pure input in fast mode takes Boolean levels. Resolve released ADC DOUT
// to idle high, as for SD/flash MISO; the reference host enables a pull-up.
fn adc_level(level: u8) -> Option<bool> {
    Some(level != 0)
}

pub struct Attached {
    audio: Option<Audio>,
    audio_columns: [[Option<usize>; 4]; 3],
    video: Option<VideoDecoder>,
    oled: Option<Oled>,
    /// Columns of each in the trace: GPDI D0, D1, D2, CLK; OLED CS#, D/C,
    /// SCLK, MOSI, RES#.
    video_columns: [usize; 4],
    oled_columns: [usize; 5],
    /// The SD card, its columns (CS, SCLK, MOSI), and the MISO level last
    /// given to the design.
    sd: Option<SdCard>,
    sd_columns: [usize; 3],
    sd_miso: u8,
    sd_native: bool,
    sd_bus_columns: [Option<usize>; 6],
    sd_inputs: Vec<(&'static str, board::Direction)>,
    sd_levels: Vec<Option<bool>>,
    flash: Option<SpiFlash>,
    flash_columns: [usize; 3],
    flash_miso: u8,
    adc: Option<Adc>,
    adc_columns: [usize; 3],
    ain_columns: [Option<usize>; 8],
    adc_miso: u8,
    rtc: Option<Rtc>,
    i2c_columns: [usize; 2],
    /// Last combined RTC/EDID pull-down given to the FPGA on the shared SDA.
    i2c_sda: Option<bool>,
    edid: Option<Edid>,
    header: Option<header::Header>,
    header_columns: Vec<usize>,
    /// The SDRAM, its columns (CLK, CKE, CS#, RAS#, CAS#, WE#, BA[1:0],
    /// A[12:0], DQM[1:0], DQ[15:0]), its latest drive output, and the DQ
    /// levels last given to the design.
    sdram: Option<Sdram>,
    sdram_columns: [usize; 39],
    sdram_drive: Drive,
    sdram_dq: [Option<bool>; 16],
    sdram_answer: Option<(Drive, u8)>,
    balls: Vec<&'static str>,
    rgba: Option<(u64, Arc<[u8]>)>,
}

impl Attached {
    /// The models for the balls `used` drives: the HDMI receiver when all
    /// four `gpdi_dp` are outputs, the OLED when its five pads are, and the
    /// SD card in SPI mode when CS, SCLK and MOSI are outputs and MISO an
    /// input. The card's image is the file `ECP5_SD_IMAGE` names, else
    /// `SdCard::demo`. Flash needs outputs nCS, SCK (U3) and MOSI and an
    /// input MISO; its image is `ECP5_FLASH_IMAGE`, else `SpiFlash::demo`.
    /// ADC needs SCLK, CS and DIN outputs and a DOUT input; its analog pads
    /// may be inputs with pulls, outputs, bidirectional, or unconfigured.
    /// RTC needs driven SCL and bidirectional SDA; ECP5_RTC_IMAGE supplies its state.
    /// Audio observes any driven DAC bit; it supplies no answering input.
    /// The attached HDMI monitor supplies EDID unless ECP5_EDID=off. It
    /// shares the RTC pads when those pads form an executable I2C bus.
    pub fn new(used: &BTreeMap<String, board::Direction>) -> Self {
        let drives = |balls: &[&str]| {
            balls
                .iter()
                .all(|b| used.get(*b).is_some_and(|d| d.drives()))
        };
        let oled_balls = [
            board::OLED_CSN,
            board::OLED_DC,
            board::OLED_CLK,
            board::OLED_MOSI,
            board::OLED_RESN,
        ];
        let mut balls = Vec::new();
        let video = drives(&board::GPDI_DP).then(VideoDecoder::new);
        let mut video_columns = [0; 4];
        if video.is_some() {
            video_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(board::GPDI_DP);
        }
        let oled = drives(&oled_balls).then(Oled::new);
        let mut oled_columns = [0; 5];
        if oled.is_some() {
            oled_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(oled_balls);
        }
        let sd_balls = [board::SD_D[3], board::SD_CLK, board::SD_CMD];
        let sd_native = drives(&[board::SD_CLK])
            && used.get(board::SD_CMD) == Some(&board::Direction::Bidirectional)
            && used.get(board::SD_D[0]).is_some_and(|d| d.receives());
        let mut sd = ((drives(&sd_balls)
            && used.get(board::SD_D[0]).is_some_and(|d| d.receives()))
            || sd_native)
            .then(
                || match std::env::var_os("ECP5_SD_IMAGE").map(std::fs::read) {
                    Some(Ok(image)) => SdCard::new(image),
                    Some(Err(e)) => {
                        eprintln!("SD card: ECP5_SD_IMAGE not read ({e}); the demo card instead");
                        SdCard::demo()
                    }
                    None => SdCard::demo(),
                },
            );
        let mut sd_columns = [0; 3];
        if sd.is_some() && !sd_native {
            sd_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(sd_balls);
        }
        let mut sd_bus_columns = [None; 6];
        let mut sd_inputs = Vec::new();
        let mut sd_levels = Vec::new();
        if sd_native {
            sd.as_mut().unwrap().native_interface();
            for (i, ball) in [
                board::SD_CLK,
                board::SD_CMD,
                board::SD_D[0],
                board::SD_D[1],
                board::SD_D[2],
                board::SD_D[3],
            ]
            .into_iter()
            .enumerate()
            {
                if used.contains_key(ball) {
                    sd_bus_columns[i] =
                        Some(if let Some(c) = balls.iter().position(|&b| b == ball) {
                            c
                        } else {
                            balls.push(ball);
                            balls.len() - 1
                        });
                }
            }
            for ball in [
                board::SD_CMD,
                board::SD_D[0],
                board::SD_D[1],
                board::SD_D[2],
                board::SD_D[3],
                board::SD_CDN,
                board::SD_WP,
            ] {
                if let Some(&direction) = used.get(ball).filter(|d| d.receives()) {
                    sd_inputs.push((ball, direction));
                    sd_levels.push(if ball == board::SD_CDN || ball == board::SD_WP {
                        Some(false)
                    } else if direction == board::Direction::Input || ball == board::SD_D[3] {
                        Some(true)
                    } else {
                        None
                    });
                }
            }
        }
        let sdram_ctl = [
            board::SDRAM_CLK,
            board::SDRAM_CKE,
            board::SDRAM_CSN,
            board::SDRAM_RASN,
            board::SDRAM_CASN,
            board::SDRAM_WEN,
        ];
        let sdram = (drives(&sdram_ctl)
            && drives(&board::SDRAM_BA)
            && drives(&board::SDRAM_A)
            && drives(&board::SDRAM_DQM)
            && board::SDRAM_D
                .iter()
                .all(|b| used.get(*b) == Some(&board::Direction::Bidirectional)))
        .then(Sdram::new);
        let mut sdram_columns = [0; 39];
        if sdram.is_some() {
            let sdram_balls: Vec<&'static str> = sdram_ctl
                .into_iter()
                .chain(board::SDRAM_BA)
                .chain(board::SDRAM_A)
                .chain(board::SDRAM_DQM)
                .chain(board::SDRAM_D)
                .collect();
            sdram_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(sdram_balls);
        }
        let flash_balls = [board::FLASH_CSN, board::FLASH_SCK, board::FLASH_MOSI];
        let mut flash_columns = [0; 3];
        let flash = (drives(&flash_balls)
            && used.get(board::FLASH_MISO).is_some_and(|d| d.receives()))
        .then(SpiFlash::from_env);
        if flash.is_some() {
            flash_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(flash_balls);
        }
        let adc_balls = [board::ADC_CSN, board::ADC_SCLK, board::ADC_MOSI];
        let mut adc = (drives(&adc_balls)
            && used.get(board::ADC_MISO).is_some_and(|d| d.receives()))
        .then(Adc::from_env);
        let mut adc_columns = [0; 3];
        let mut ain_columns = [None; 8];
        if let Some(adc) = &mut adc {
            adc_columns = std::array::from_fn(|i| balls.len() + i);
            balls.extend(adc_balls);
            for (i, ball) in board::ADC_AIN.into_iter().enumerate() {
                // An unconfigured PIO has no executable pad; it is floating.
                // Configured inputs are traced too, so their pulls reach the ADC.
                if let Some(direction) = used.get(ball) {
                    adc.set_driven(i, direction.drives());
                    ain_columns[i] = Some(balls.len());
                    balls.push(ball);
                }
            }
        }
        let rtc = (drives(&[board::RTC_SCL])
            && used.get(board::RTC_SDA) == Some(&board::Direction::Bidirectional))
        .then(Rtc::from_env);
        let edid = video.as_ref().map(|_| Edid::from_env());
        let mut i2c_columns = [0; 2];
        if rtc.is_some() {
            i2c_columns = [balls.len(), balls.len() + 1];
            balls.extend([board::RTC_SCL, board::RTC_SDA]);
        }
        let mut audio_columns = [[None; 4]; 3];
        let audio_active = [board::AUDIO_L, board::AUDIO_R, board::AUDIO_V].map(|channel| {
            channel
                .iter()
                .any(|ball| used.get(*ball).is_some_and(|d| d.drives()))
        });
        let audio = audio_active
            .iter()
            .any(|&a| a)
            .then(|| Audio::new(audio_active));
        for (channel, pads) in [board::AUDIO_L, board::AUDIO_R, board::AUDIO_V]
            .into_iter()
            .enumerate()
        {
            for (bit, ball) in pads.into_iter().enumerate() {
                if used.get(ball).is_some_and(|d| d.drives()) {
                    audio_columns[channel][bit] = Some(balls.len());
                    balls.push(ball);
                }
            }
        }
        let header = header::Header::new(used);
        let header_columns = header
            .as_ref()
            .into_iter()
            .flat_map(header::Header::indices)
            .map(|i| {
                let ball = board::HEADER[i].ball;
                if let Some(column) = balls.iter().position(|&b| b == ball) {
                    column
                } else {
                    balls.push(ball);
                    balls.len() - 1
                }
            })
            .collect();
        Self {
            header,
            header_columns,
            audio,
            audio_columns,
            rtc,
            i2c_columns,
            i2c_sda: None,
            edid,
            adc,
            adc_columns,
            ain_columns,
            adc_miso: 3,
            video,
            oled,
            video_columns,
            oled_columns,
            sd_miso: sd.as_ref().map_or(1, SdCard::miso),
            sd,
            sd_columns,
            sd_native,
            sd_bus_columns,
            sd_inputs,
            sd_levels,
            flash,
            flash_columns,
            flash_miso: 1,
            sdram,
            sdram_columns,
            sdram_drive: None,
            sdram_dq: [None; 16],
            sdram_answer: None,
            balls,
            rgba: None,
        }
    }
    /// The design's inputs the peripherals drive, with their level at the
    /// start (`None` = Z): SD, flash and ADC MISO high, the SDRAM's 16 DQ pads
    /// high impedance. RTC/EDID share one open-drain SDA driver: either
    /// device can pull low, otherwise it releases to the pad pull-up.
    pub fn input_balls(&self) -> Vec<(&'static str, Option<bool>)> {
        let mut out = Vec::new();
        if self.sd_native {
            out.extend(
                self.sd_inputs
                    .iter()
                    .zip(&self.sd_levels)
                    .map(|(&(ball, _), &level)| (ball, level)),
            );
        } else if self.sd.is_some() {
            out.push((board::SD_D[0], Some(self.sd_miso == 1)));
        }
        if self.flash.is_some() {
            out.push((board::FLASH_MISO, Some(self.flash_miso == 1)));
        }
        if self.adc.is_some() {
            out.push((board::ADC_MISO, adc_level(self.adc_miso)));
        }
        if self.rtc.is_some() {
            out.push((board::RTC_SDA, self.i2c_sda));
        }
        if self.sdram.is_some() {
            for (&ball, &level) in board::SDRAM_D.iter().zip(&self.sdram_dq) {
                out.push((ball, level));
            }
        }
        out
    }
    /// How many clock edges a run may go before the peripherals must be fed
    /// and asked to `respond`: one while SDRAM is attached or SD, flash or
    /// ADC is selected (SPI answers on falling SCLK, before rising SCLK),
    /// one during an I2C transfer, 16 in I2C idle; none when nothing answers.
    /// At 25 MHz, 16 edges are 0.32 us: less than even the 400 kHz
    /// I2C START hold time (0.6 us), before the first bit clock. Standard
    /// 100 kHz holds START for 4 us. Faster buses/PLL SCL are unsupported.
    /// Native SD idle also uses 16: a 48-bit command needs at least 96
    /// edges plus NCR, so a chunk catches its start before any response.
    pub fn horizon(&self) -> Option<u64> {
        if self.sdram.is_some() {
            return Some(1);
        }
        let selected = self.sd.as_ref().is_some_and(SdCard::selected)
            || self.flash.as_ref().is_some_and(SpiFlash::selected)
            || self.adc.as_ref().is_some_and(Adc::selected)
            || self.rtc.as_ref().is_some_and(Rtc::active)
            || self.edid.as_ref().is_some_and(Edid::active);
        (self.sd.is_some() || self.flash.is_some() || self.adc.is_some() || self.rtc.is_some())
            .then_some(if selected {
                1
            } else if self.rtc.is_some() || self.sd_native {
                16
            } else {
                SD_IDLE_EDGES
            })
    }
    /// Flash and ADC can answer within a short SPI frame. Even while CS is
    /// high, watch each edge in a batch so a newly asserted CS cannot hide
    /// an entire command before the next answer. `horizon` still bounds
    /// the delay between answers; the SD card has a longer command frame.
    pub fn observe_every_edge(&self) -> bool {
        self.flash.is_some() || self.adc.is_some() || self.horizon() == Some(1)
    }
    /// The inputs (by `input_balls` index) whose level the peripherals now
    /// drive differently from what the design was last given; the runner
    /// gives them just after the edge it ran to.
    pub fn respond(&mut self) -> Vec<(usize, Option<bool>)> {
        let mut out = Vec::new();
        self.respond_into(&mut out);
        out
    }
    /// Append changed answers to a buffer the runner can reuse each edge.
    pub fn respond_into(&mut self, out: &mut Vec<(usize, Option<bool>)>) {
        let sd_count = if self.sd_native {
            self.sd_inputs.len()
        } else {
            usize::from(self.sd.is_some())
        };
        if let Some(sd) = &self.sd {
            if self.sd_native {
                let mut levels = sd.bus_outputs();
                if levels[4].is_none() && sd.dat3_bias() {
                    levels[4] = Some(true);
                }
                for (index, &(ball, direction)) in self.sd_inputs.iter().enumerate() {
                    let level = if ball == board::SD_CDN || ball == board::SD_WP {
                        Some(false)
                    } else {
                        let slot = [
                            board::SD_CMD,
                            board::SD_D[0],
                            board::SD_D[1],
                            board::SD_D[2],
                            board::SD_D[3],
                        ]
                        .iter()
                        .position(|&b| b == ball)
                        .unwrap();
                        levels[slot]
                            .or_else(|| (direction == board::Direction::Input).then_some(true))
                    };
                    if self.sd_levels[index] != level {
                        self.sd_levels[index] = level;
                        out.push((index, level));
                    }
                }
            } else if sd.miso() != self.sd_miso {
                self.sd_miso = sd.miso();
                out.push((0, Some(self.sd_miso == 1)));
            }
        }
        if let Some(flash) = &self.flash {
            if flash.miso() != self.flash_miso {
                self.flash_miso = flash.miso();
                out.push((sd_count, Some(self.flash_miso == 1)));
            }
        }
        if let Some(adc) = &self.adc {
            if adc.miso() != self.adc_miso {
                self.adc_miso = adc.miso();
                let index = sd_count + usize::from(self.flash.is_some());
                out.push((index, adc_level(self.adc_miso)));
            }
        }
        if let Some(rtc) = &self.rtc {
            let sda = (rtc.sda() == Some(false)
                || self.edid.as_ref().is_some_and(|e| e.sda() == Some(false)))
            .then_some(false);
            if sda != self.i2c_sda {
                self.i2c_sda = sda;
                let index =
                    sd_count + usize::from(self.flash.is_some()) + usize::from(self.adc.is_some());
                out.push((index, self.i2c_sda));
            }
        }
        if let Some(sdram) = &self.sdram {
            let base = sd_count
                + usize::from(self.flash.is_some())
                + usize::from(self.adc.is_some())
                + usize::from(self.rtc.is_some());
            let mask = sdram.drive_mask();
            if self.sdram_answer == Some((self.sdram_drive, mask)) {
                return;
            }
            self.sdram_answer = Some((self.sdram_drive, mask));
            for (i, dq) in self.sdram_dq.iter_mut().enumerate() {
                let level = match self.sdram_drive {
                    Some(v) if mask & (1 << (i / 8)) != 0 => Some(v & (1 << i) != 0),
                    _ => None,
                };
                if level != *dq {
                    *dq = level;
                    out.push((base + i, level));
                }
            }
        }
    }
    /// External header position and its shared AIN voltage, checkpointed together.
    pub fn set_header_level(&mut self, index: usize, level: Option<bool>) -> bool {
        if !self
            .header
            .as_mut()
            .is_some_and(|h| h.set_level(index, level))
        {
            return false;
        }
        if let Some(channel) = board::ADC_AIN
            .iter()
            .position(|&b| b == board::HEADER[index].ball)
        {
            if let Some(adc) = &mut self.adc {
                adc.set_header(channel, level);
            }
        }
        true
    }
    pub fn header_levels(&self) -> Vec<Option<bool>> {
        self.header
            .as_ref()
            .map_or_else(Vec::new, header::Header::levels)
    }
    /// An external analog input; included in snapshots and replayed by the runner.
    pub fn set_volts(&mut self, channel: usize, volts: f64) -> bool {
        self.adc
            .as_mut()
            .is_some_and(|adc| adc.set_volts(channel, volts))
    }
    /// Whether any peripheral is attached.
    pub fn any(&self) -> bool {
        !self.balls.is_empty()
    }
    /// The balls to trace, in the column order `feed` expects.
    pub fn balls(&self) -> Vec<&'static str> {
        self.balls.clone()
    }
    /// A trace of `balls()` (`FastSim::take_trace`).
    pub fn feed(&mut self, trace: &Trace) {
        if trace.width != self.balls.len() || trace.rows.len() != trace.times_ps.len() * trace.width
        {
            return;
        }
        if trace.gap {
            if let Some(video) = &mut self.video {
                video.gap();
            }
        }
        let (v, o, d) = (self.video_columns, self.oled_columns, self.sd_columns);
        for (&ps, row) in trace
            .times_ps
            .iter()
            .zip(trace.rows.chunks_exact(trace.width.max(1)))
        {
            if let Some(header) = &mut self.header {
                header.feed(ps, self.header_columns.iter().map(|&c| row[c]));
            }
            if let Some(audio) = &mut self.audio {
                audio.feed(
                    ps,
                    self.audio_columns
                        .map(|c| c.map(|column| column.map_or(3, |c| row[c]))),
                );
            }
            if let Some(video) = &mut self.video {
                video.sample(ps, [row[v[0]], row[v[1]], row[v[2]], row[v[3]]]);
            }
            if let Some(oled) = &mut self.oled {
                oled.feed(
                    ps,
                    OledPins {
                        csn: row[o[0]],
                        dc: row[o[1]],
                        clk: row[o[2]],
                        mosi: row[o[3]],
                        resn: row[o[4]],
                    },
                );
            }
            if let Some(sd) = &mut self.sd {
                if self.sd_native {
                    let c = self.sd_bus_columns;
                    sd.feed_bus(
                        ps,
                        sdcard::native::Pins {
                            clk: c[0].map_or(3, |c| row[c]),
                            cmd: c[1].map_or(3, |c| row[c]),
                            dat: std::array::from_fn(|i| c[i + 2].map_or(3, |c| row[c])),
                        },
                    );
                } else {
                    sd.feed(
                        ps,
                        SdPins {
                            cs: row[d[0]],
                            clk: row[d[1]],
                            mosi: row[d[2]],
                        },
                    );
                }
            }
            if let Some(flash) = &mut self.flash {
                let c = self.flash_columns;
                flash.feed(
                    ps,
                    spiflash::Pins {
                        cs: row[c[0]],
                        clk: row[c[1]],
                        mosi: row[c[2]],
                    },
                );
            }
            if let Some(adc) = &mut self.adc {
                let c = self.adc_columns;
                adc.feed(
                    ps,
                    adc::Pins {
                        cs: row[c[0]],
                        clk: row[c[1]],
                        mosi: row[c[2]],
                        ain: self.ain_columns.map(|c| c.map_or(3, |c| row[c])),
                    },
                );
            }
            if let Some(rtc) = &mut self.rtc {
                let c = self.i2c_columns;
                let pins = rtc::Pins {
                    scl: row[c[0]],
                    sda: row[c[1]],
                };
                // Both devices see the same resolved bus, before combining
                // their answers. Neither may drive SDA high.
                rtc.feed(ps, pins);
                if let Some(edid) = &mut self.edid {
                    edid.feed(ps, pins);
                }
            }
            if let Some(sdram) = &mut self.sdram {
                let c = self.sdram_columns;
                self.sdram_drive = sdram.feed(
                    ps,
                    &SdramPins {
                        clk: logic(row[c[0]]),
                        cke: logic(row[c[1]]),
                        csn: logic(row[c[2]]),
                        rasn: logic(row[c[3]]),
                        casn: logic(row[c[4]]),
                        wen: logic(row[c[5]]),
                        ba: std::array::from_fn(|i| logic(row[c[6 + i]])),
                        a: std::array::from_fn(|i| logic(row[c[8 + i]])),
                        dqm: std::array::from_fn(|i| logic(row[c[21 + i]])),
                        dq: std::array::from_fn(|i| logic(row[c[23 + i]])),
                    },
                );
            }
        }
    }
    /// What they show now. The OLED renders here, at the window's pace, not
    /// on every byte it takes.
    pub fn view(&mut self) -> View {
        let oled = self.oled.as_mut().map(Oled::status);
        let oled_rgba = oled.as_ref().map(|s| {
            if self.rgba.as_ref().is_none_or(|(v, _)| *v != s.version) {
                let rgba: Arc<[u8]> = s
                    .image
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|&[r, g, b]| [r, g, b, 255])
                    .collect();
                self.rgba = Some((s.version, rgba));
            }
            let (version, rgba) = self.rgba.as_ref().unwrap();
            (Arc::clone(rgba), *version)
        });
        View {
            video: self.video.as_ref().map(VideoDecoder::status),
            oled,
            oled_rgba,
            sd: self.sd.as_ref().map(SdCard::status),
            flash: self.flash.as_ref().map(SpiFlash::status),
            adc: self.adc.as_ref().map(Adc::status),
            rtc: self.rtc.as_ref().map(Rtc::status),
            audio: self.audio.as_ref().map(Audio::status),
            edid: self.edid.as_ref().map(Edid::status),
            header: self.header.as_ref().map(header::Header::status),
            sdram: self.sdram.as_ref().map(Sdram::status),
        }
    }
    /// Final CLI counters, in the fixed order and format in PERIPHERALS.md.
    /// Each line includes its `peripheral:` prefix; absent models emit nothing.
    pub fn report(&mut self) -> Vec<String> {
        let view = self.view();
        let mut lines = Vec::new();
        let yes = |value| if value { "yes" } else { "no" };
        if let Some(s) = view.sdram {
            lines.push(format!("peripheral: sdram init={} reads={} writes={} timing_violations={} unknown_samples={} refresh_shortfalls={}",
                yes(s.initialised), s.reads, s.writes, s.timing_violations, s.unknown_samples, s.refresh_shortfalls));
            lines.extend(
                s.messages
                    .into_iter()
                    .map(|message| format!("peripheral: sdram message: {message}")),
            );
        }
        if let Some(s) = view.sd {
            let state = match s.state {
                sdcard::SdState::Idle => "idle",
                sdcard::SdState::Ready => "ready",
                sdcard::SdState::Reading => "reading",
                sdcard::SdState::Writing => "writing",
            };
            lines.push(format!("peripheral: sd state={state} last_command={} last_argument={:08X} blocks_read={} blocks_written={} crc_errors={} illegal_commands={} mode={} card_state={} rca={:04X} bus_width={} commands={} responses={} dat3_pullup={}",
                s.last_command.map_or("--".into(), |c| c.to_string()), s.last_argument,
                s.blocks_read, s.blocks_written, s.crc_errors, s.illegal_commands,s.mode,s.native_state.map_or(state,sdcard::native::State::name),s.rca,s.bus_width,s.commands,s.responses,u8::from(s.dat3_pullup)));
        }
        if let Some(s) = view.flash {
            let [a, b, c] = s.jedec;
            lines.push(format!("peripheral: flash jedec={a:02X}{b:02X}{c:02X} last_command={} last_address={} bytes_read={} ignored_writes={} programs={} erases={} protected={} busy_ignored={} config_writes={}",
                s.last_command.map_or("--".into(), |c| format!("{c:02X}")),
                s.last_address.map_or("------".into(), |a| format!("{a:06X}")), s.bytes_read, s.ignored_writes,
                s.programs, s.erases, s.protected, s.busy_ignored, s.config_writes));
            if let Some(warning) = s.first_config_write {
                lines.push(format!("peripheral: flash warning: {warning}"));
            }
        }
        if let Some(s) = view.adc {
            lines.push(format!(
                "peripheral: adc frames={} unsupported_frames={} mode={:04X} codes={}",
                s.frames,
                s.unsupported_frames,
                s.mode,
                s.codes
                    .iter()
                    .map(|c| c.map_or("----".into(), |c| format!("{c:04X}")))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        if let Some(s) = view.rtc {
            let [year, month, date, hour, minute, second] = s.time;
            lines.push(format!("peripheral: rtc time={year:04}-{month:02}-{date:02}T{hour:02}:{minute:02}:{second:02} st={} oscrun={} last_bus_address={} last_address={} bytes_read={} bytes_written={} nacks={}",
                u8::from(s.st), u8::from(s.oscrun),
                s.last_bus_address.map_or("--".into(), |a| format!("{a:02X}")),
                s.last_address.map_or("--".into(), |a| format!("{a:02X}")), s.bytes_read, s.bytes_written, s.nacks));
        }
        if let Some(s) = view.oled {
            lines.push(format!("peripheral: oled frames={} crc32={:08X} commands={} data_bytes={} errors={} unknown_commands={} display_on={}",
                s.frames, s.crc32, s.commands, s.data_bytes, s.errors, s.unknown_commands, yes(s.display_on)));
        }
        if let Some(s) = view.video {
            let (width, height, crc) = s
                .last_frame
                .as_ref()
                .map_or((0, 0, "--------".into()), |f| {
                    (f.width, f.height, format!("{:08X}", f.crc32))
                });
            lines.push(format!("peripheral: hdmi frames={} width={width} height={height} crc32={crc} invalid_symbols={} relocks={}",
                s.frames_decoded, s.invalid_symbols, s.relocks));
        }
        if let Some(s) = view.edid {
            lines.push(format!(
                "peripheral: edid present={} reads={} bytes_read={} last_pointer={:02X} nacks={}",
                yes(s.present),
                s.reads,
                s.bytes_read,
                s.last_pointer,
                s.nacks
            ));
        }
        if let Some(s) = view.header {
            let fields = s
                .pins
                .iter()
                .filter(|p| p.direction.drives())
                .map(|p| {
                    let hz = p.hz.map_or("--".into(), |v| format!("{v:.3}"));
                    let duty = p.duty.map_or("--".into(), |v| format!("{:.3}", v * 100.0));
                    format!("{}={hz}Hz/{duty}%", board::HEADER[p.index].name)
                })
                .collect::<Vec<_>>();
            lines.push(if fields.is_empty() {
                "peripheral: header".into()
            } else {
                format!("peripheral: header {}", fields.join(" "))
            });
        }
        if let Some(s) = view.audio {
            let number =
                |n: Option<f64>, precision| n.map_or("--".into(), |v| format!("{v:.precision$}"));
            let [l, r, v] = s.channels;
            lines.push(format!("peripheral: audio l_hz={} l_vpp={} r_hz={} r_vpp={} v_hz={} v_vpp={} transitions={}",
                number(l.hz, 3), number(l.vpp, 6), number(r.hz, 3), number(r.vpp, 6), number(v.hz, 3), number(v.vpp, 6), s.transitions));
        }
        lines
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            oled: self.oled.clone(),
            audio: self.audio.clone(),
            edid: self.edid.clone(),
            header: self.header.clone(),
            sd: self
                .sd
                .clone()
                .map(|sd| (sd, self.sd_miso, self.sd_levels.clone())),
            flash: self.flash.clone().map(|flash| (flash, self.flash_miso)),
            adc: self.adc.clone().map(|adc| (adc, self.adc_miso)),
            rtc: self.rtc.clone().map(|rtc| (rtc, self.i2c_sda)),
            sdram: self
                .sdram
                .clone()
                .map(|sdram| (sdram, self.sdram_drive, self.sdram_dq)),
        }
    }
    /// Back (or on) to a snapshot: the OLED as it was, the HDMI receiver
    /// starting its lock again (its next frame is the first shown).
    pub fn restore(&mut self, s: &Snapshot) {
        self.header.clone_from(&s.header);
        if let (Some(edid), Some(was)) = (&mut self.edid, &s.edid) {
            edid.clone_from(was);
        }
        if let (Some(audio), Some(was)) = (&mut self.audio, &s.audio) {
            audio.clone_from(was);
        }
        if let (Some(oled), Some(was)) = (&mut self.oled, &s.oled) {
            oled.clone_from(was);
        }
        if let (Some(sd), Some((was, miso, levels))) = (&mut self.sd, &s.sd) {
            sd.clone_from(was);
            self.sd_miso = *miso;
            self.sd_levels.clone_from(levels);
        }
        if let (Some(flash), Some((was, miso))) = (&mut self.flash, &s.flash) {
            flash.clone_from(was);
            self.flash_miso = *miso;
        }
        if let (Some(adc), Some((was, miso))) = (&mut self.adc, &s.adc) {
            adc.clone_from(was);
            self.adc_miso = *miso;
        }
        if let (Some(rtc), Some((was, sda))) = (&mut self.rtc, &s.rtc) {
            rtc.clone_from(was);
            self.i2c_sda = *sda;
        }
        if let (Some(sdram), Some((was, drive, dq))) = (&mut self.sdram, &s.sdram) {
            sdram.clone_from(was);
            self.sdram_drive = *drive;
            self.sdram_dq = *dq;
            self.sdram_answer = None;
        }
        if let Some(video) = &mut self.video {
            video.gap();
        }
    }
}

#[cfg(test)]
mod tests;
