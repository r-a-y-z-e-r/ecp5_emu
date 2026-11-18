//! SSD1331 on the ULX3S, sampled at the package pins. No executor is required.
//! RAM (0, 0) is the top-left pixel with remap 0x72, the usual panel init.
//! See docs/PERIPHERALS_OLED.md for the digital rendering convention and limits.
use std::sync::Arc;

pub const WIDTH: usize = 96;
pub const HEIGHT: usize = 64;
pub const CLK_BALL: &str = crate::board::OLED_CLK;
pub const MOSI_BALL: &str = crate::board::OLED_MOSI;
pub const DC_BALL: &str = crate::board::OLED_DC;
pub const RESN_BALL: &str = crate::board::OLED_RESN;
pub const CSN_BALL: &str = crate::board::OLED_CSN;

/// 0 and 1 are levels, 2 is X and 3 is Z. All five pins are FPGA outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OledPins {
    pub csn: u8,
    pub dc: u8,
    pub clk: u8,
    pub mosi: u8,
    pub resn: u8,
}

/// Digital register values. Analogue drive settings are retained for inspection.
#[derive(Clone, Debug)]
pub struct OledRegisters {
    pub columns: [u8; 2],
    pub rows: [u8; 2],
    pub contrast: [u8; 3],
    pub master_current: u8,
    pub remap: u8,
    pub start_line: u8,
    pub offset: u8,
    pub mode: u8,
    pub multiplex: u8,
    pub master_configuration: u8,
    pub power_save: u8,
    pub phase: u8,
    pub clock: u8,
    pub second_precharge: [u8; 3],
    pub precharge_level: u8,
    pub vcomh: u8,
    pub fill: u8,
    pub dim_window: Option<[u8; 4]>,
    pub scroll: [u8; 5],
    pub scrolling: bool,
    pub display_on: bool,
}
impl Default for OledRegisters {
    fn default() -> Self {
        Self {
            columns: [0, 95],
            rows: [0, 63],
            contrast: [0x80; 3],
            master_current: 15,
            remap: 0x40,
            start_line: 0,
            offset: 0,
            mode: 0xa4,
            multiplex: 63,
            master_configuration: 0x8f,
            power_save: 0x1a,
            phase: 0x74,
            clock: 0xd0,
            second_precharge: [0x80; 3],
            precharge_level: 0x3e,
            vcomh: 0x3e,
            fill: 0,
            dim_window: None,
            // Assumption: no scroll setup is specified at reset; use zeros.
            scroll: [0; 5],
            scrolling: false,
            display_on: false,
        }
    }
}

/// An immutable UI snapshot. The RGB888 image is shared until RAM or mapping changes.
#[derive(Clone, Debug)]
pub struct OledStatus {
    pub image: Arc<Vec<u8>>,
    pub display_on: bool,
    pub frames: u64,
    pub crc32: u32,
    pub commands: u64,
    pub data_bytes: u64,
    pub errors: u64,
    pub unknown_commands: u64,
    pub version: u64,
    pub note: &'static str,
}

#[derive(Clone)]
pub struct Oled {
    registers: OledRegisters,
    ram: Vec<u16>,
    previous: Option<OledPins>,
    bits: u8,
    shift: u8,
    command: u8,
    args: Vec<u8>,
    needed: usize,
    pixel: [u8; 3],
    pixel_bytes: usize,
    x: u8,
    y: u8,
    crc: u32,
    frame_valid: bool,
    frames: u64,
    last_crc: u32,
    commands: u64,
    data_bytes: u64,
    errors: u64,
    unknown_commands: u64,
    version: u64,
    rendered_version: u64,
    image: Arc<Vec<u8>>,
}
impl Default for Oled {
    fn default() -> Self {
        Self::new()
    }
}
impl Oled {
    pub fn new() -> Self {
        Self {
            registers: OledRegisters::default(),
            ram: vec![0; WIDTH * HEIGHT],
            previous: None,
            bits: 0,
            shift: 0,
            command: 0,
            args: Vec::with_capacity(32),
            needed: 0,
            pixel: [0; 3],
            pixel_bytes: 0,
            x: 0,
            y: 0,
            crc: !0,
            frame_valid: true,
            frames: 0,
            last_crc: 0,
            commands: 0,
            data_bytes: 0,
            errors: 0,
            unknown_commands: 0,
            version: 0,
            rendered_version: 0,
            image: Arc::new(vec![0; WIDTH * HEIGHT * 3]),
        }
    }
    pub fn registers(&self) -> &OledRegisters {
        &self.registers
    }
    pub fn frames(&self) -> u64 {
        self.frames
    }
    pub fn last_frame_crc32(&self) -> u32 {
        self.last_crc
    }

