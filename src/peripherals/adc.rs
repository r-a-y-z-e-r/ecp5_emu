//! MAX11125, the ULX3S's eight-channel, 12-bit ADC. See docs/PERIPHERALS_ADC.md
//! and the MAX11120–MAX11128 datasheet, Rev 5, Figures 2 and Tables 1–13.
//! SPI mode 3: DIN on rising SCLK, DOUT on falling SCLK, MSB first, 16 clocks.
//! A command selects the conversion at the next falling CS, not the current one.
//! Assumptions: ideal rails and instantaneous conversion, no acquisition, power-up
//! or output delays; invalid/unsupported results and the first result are zero.
//! The requested endpoint formula uses 4095, rather than the silicon's 4096 LSBs.
//! Internal-clock/FIFO/averaging and SampleSet are not implemented. PM wake-up
//! timing is ignored; REF- is board ground and REF+/FPGA high are ideal 3.3 V.

pub const VREF: f64 = 3.3;

/// Resolved pad levels: 0/1 include FPGA drive or pull, 2 is X, 3 is Z.
#[derive(Clone, Copy, Debug)]
pub struct Pins {
    pub cs: u8,
    pub clk: u8,
    pub mosi: u8,
    pub ain: [u8; 8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub volts: [f64; 8],
    /// Last completed conversion on each channel, including its two's complement code.
    pub codes: [Option<u16>; 8],
    /// Whether the design can drive the shared header pad (no slider then).
    pub driven: [bool; 8],
    pub mode: u16,
    pub configuration: u16,
    pub unipolar: u16,
    pub bipolar: u16,
    pub range: u16,
    pub frames: u64,
    pub unsupported_frames: u64,
}
impl Status {
    pub fn mode_name(&self) -> &'static str {
        match (self.mode >> 11) & 15 {
            1 => "Manual",
            4 => "Standard_Ext",
            6 => "Upper_Ext",
            8 => "Custom_Ext",
            _ => "unsupported scan",
        }
    }
}

#[derive(Clone)]
pub struct Adc {
    status: Status,
    previous: Pins,
    defaults: [f64; 8],
    overrides: [Option<f64>; 8],
    header: [Option<bool>; 8],
    shift: u16,
    bits: u8,
    falls: u8,
    output: u16,
    miso: u8,
    valid: bool,
    active: bool,
    scan_channel: usize,
    custom: u8,
    echo: Option<u16>,
}
impl Default for Adc {
    fn default() -> Self {
        Self::new()
    }
}
impl Adc {
    pub fn new() -> Self {
        Self {
            status: Status {
                volts: [0.0; 8],
                codes: [None; 8],
                driven: [false; 8],
                mode: 0x0800,
                configuration: 0,
                unipolar: 0,
                bipolar: 0,
                range: 0,
                frames: 0,
                unsupported_frames: 0,
            },
            previous: Pins {
                cs: 1,
                clk: 1,
                mosi: 0,
                ain: [3; 8],
            },
            defaults: [0.0; 8],
            overrides: [None; 8],
            header: [None; 8],
            shift: 0,
            bits: 0,
            falls: 0,
            output: 0,
            miso: 3,
            valid: true,
            active: false,
            scan_channel: 0,
            custom: 0,
            echo: None,
        }
    }
    /// Overrides at start: ECP5_ADC_VOLTS="0=1.65,7=3.3". Invalid entries are ignored.
    pub fn from_env() -> Self {
        let mut adc = Self::new();
        if let Ok(values) = std::env::var("ECP5_ADC_VOLTS") {
            adc.apply_volts(&values);
        }
        adc
    }
    /// Parse the same syntax as ECP5_ADC_VOLTS; return the accepted entry count.
    pub fn apply_volts(&mut self, values: &str) -> usize {
        values
            .split(',')
            .filter(|entry| {
                let Some((channel, volts)) = entry.split_once('=') else {
                    return false;
                };
                match (channel.trim().parse(), volts.trim().parse()) {
                    (Ok(channel), Ok(volts)) => self.set_volts(channel, volts),
                    _ => false,
                }
            })
            .count()
    }
    /// A finite external voltage takes precedence over the pad, including its pull.
    pub fn set_volts(&mut self, channel: usize, volts: f64) -> bool {
        if channel >= 8 || !volts.is_finite() {
            return false;
        }
        self.overrides[channel] = Some(volts);
        self.sample_volts(self.previous.ain);
        true
    }
    /// An externally driven shared header wins over AdcVolts. Releasing it
    /// restores the retained voltage override or pad/default source.
    pub fn set_header(&mut self, channel: usize, level: Option<bool>) {
        self.header[channel] = level;
        self.sample_volts(self.previous.ain);
    }
    /// The voltage used for a floating or unknown pad without an override.
    pub fn set_default_volts(&mut self, channel: usize, volts: f64) -> bool {
        if channel >= 8 || !volts.is_finite() {
            return false;
        }
        self.defaults[channel] = volts;
        self.sample_volts(self.previous.ain);
        true
    }
    pub fn set_driven(&mut self, channel: usize, driven: bool) {
        if let Some(value) = self.status.driven.get_mut(channel) {
            *value = driven;
        }
    }
    pub fn status(&self) -> Status {
        self.status.clone()
    }
    pub fn selected(&self) -> bool {
        self.previous.cs == 0
    }
    /// 3 is high impedance while deselected or after the output word.
    pub fn miso(&self) -> u8 {
        self.miso
    }