    /// Samples in time order; repeated levels do nothing. Time is reserved for
    /// future timing diagnostics: this model imposes no minimum pulse width.
    pub fn feed(&mut self, _time_ps: u64, pins: OledPins) {
        let previous = self.previous.replace(pins);
        if previous == Some(pins) {
            return;
        }
        if pins.resn == 0 {
            if previous.is_none_or(|p| p.resn != 0) {
                self.reset();
            }
            return;
        }
        if pins.csn == 0
            && [pins.csn, pins.dc, pins.clk, pins.mosi, pins.resn]
                .iter()
                .any(|&v| v > 1)
        {
            self.errors += 1;
            self.bits = 0;
            self.pixel_bytes = 0;
            self.frame_valid = false;
            return;
        }
        if pins.csn != 0 {
            // Unknown CS cannot select the device. If a transfer was selected,
            // report the ambiguity and discard it, just as an unknown data pin.
            if pins.csn > 1 && previous.is_some_and(|p| p.csn == 0) {
                self.errors += 1;
                self.pixel_bytes = 0;
                self.frame_valid = false;
            }
            self.bits = 0;
            return;
        }
        if pins.resn != 1 {
            return;
        }
        if pins.clk == 1 && previous.is_some_and(|p| p.clk == 0) {
            self.shift = (self.shift << 1) | pins.mosi;
            self.bits += 1;
            if self.bits == 8 {
                self.bits = 0;
                self.byte(pins.dc, self.shift);
            }
        }
    }
    fn reset(&mut self) {
        self.registers = OledRegisters::default();
        self.bits = 0;
        self.shift = 0;
        self.needed = 0;
        self.args.clear();
        self.restart_window();
        self.version += 1;
        // RAM and diagnostic counters survive reset; frame CRC covers data only.
    }
    fn restart_window(&mut self) {
        self.x = self.registers.columns[0];
        self.y = self.registers.rows[0];
        self.pixel_bytes = 0;
        self.crc = !0;
        self.frame_valid = true;
    }
    fn byte(&mut self, dc: u8, byte: u8) {
        if dc == 1 {
            self.data_bytes += 1;
            if self.needed != 0 {
                // D/C changing before the argument list ends is malformed.
                self.errors += 1;
                self.needed = 0;
                self.args.clear();
            }
            self.data(byte);
        } else if self.needed != 0 {
            self.args.push(byte);
            if self.args.len() == self.needed {
                self.needed = 0;
                self.execute();
            }
        } else {
            self.commands += 1;
            self.command = byte;
            self.args.clear();
            self.needed = match byte {
                0x15 | 0x75 => 2,
                0x21 => 7,
                0x22 => 10,
                0x23 => 6,
                0x24 | 0x25 => 4,
                0x27 | 0xab => 5,
                0xb8 => 32,
                0x26
                | 0x81..=0x83
                | 0x87
                | 0x8a..=0x8c
                | 0xa0..=0xa2
                | 0xa8
                | 0xad
                | 0xb0
                | 0xb1
                | 0xb3
                | 0xbb
                | 0xbe
                | 0xfd => 1,
                _ => 0,
            };
            if self.needed == 0 {
                self.execute();
            }
        }
    }
    fn execute(&mut self) {
        let a = self.args.clone();
        let r = &mut self.registers;
        match self.command {
            0x15 | 0x75 => {
                let max = if self.command == 0x15 { 95 } else { 63 };
                let mask = if max == 95 { 127 } else { 63 };
                let window = [a[0] & mask, a[1] & mask];
                if window[0] > window[1] || window[1] > max {
                    // Assumption: reject unspecified out-of-range windows.
                    self.errors += 1;
                } else {
                    if self.command == 0x15 {
                        r.columns = window;
                    } else {
                        r.rows = window;
                    }
                    // Assumption: setting either window resets both axes.
                    self.restart_window();
                }
            }
            0x81..=0x83 => r.contrast[(self.command - 0x81) as usize] = a[0],
            0x87 => r.master_current = a[0] & 15,
            0x8a..=0x8c => r.second_precharge[(self.command - 0x8a) as usize] = a[0],
            0xa0 => {
                // Assumption: a new format begins at the window origin. This
                // also discards a partial pixel and starts a fresh frame CRC.
                r.remap = a[0];
                self.restart_window();
                self.version += 1;
            }
            0xa1 => {
                r.start_line = a[0] & 63;
                self.version += 1;
            }
            0xa2 => {
                r.offset = a[0] & 63;
                self.version += 1;
            }
            0xa4..=0xa7 => {
                r.mode = self.command;
                self.version += 1;
            }
            0xa8 => {
                if a[0] & 63 < 15 {
                    self.errors += 1;
                } else {
                    r.multiplex = a[0] & 63;
                    self.version += 1;
                }
            }
            0xad => r.master_configuration = a[0],
            0xae | 0xaf => {
                r.display_on = self.command == 0xaf;
                self.version += 1;
            }
            0xb0 => r.power_save = a[0],
            0xb1 => r.phase = a[0],
            0xb3 => r.clock = a[0],
            0xbb => r.precharge_level = a[0],
            0xbe => r.vcomh = a[0],
            0x26 => r.fill = a[0] & 0x11,
            0x27 => r.scroll.copy_from_slice(&a),
            0x2e | 0x2f => {
                r.scrolling = self.command == 0x2f;
            }
            0x21..=0x25 => self.graphic(&a),
            0xbc | 0xbd | 0xe3 => (),
            // Known but unimplemented commands consume their datasheet argument
            // count. Unrecognised opcodes have no recoverable argument length.
            _ => self.unknown_commands += 1,
        }
    }
    fn data(&mut self, byte: u8) {
        self.crc = crc_byte(self.crc, byte);
        self.pixel[self.pixel_bytes] = byte;
        self.pixel_bytes += 1;
        let format = self.registers.remap >> 6;
        let needed = match format {
            0 => 1,
            1 => 2,
            2 => 3,
            _ => {
                self.errors += 1;
                self.pixel_bytes = 0;
                self.frame_valid = false;
                return;
            }
        };
        if self.pixel_bytes != needed {
            return;
        }
        let p = self.pixel;
        let value = match format {
            0 => {
                // Figure 11 copies the most significant source bit into all
                // vacant low bits; this is not the usual cyclic bit replication.
                let red = p[0] >> 5;
                let green = (p[0] >> 2) & 7;
                let blue = p[0] & 3;
                ((u16::from(red) << 2 | if red & 4 != 0 { 3 } else { 0 }) << 11)
                    | ((u16::from(green) << 3 | if green & 4 != 0 { 7 } else { 0 }) << 5)
                    | (u16::from(blue) << 3 | if blue & 2 != 0 { 7 } else { 0 })
            }
            1 => u16::from_be_bytes([p[0], p[1]]),
            _ => graphic_colour(&p),
        };
        self.put(self.x as i32, self.y as i32, value);
        self.pixel_bytes = 0;
        let r = &self.registers;
        let wrapped = if r.remap & 1 == 0 {
            if self.x == r.columns[1] {
                self.x = r.columns[0];
                if self.y == r.rows[1] {
                    self.y = r.rows[0];
                    true
                } else {
                    self.y += 1;
                    false
                }
            } else {
                self.x += 1;
                false
            }
        } else if self.y == r.rows[1] {
            self.y = r.rows[0];
            if self.x == r.columns[1] {
                self.x = r.columns[0];
                true
            } else {
                self.x += 1;
                false
            }
        } else {
            self.y += 1;
            false
        };
        if wrapped {
            if self.frame_valid {
                self.frames += 1;
                self.last_crc = !self.crc;
            }
            self.crc = !0;
            self.frame_valid = true;
        }
    }
    fn put(&mut self, x: i32, y: i32, colour: u16) {
        if (0..96).contains(&x) && (0..64).contains(&y) {
            let cell = &mut self.ram[y as usize * WIDTH + x as usize];
            if *cell != colour {
                *cell = colour;
                self.version += 1;
            }
        }
    }
    fn line(&mut self, mut x: i32, mut y: i32, ex: i32, ey: i32, colour: u16) {
        // Assumption: integer Bresenham with both endpoints included. The
        // datasheet does not specify the accelerator's tie-breaking rule.
        let dx = (ex - x).abs();
        let sx = if x < ex { 1 } else { -1 };
        let dy = -(ey - y).abs();
        let sy = if y < ey { 1 } else { -1 };
        let mut error = dx + dy;
        loop {
            self.put(x, y, colour);
            if x == ex && y == ey {
                break;
            }
            let twice = 2 * error;
            if twice >= dy {
                error += dy;
                x += sx;
            }
            if twice <= dx {
                error += dx;
                y += sy;
            }
        }
    }
    fn graphic(&mut self, a: &[u8]) {
        let (x, y, ex, ey) = (
            (a[0] & 127) as i32,
            (a[1] & 63) as i32,
            (a[2] & 127) as i32,
            (a[3] & 63) as i32,
        );
        if x >= 96 || ex >= 96 || (self.command != 0x21 && (x > ex || y > ey)) {
            self.errors += 1;
            return;
        }
        match self.command {
            0x21 => self.line(x, y, ex, ey, graphic_colour(&a[4..])),
            0x22 => {
                for row in y..=ey {
                    for col in x..=ex {
                        if row == y || row == ey || col == x || col == ex {
                            self.put(col, row, graphic_colour(&a[4..7]));
                        } else if self.registers.fill & 1 != 0 {
                            self.put(col, row, graphic_colour(&a[7..10]));
                        }
                    }
                }
            }
            0x23 => {
                // Assumption: snapshot the source for overlapping copies. The
                // reverse-copy bit complements colours, not traversal order.
                let source = self.ram.clone();
                for row in y..=ey {
                    for col in x..=ex {
                        let mut colour = source[row as usize * WIDTH + col as usize];
                        if self.registers.fill & 16 != 0 {
                            colour = !colour;
                        }
                        // Assumption: clip a destination extending beyond RAM.
                        self.put(
                            (a[4] & 127) as i32 + col - x,
                            (a[5] & 63) as i32 + row - y,
                            colour,
                        );
                    }
                }
            }
            0x24 => {
                self.registers.dim_window = Some([x as u8, y as u8, ex as u8, ey as u8]);
                for row in y..=ey {
                    for col in x..=ex {
                        let p = self.ram[row as usize * WIDTH + col as usize];
                        // Assumption: five-bit channels use six-bit gray scale with a zero
                        // low bit. Dim changes RAM once, so applying it is idempotent.
                        let dim = |v: u16| if v >= 16 { v >> 2 } else { v };
                        let red = dim((p >> 11) * 2) / 2;
                        let green = dim((p >> 5) & 63);
                        let blue = dim((p & 31) * 2) / 2;
                        self.put(col, row, red << 11 | green << 5 | blue);
                    }
                }
            }
            0x25 => {
                for row in y..=ey {
                    for col in x..=ex {
                        self.put(col, row, 0);
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    /// Lazily renders at UI cadence, rather than allocating an image per SPI bit.
    pub fn status(&mut self) -> OledStatus {
        if self.rendered_version != self.version {
            self.image = Arc::new(self.render());
            self.rendered_version = self.version;
        }
        OledStatus {
            image: Arc::clone(&self.image),
            display_on: self.registers.display_on,
            frames: self.frames,
            crc32: self.last_crc,
            commands: self.commands,
            data_bytes: self.data_bytes,
            errors: self.errors,
            unknown_commands: self.unknown_commands,
            version: self.version,
            note: if !self.registers.display_on {
                "display off"
            } else if self.registers.scrolling {
                "scroll not rendered"
            } else if self.unknown_commands != 0 {
                "unsupported commands"
            } else if self.data_bytes == 0 {
                "no pixel data yet"
            } else {
                "display on"
            },
        }
    }
    fn render(&self) -> Vec<u8> {
        let mut image = vec![0; WIDTH * HEIGHT * 3];
        let r = &self.registers;
        if !r.display_on {
            return image;
        }
        for y in 0..HEIGHT {
            // Assumption: fixed panel wiring follows 0x72, with reverse
            // scan and split odd/even. Alternative maps are not board-measured.
            // Convert its row to a physical pad, then decode the current map.
            let baseline_com = 63 - y;
            let mut pad = baseline_com / 2 + (baseline_com % 2) * 32;
            if r.remap & 8 != 0 {
                pad ^= 32;
            }
            let com = if r.remap & 32 != 0 {
                (pad % 32) * 2 + pad / 32
            } else {
                pad
            };
            let scan = (com + 64 - r.offset as usize) % 64;
            let n = r.multiplex as usize + 1;
            if scan >= n {
                continue;
            }
            // Table 14: reverse scan reverses the RAM row including start line.
            let row = if r.remap & 16 != 0 {
                (n - 1 - scan + r.start_line as usize) % 64
            } else {
                (scan + r.start_line as usize) % 64
            };
            for x in 0..WIDTH {
                let col = if r.remap & 2 != 0 { x } else { WIDTH - 1 - x };
                let mut p = self.ram[row * WIDTH + col];
                p = match r.mode {
                    0xa5 => 0xffff,
                    0xa6 => 0,
                    0xa7 => !p,
                    _ => p,
                };
                let mut rgb = rgb888(p);
                if r.remap & 4 != 0 {
                    rgb.swap(0, 2);
                }
                let at = (y * WIDTH + x) * 3;
                image[at..at + 3].copy_from_slice(&rgb);
            }
        }
        image
    }
}
fn graphic_colour(a: &[u8]) -> u16 {
    (u16::from(a[0] & 62) << 10) | (u16::from(a[1] & 63) << 5) | u16::from((a[2] & 62) >> 1)
}
fn rgb888(p: u16) -> [u8; 3] {
    let r = (p >> 11) as u8;
    let g = ((p >> 5) & 63) as u8;
    let b = (p & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}
/// Reflected CRC-32/ISO-HDLC, initial/final XOR 0xffffffff, polynomial 0x04c11db7.
fn crc_byte(mut crc: u32, byte: u8) -> u32 {
    crc ^= u32::from(byte);
    for _ in 0..8 {
        crc = (crc >> 1) ^ if crc & 1 != 0 { 0xedb8_8320 } else { 0 };
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    const INIT: &[u8] = &[
        0xae, 0xa0, 0x72, 0xa1, 0, 0xa2, 0, 0xa4, 0xa8, 0x3f, 0xad, 0x8e, 0xb0, 0x0b, 0xb1, 0x31,
        0xb3, 0xf0, 0x8a, 0x64, 0x8b, 0x78, 0x8c, 0x64, 0xbb, 0x3a, 0xbe, 0x3e, 0x87, 6, 0x81,
        0x91, 0x82, 0x50, 0x83, 0x7d, 0xaf,
    ];
    struct Spi {
        oled: Oled,
        pins: OledPins,
        time: u64,
    }
    impl Spi {
        fn new() -> Self {
            let mut s = Self {
                oled: Oled::new(),
                pins: OledPins {
                    csn: 1,
                    dc: 0,
                    clk: 0,
                    mosi: 0,
                    resn: 1,
                },
                time: 0,
            };
            s.sample();
            s
        }
        fn sample(&mut self) {
            self.time += 10;
            self.oled.feed(self.time, self.pins);
        }
        fn bit(&mut self, bit: u8) {
            self.pins.clk = 0;
            self.pins.mosi = bit;
            self.sample();
            self.pins.clk = 1;
            self.sample();
            self.pins.clk = 0;
            self.sample();
        }
        fn bytes(&mut self, dc: u8, bytes: &[u8]) {
            for &b in bytes {
                self.pins.csn = 0;
                self.pins.dc = dc;
                self.sample();
                for bit in (0..8).rev() {
                    self.bit(b >> bit & 1);
                }
                self.pins.csn = 1;
                self.sample();
            }
        }
        fn cmd(&mut self, bytes: &[u8]) {
            self.bytes(0, bytes);
        }
        fn init(&mut self) {
            self.cmd(INIT);
        }
        fn window(&mut self, x: u8, ex: u8, y: u8, ey: u8) {
            self.cmd(&[0x15, x, ex, 0x75, y, ey]);
        }
        fn pixel(&mut self, p: u16) {
            self.bytes(1, &p.to_be_bytes());
        }
        fn at(&mut self, x: usize, y: usize) -> [u8; 3] {
            self.oled.status().image[(y * WIDTH + x) * 3..(y * WIDTH + x) * 3 + 3]
                .try_into()
                .unwrap()
        }
    }
    #[test]
    fn common_init_full_image_and_crc() {
        let mut s = Spi::new();
        s.init();
        s.window(0, 95, 0, 63);
        let mut expected = Vec::new();
        for y in 0..64u16 {
            for x in 0..96u16 {
                let (r, g, b) = (x % 32, y, (x + y) % 32);
                s.pixel(r << 11 | g << 5 | b);
                expected.extend_from_slice(&[
                    (r * 8 + r / 4) as u8,
                    (g * 4 + g / 16) as u8,
                    (b * 8 + b / 4) as u8,
                ]);
            }
        }
        let st = s.oled.status();
        assert_eq!(&*st.image, &expected);
        assert!(st.display_on);
        assert_eq!(st.frames, 1);
        // Independently calculated with Python zlib.crc32 over the image bytes.
        assert_eq!(st.crc32, 0x7cc39cfa);
        assert_eq!(
            (st.data_bytes, st.errors, st.unknown_commands),
            (12288, 0, 0)
        );
        let again = s.oled.status();
        assert!(Arc::ptr_eq(&st.image, &again.image));
        assert_eq!(INIT.last(), Some(&0xaf));
        assert_eq!(s.oled.registers().contrast, [0x91, 0x50, 0x7d]);
        assert_eq!(s.oled.registers().second_precharge, [0x64, 0x78, 0x64]);
        let crc = b"123456789".iter().fold(!0, |c, &b| crc_byte(c, b));
        assert_eq!(!crc, 0xcbf4_3926);
    }
    #[test]
    fn colour_256_and_format_two() {
        let mut s = Spi::new();
        s.init();
        s.cmd(&[0xa0, 0x32]);
        s.window(0, 3, 0, 0);
        s.bytes(1, &[0xe0, 0x1c, 0x03, 0x49]);
        assert_eq!(s.at(0, 0), [255, 0, 0]);
        assert_eq!(s.at(1, 0), [0, 255, 0]);
        assert_eq!(s.at(2, 0), [0, 0, 255]);
        // 010 010 01 -> RAM R=8, G=16, B=8, as figure 11 specifies.
        assert_eq!(s.at(3, 0), [66, 65, 66]);
        assert_eq!(s.oled.frames(), 1);
        s.cmd(&[0xa0, 0xb2]);
        s.window(0, 0, 0, 0);
        s.bytes(1, &[62, 32, 16]);
        assert_eq!(s.at(0, 0), [255, 130, 66]);
        s.cmd(&[0xa0, 0xb6]);
        assert_eq!(s.at(0, 0), [66, 130, 255]);
    }
    #[test]
    fn partial_window_wrap_and_vertical_increment() {
        for vertical in [false, true] {
            let mut s = Spi::new();
            s.init();
            s.cmd(&[0xa0, 0x72 | u8::from(vertical)]);
            s.window(2, 3, 4, 5);
            for p in [0xf800, 0x07e0, 0x001f, 0xffff] {
                s.pixel(p);
            }
            assert_eq!(s.oled.frames(), 1);
            assert_eq!(s.at(2, 4), [255, 0, 0]);
            assert_eq!(s.at(3, 5), [255; 3]);
            assert_eq!(
                s.at(if vertical { 2 } else { 3 }, if vertical { 5 } else { 4 }),
                [0, 255, 0]
            );
            s.pixel(0);
            assert_eq!(s.at(2, 4), [0; 3]);
            assert_eq!(s.oled.frames(), 1);
            assert_eq!(s.at(1, 4), [0; 3]);
        }
    }
    #[test]
    fn remap_flips_split_start_offset_mux_and_display_modes() {
        let mut s = Spi::new();
        s.init();
        s.window(0, 0, 0, 0);
        s.pixel(0xf800);
        s.window(0, 0, 1, 1);
        s.pixel(0x07e0);
        assert_eq!(s.at(0, 0), [255, 0, 0]);
        s.cmd(&[0xa0, 0x70]);
        assert_eq!(s.at(95, 0), [255, 0, 0]);
        assert_eq!(s.at(0, 0), [0; 3]);
        s.cmd(&[0xa0, 0x62]);
        assert_eq!(s.at(0, 63), [255, 0, 0]);
        s.cmd(&[0xa0, 0x52]);
        assert_eq!(s.at(0, 0), [255, 0, 0]);
        assert_eq!(s.at(0, 2), [0, 255, 0]);
        s.cmd(&[0xa0, 0x72, 0xa1, 1]);
        assert_eq!(s.at(0, 63), [255, 0, 0]);
        s.cmd(&[0xa1, 0, 0xa2, 1]);
        assert_eq!(s.at(0, 63), [255, 0, 0]);
        s.cmd(&[0xa2, 0, 0xa5]);
        assert_eq!(s.at(50, 50), [255; 3]);
        s.cmd(&[0xa8, 31]);
        assert_eq!(s.at(50, 0), [0; 3]);
        assert_eq!(s.at(50, 63), [255; 3]);
        s.cmd(&[0xa8, 63, 0xa6]);
        assert_eq!(s.at(0, 0), [0; 3]);
        s.cmd(&[0xa7]);
        assert_eq!(s.at(0, 0), [0, 255, 255]);
        s.cmd(&[0xae]);
        assert_eq!(s.at(0, 0), [0; 3]);
    }
    #[test]
    fn drawing_fill_line_clear_copy_and_dim() {
        let mut s = Spi::new();
        s.init();
        s.cmd(&[0x22, 2, 2, 6, 6, 62, 0, 0, 0, 63, 0]);
        assert_eq!(s.at(2, 2), [255, 0, 0]);
        assert_eq!(s.at(3, 3), [0; 3]);
        s.cmd(&[0x26, 1, 0x22, 2, 2, 6, 6, 62, 0, 0, 0, 63, 0]);
        assert_eq!(s.at(3, 3), [0, 255, 0]);
        s.cmd(&[0x21, 0, 0, 3, 3, 0, 0, 62]);
        for i in 0..=3 {
            assert_eq!(s.at(i, i), [0, 0, 255]);
        }
        s.cmd(&[0x23, 2, 2, 6, 6, 10, 10]);
        assert_eq!(s.at(11, 11), [0, 0, 255]);
        s.cmd(&[0x25, 2, 2, 6, 6]);
        assert_eq!(s.at(3, 3), [0; 3]);
        assert_eq!(s.at(11, 11), [0, 0, 255]);
        s.cmd(&[0x24, 10, 10, 14, 14]);
        assert_eq!(s.at(11, 11), [0, 0, 57]);
        let before = s.oled.status().image;
        s.cmd(&[0x24, 10, 10, 14, 14]);
        assert_eq!(s.oled.status().image, before);
        s.cmd(&[0x26, 16, 0x23, 10, 10, 14, 14, 10, 10]);
        assert_eq!(s.at(11, 11), [255, 255, 198]);
        assert_eq!(s.oled.frames(), 0);
    }
    #[test]
    fn spi_abort_reset_unknown_and_dc_at_eighth_bit() {
        let mut s = Spi::new();
        s.init();
        s.pins.csn = 0;
        s.sample();
        for _ in 0..4 {
            s.bit(1);
        }
        s.pins.csn = 1;
        s.sample();
        s.cmd(&[0xae]);
        assert!(!s.oled.status().display_on);
        // D/C is sampled on bit eight, rather than at byte start.
        s.pins.dc = 1;
        s.pins.csn = 0;
        s.sample();
        for bit in (1..8).rev() {
            s.bit(0xaf >> bit & 1);
        }
        s.pins.dc = 0;
        s.bit(1);
        assert!(s.oled.status().display_on);
        s.pins.csn = 1;
        s.sample();
        s.window(0, 0, 0, 0);
        s.pixel(0xf800);
        s.cmd(&[0x22, 1, 2]); // reset clears this incomplete command
        s.pins.resn = 0;
        s.sample();
        s.pins.resn = 1;
        s.sample();
        assert!(!s.oled.status().display_on);
        assert_eq!(s.oled.ram[0], 0xf800);
        assert_eq!(s.oled.registers().remap, 0x40);
        assert_eq!(s.oled.registers().phase, 0x74);
        s.cmd(&[0xa0, 0x72, 0xaf]);
        assert_eq!(s.at(0, 0), [255, 0, 0]);
        for field in 0..5 {
            let mut t = Spi::new();
            t.pins.csn = 0;
            t.sample();
            t.bit(1);
            match field {
                0 => t.pins.mosi = 2,
                1 => t.pins.clk = 3,
                2 => t.pins.dc = 2,
                3 => t.pins.resn = 3,
                _ => t.pins.csn = 2,
            }
            t.sample();
            t.sample();
            assert_eq!(t.oled.status().errors, 1);
            t.pins = OledPins {
                csn: 1,
                dc: 0,
                clk: 0,
                mosi: 0,
                resn: 1,
            };
            t.sample();
            t.cmd(&[0xaf]);
            assert!(t.oled.status().display_on);
        }
    }
    #[test]
    fn unsupported_commands_keep_known_argument_lengths_and_scroll_is_reported() {
        let mut s = Spi::new();
        s.cmd(&[
            0xab, 0, 0xaf, 0xaf, 0xaf, 0, 0xaf, 0x27, 1, 2, 3, 4, 0, 0x2f,
        ]);
        let st = s.oled.status();
        assert!(st.display_on);
        assert_eq!(st.unknown_commands, 1);
        assert_eq!(st.note, "scroll not rendered");
        assert_eq!(s.oled.registers().scroll, [1, 2, 3, 4, 0]);
        s.cmd(&[0x2e, 0xff, 0xae]);
        assert!(!s.oled.status().display_on);
        assert_eq!(s.oled.status().unknown_commands, 2);
    }
}