    fn sample_volts(&mut self, ain: [u8; 8]) {
        for (i, level) in ain.into_iter().enumerate() {
            self.status.volts[i] = self.header[i]
                .map(|high| if high { VREF } else { 0.0 })
                .or(self.overrides[i])
                .unwrap_or(match level {
                    0 => 0.0,
                    1 => VREF,
                    _ => self.defaults[i],
                });
        }
    }
    fn conversion(&mut self, channel: usize) -> u16 {
        let pair = channel / 2;
        let mask = 1 << (10 - pair);
        let common = self.status.unipolar & 4 != 0;
        let uni = !common && self.status.unipolar & mask != 0;
        let bip = !common && !uni && self.status.bipolar & mask != 0;
        let volts = if uni || bip {
            // Table 9: either CHSEL of a pair converts even AIN minus odd AIN.
            self.status.volts[pair * 2] - self.status.volts[pair * 2 + 1]
        } else {
            self.status.volts[channel]
        };
        let code = if bip {
            let span = if self.status.range & mask == 0 {
                VREF
            } else {
                2.0 * VREF
            };
            ((volts / span * 4096.0).round().clamp(-2048.0, 2047.0) as i16 as u16) & 4095
        } else {
            (volts / VREF * 4095.0).round().clamp(0.0, 4095.0) as u16
        };
        self.status.codes[channel] = Some(code);
        // Assumption: differential CHAN_ID keeps the requested pair member.
        // No-ID padding stays zero through all 16 clocks; the physical output
        // release during those final padding bits is not timed here.
        if self.status.mode & 4 != 0 {
            ((channel as u16) << 12) | code
        } else {
            code << 3
        }
    }
    fn begin(&mut self) {
        self.bits = 0;
        self.falls = 0;
        self.shift = 0;
        self.valid = true;
        self.miso = 3;
        self.output = 0;
        if let Some(echo) = self.echo.take() {
            self.output = echo;
            return;
        }
        if !self.active || self.status.configuration & 0x18 != 0 {
            return;
        }
        let mode = (self.status.mode >> 11) & 15;
        let n = ((self.status.mode >> 7) & 15) as usize;
        let channel = match mode {
            1 if n < 8 => n,
            4 if n < 8 => self.scan_channel.min(n),
            6 if n < 8 => self.scan_channel.max(n),
            8 if self.custom != 0 => (0..8)
                .map(|i| (self.scan_channel + i) % 8)
                .find(|&i| self.custom & (1 << i) != 0)
                .unwrap(),
            _ => {
                self.status.unsupported_frames += 1;
                return;
            }
        };
        self.output = self.conversion(channel);
        self.scan_channel = match mode {
            4 if channel == n => 0,
            6 if channel == 7 => n,
            _ => (channel + 1) % 8,
        };
    }
    fn instruction(&mut self, word: u16) {
        if word & 0x8000 == 0 {
            if (word >> 5) & 3 == 2 {
                // Register reset preserves external voltage sources and status counters.
                self.status.mode = 0x0800;
                self.status.configuration = 0;
                self.status.unipolar = 0;
                self.status.bipolar = 0;
                self.status.range = 0;
                self.custom = 0;
                self.echo = None;
                self.active = false;
                self.scan_channel = 0;
                return;
            }
            // RESET=01 resets the unimplemented FIFO only.
            let scan = (word >> 11) & 15;
            if (1..=9).contains(&scan) {
                self.status.mode = word & !0x60;
                self.scan_channel = if scan == 6 {
                    ((word >> 7) & 7) as usize
                } else {
                    0
                };
            } else if scan == 0 {
                // Table 3 permits PM, RESET and SWCNV writes even with SCAN=0.
                self.status.mode = (self.status.mode & !0x1a) | (word & 0x1a);
            }
            self.active = true;
        } else {
            match word >> 11 {
                0x10 => {
                    self.status.configuration = word & 0x7ff;
                }
                0x11 => self.status.unipolar = word & 0x7ff,
                0x12 => self.status.bipolar = word & 0x7ff,
                0x13 => self.status.range = word & 0x7ff,
                0x15 => self.custom = (word >> 3) as u8,
                _ => {
                    self.status.unsupported_frames += 1;
                }
            }
            // Configuration invalidates following results until a mode instruction (p.34).
            self.active = false;
            // Assumption: ECHO returns each configuration instruction one frame later;
            // mode instructions resume conversion, and are not echoed.
            if self.status.configuration & 4 != 0 {
                self.echo = Some(word);
            }
        }
    }
    /// Samples in time order. A falling CS holds the analog sample for this frame.
    /// Short frames and unknown DIN/SCLK discard the instruction until the next CS.
    /// Assumption: only the first 16 clocks write an instruction; extra clocks are ignored.
    pub fn feed(&mut self, _time_ps: u64, pins: Pins) {
        let old = self.previous;
        self.previous = pins;
        self.sample_volts(pins.ain);
        if pins.cs != 0 {
            self.miso = 3;
            return;
        }
        if old.cs != 0 {
            self.begin();
            return;
        }
        if pins.clk > 1 || old.clk > 1 {
            self.valid = false;
            self.miso = 3;
        } else if old.clk == 1 && pins.clk == 0 {
            self.miso = if self.valid && self.falls < 16 {
                let bit = (self.output >> (15 - self.falls)) as u8 & 1;
                self.falls += 1;
                bit
            } else {
                3
            };
        } else if old.clk == 0 && pins.clk == 1 && self.bits < 16 {
            if pins.mosi > 1 {
                self.valid = false;
            }
            self.shift = (self.shift << 1) | u16::from(pins.mosi & 1);
            self.bits += 1;
            if self.bits == 16 && self.valid {
                self.status.frames += 1;
                self.instruction(self.shift);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        adc: Adc,
        pins: Pins,
        time: u64,
    }
    impl Host {
        fn new() -> Self {
            Self {
                adc: Adc::new(),
                pins: Pins {
                    cs: 1,
                    clk: 1,
                    mosi: 0,
                    ain: [1, 0, 1, 0, 1, 0, 1, 0],
                },
                time: 0,
            }
        }
        fn feed(&mut self) {
            self.time += 10;
            self.adc.feed(self.time, self.pins);
        }
        fn select(&mut self) {
            self.pins.cs = 0;
            self.feed();
        }
        fn bit(&mut self, bit: u8) -> u8 {
            self.pins.clk = 0;
            self.pins.mosi = bit;
            self.feed();
            let got = self.adc.miso();
            self.pins.clk = 1;
            self.feed();
            assert_eq!(self.adc.miso(), got, "DOUT holds through rising SCLK");
            got
        }
        fn end(&mut self) {
            self.pins.cs = 1;
            self.pins.clk = 1;
            self.feed();
            assert_eq!(self.adc.miso(), 3);
        }
        fn frame(&mut self, word: u16) -> u16 {
            self.select();
            let mut out = 0;
            for i in (0..16).rev() {
                out = (out << 1) | u16::from(self.bit(((word >> i) & 1) as u8));
            }
            self.end();
            out
        }
    }
    #[test]
    fn header_source_temporarily_wins_over_analog_overrides() {
        let mut adc = Adc::new();
        adc.set_volts(0, 1.25);
        adc.set_header(0, Some(true));
        assert_eq!(adc.status().volts[0], VREF);
        adc.set_volts(0, 0.75);
        assert_eq!(adc.status().volts[0], VREF);
        let saved = adc.clone();
        adc.set_header(0, Some(false));
        assert_eq!(adc.status().volts[0], 0.0);
        adc.set_header(0, None);
        assert_eq!(adc.status().volts[0], 0.75);
        adc = saved;
        assert_eq!(adc.status().volts[0], VREF);
        adc.set_header(0, None);
        assert_eq!(adc.status().volts[0], 0.75);
    }
    #[test]
    fn manual_pipeline_channel_id_and_no_id_alignment() {
        let mut h = Host::new();
        assert_eq!(h.frame(0x0804), 0); // first result is unspecified, model zero
        for channel in 0..8 {
            h.frame(0x0804 | channel << 7);
            let code = if channel % 2 == 0 { 4095 } else { 0 };
            assert_eq!(h.frame(0), channel << 12 | code);
        }
        h.frame(0x0800);
        assert_eq!(h.frame(0), 0x7ff8); // leading zero, 12-bit result, three zeros
        assert_eq!(h.adc.status().frames, 19);
        assert_eq!(
            h.adc.status().codes,
            [
                Some(4095),
                Some(0),
                Some(4095),
                Some(0),
                Some(4095),
                Some(0),
                Some(4095),
                Some(0)
            ]
        );
    }
    #[test]
    fn pad_pulls_floating_defaults_overrides_and_clamping() {
        let mut h = Host::new();
        h.pins.ain = [0, 1, 3, 2, 1, 0, 0, 0];
        h.adc.set_default_volts(2, 1.65);
        h.adc.set_default_volts(3, 0.825);
        assert_eq!(
            h.adc
                .apply_volts("4=-1,5=4,6=1.1,7=2.2,8=1,0=NaN,bad,1=inf"),
            4
        );
        assert!(!h.adc.set_volts(0, f64::NAN));
        assert!(!h.adc.set_default_volts(8, 0.0));
        for (channel, expected) in [0, 4095, 2048, 1024, 0, 4095, 1365, 2730]
            .into_iter()
            .enumerate()
        {
            h.frame(0x0804 | (channel as u16) << 7);
            assert_eq!(h.frame(0) & 4095, expected);
        }
        // Voltage changes during a frame affect only the next CS sample.
        h.frame(0x0804);
        h.select();
        h.adc.set_volts(0, VREF);
        let mut out = 0;
        for _ in 0..16 {
            out = out << 1 | u16::from(h.bit(0));
        }
        h.end();
        assert_eq!(out, 0);
        assert_eq!(h.frame(0), 4095);
    }
    #[test]
    fn differential_registers_range_precedence_and_common_reference() {
        let mut h = Host::new();
        h.adc.set_volts(0, 0.825);
        h.adc.set_volts(1, 1.65);
        h.frame(0x9400); // BCH0/1
        h.frame(0x0884); // select odd member, same even-minus-odd pair
        assert_eq!(h.frame(0), 0x1c00); // -1024 in two's complement
        h.frame(0x9c00);
        h.frame(0x0804);
        assert_eq!(h.frame(0), 0x0e00); // +/-VREF range: -512
        h.frame(0x8c00);
        h.frame(0x0804);
        assert_eq!(h.frame(0), 0); // unipolar wins and clamps negative differential
        h.frame(0x8c04);
        h.frame(0x0804);
        assert_eq!(h.frame(0), 1024); // common REF- is board ground
        h.frame(0x8010);
        h.frame(0x0804);
        assert_eq!(h.frame(0), 0); // static partial shutdown suppresses conversions
        h.frame(0x8040);
        h.frame(0x0844); // register reset
        let s = h.adc.status();
        assert_eq!(
            (s.mode, s.configuration, s.unipolar, s.bipolar, s.range),
            (0x0800, 0, 0, 0, 0)
        );
        assert_eq!(s.volts[0], 0.825); // reset retains voltage sources
    }
    #[test]
    fn external_scans_and_configuration_echo() {
        let mut h = Host::new();
        h.frame(0x2104); // Standard_Ext, channels 0..2
        assert_eq!(
            [h.frame(0), h.frame(0), h.frame(0), h.frame(0)],
            [0x0fff, 0x1000, 0x2fff, 0x0fff]
        );
        h.frame(0x3304); // Upper_Ext, channels 6..7
        assert_eq!(
            [h.frame(0), h.frame(0), h.frame(0)],
            [0x6fff, 0x7000, 0x6fff]
        );
        h.frame(0xa848); // Custom Scan1, bits 0 and 3
        h.frame(0x4004);
        assert_eq!(
            [h.frame(0), h.frame(0), h.frame(0)],
            [0x0fff, 0x3000, 0x0fff]
        );
        h.frame(0x8004);
        assert_eq!(h.frame(0x0804), 0x8004);
        assert_eq!(h.frame(0), 0x0fff);
        h.frame(0x1004); // internal Repeat mode is explicitly unsupported
        assert_eq!(h.frame(0), 0);
        assert!(h.adc.status().unsupported_frames > 0);
    }
    #[test]
    fn short_unknown_and_extra_clocks_and_mid_frame_checkpoint() {
        let mut h = Host::new();
        h.select();
        for _ in 0..7 {
            h.bit(1);
        }
        h.end();
        assert_eq!(h.adc.status().frames, 0);
        h.frame(0x0804);
        h.select();
        for _ in 0..5 {
            h.bit(0);
        }
        let saved = h.adc.clone();
        let pins = h.pins;
        let mut tail = 0;
        for _ in 0..11 {
            tail = tail << 1 | u16::from(h.bit(0));
        }
        let status = h.adc.status();
        h.adc = saved;
        h.pins = pins;
        let mut again = 0;
        for _ in 0..11 {
            again = again << 1 | u16::from(h.bit(0));
        }
        assert_eq!(again, tail);
        assert_eq!(h.adc.status(), status);
        assert_eq!(h.bit(0), 3);
        h.end();
        let frames = h.adc.status().frames;
        h.select();
        h.bit(2);
        for _ in 0..15 {
            h.bit(0);
        }
        h.end();
        assert_eq!(h.adc.status().frames, frames);
        h.select();
        h.pins.clk = 2;
        h.feed();
        assert_eq!(h.adc.miso(), 3);
        h.end();
        h.frame(0x0884);
        assert_eq!(h.frame(0), 0x1000);
    }
}
