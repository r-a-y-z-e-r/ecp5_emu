//! MCP7940N on the ULX3S I2C bus. Pad-fed, with time in picoseconds.
//! SDA only pulls low; the board's pull-up supplies a released high level.
//! Register semantics follow Microchip DS20005010J, sections 5 and 6.

pub const ADDRESS: u8 = 0x6f;
pub const SECOND_PS: u64 = 1_000_000_000_000;
pub const CAPACITY: usize = 96;

/// Resolved pad levels: 0/1, X=2, Z=3. Z resolves through the board pull-up.
#[derive(Clone, Copy)]
pub struct Pins {
    pub scl: u8,
    pub sda: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    /// Calendar year (2000..2099), month, date, hour (24-hour), minute, second.
    pub time: [u16; 6],
    pub st: bool,
    pub oscrun: bool,
    pub active: bool,
    /// Last seven-bit bus address, including addresses we did not answer.
    pub last_bus_address: Option<u8>,
    pub last_read: bool,
    pub last_address: Option<u8>,
    pub bytes_read: u64,
    /// Data writes only; setting the pointer is not a register write.
    pub bytes_written: u64,
    /// Rejected addresses/pointers/data and the master's terminating NACKs.
    pub nacks: u64,
}

#[derive(Clone, Copy)]
enum Receive {
    Address,
    Pointer,
    Data,
}
#[derive(Clone, Copy)]
enum Next {
    Receive(Receive),
    Send,
    Wait,
}
#[derive(Clone, Copy)]
enum Phase {
    Wait,
    Receive(Receive, u8, u8),
    Ack(Next, bool, u8),
    Send(u8, u8),
    HostAck,
    NextByte,
}

#[derive(Clone)]
pub struct Rtc {
    registers: [u8; CAPACITY],
    buffered_time: [u8; 7],
    pointer: u8,
    phase: Phase,
    previous: Pins,
    sda: Option<bool>,
    status: Status,
    time_ps: u64,
    fraction_ps: u64,
}

fn decimal(bcd: u8) -> u8 {
    (bcd >> 4) * 10 + (bcd & 15)
}
fn bcd(value: u8) -> u8 {
    (value / 10) * 16 + value % 10
}
fn hour(value: u8) -> u8 {
    if value & 0x40 != 0 {
        decimal(value & 0x1f) % 12 + if value & 0x20 != 0 { 12 } else { 0 }
    } else {
        decimal(value & 0x3f)
    }
}
fn mask(address: usize) -> u8 {
    match address {
        0x01 | 0x02 | 0x0a..=0x0c | 0x11..=0x13 | 0x18..=0x19 | 0x1c..=0x1d => 0x7f,
        0x03 | 0x04 | 0x05 | 0x0e | 0x15 | 0x1a | 0x1e => 0x3f,
        0x0f | 0x16 => 0x1f,
        _ => 0xff,
    }
}

impl Default for Rtc {
    fn default() -> Self {
        let mut image = [0; CAPACITY];
        image[..7].copy_from_slice(&[0x80, 0, 0, 5, 1, 1, 0x26]);
        Self::new(image)
    }
}

impl Rtc {
    /// Load 0x00..0x5F. Unimplemented bits read zero; reserved bytes retain
    /// the image's values, following the HR09 board read-back. Read-only
    /// fields are derived from ST, year, hour format and alarm polarity.
    pub fn new(mut image: [u8; CAPACITY]) -> Self {
        for (i, byte) in image.iter_mut().enumerate() {
            *byte &= mask(i);
        }
        let mut rtc = Self {
            registers: image,
            buffered_time: [0; 7],
            pointer: 0,
            phase: Phase::Wait,
            previous: Pins { scl: 1, sda: 1 },
            sda: None,
            status: Status {
                time: [0; 6],
                st: false,
                oscrun: false,
                active: false,
                last_bus_address: None,
                last_read: false,
                last_address: None,
                bytes_read: 0,
                bytes_written: 0,
                nacks: 0,
            },
            time_ps: 0,
            fraction_ps: 0,
        };
        rtc.derived();
        rtc.buffer_time();
        rtc
    }

    /// ECP5_RTC_IMAGE must contain exactly 96 bytes; otherwise use the
    /// deterministic running clock at 2026-01-01 00:00:00 and zero SRAM.
    pub fn from_env() -> Self {
        match std::env::var_os("ECP5_RTC_IMAGE").map(std::fs::read) {
            Some(Ok(bytes)) => match bytes.try_into() {
                Ok(image) => Self::new(image),
                Err(_) => {
                    eprintln!("RTC: ECP5_RTC_IMAGE needs 96 bytes; default clock instead");
                    Self::default()
                }
            },
            Some(Err(e)) => {
                eprintln!("RTC: ECP5_RTC_IMAGE not read ({e}); default clock instead");
                Self::default()
            }
            None => Self::default(),
        }
    }
    pub fn image(&self) -> [u8; CAPACITY] {
        self.registers
    }
    pub fn status(&self) -> Status {
        self.status.clone()
    }
    pub fn active(&self) -> bool {
        self.status.active
    }
    pub fn sda(&self) -> Option<bool> {
        self.sda
    }

    fn derived(&mut self) {
        let r = &mut self.registers;
        let st = r[0] & 0x80 != 0;
        r[3] = (r[3] & !0x20) | if st { 0x20 } else { 0 };
        r[5] = (r[5] & !0x20)
            | if decimal(r[6]).is_multiple_of(4) {
                0x20
            } else {
                0
            };
        for a in [0x0c, 0x13] {
            r[a] = (r[a] & !0x40) | (r[2] & 0x40);
        }
        r[0x14] = (r[0x14] & 0x7f) | (r[0x0d] & 0x80);
        self.status.st = st;
        self.status.oscrun = st;
        self.status.time = [
            2000 + u16::from(decimal(r[6])),
            u16::from(decimal(r[5] & 0x1f)),
            u16::from(decimal(r[4])),
            u16::from(hour(r[2])),
            u16::from(decimal(r[1])),
            u16::from(decimal(r[0] & 0x7f)),
        ];
    }
    fn buffer_time(&mut self) {
        self.buffered_time.copy_from_slice(&self.registers[..7]);
    }
    fn increment_pointer(&mut self) {
        self.pointer = match self.pointer {
            0x1f => 0,
            0x5f => 0x20,
            p => p.wrapping_add(1),
        };
        if self.pointer == 0 {
            self.buffer_time();
        }
    }
    fn alarms(&mut self) {
        for a in [0x0a, 0x11] {
            let r = &self.registers;
            let equal = [
                r[0] & 0x7f == r[a],
                r[1] == r[a + 1],
                hour(r[2]) == hour(r[a + 2]),
                r[3] & 7 == r[a + 3] & 7,
                r[4] == r[a + 4],
                r[5] & 0x1f == r[a + 5],
            ];
            let matches = match (r[a + 3] >> 4) & 7 {
                n @ 0..=4 => equal[n as usize],
                7 => equal.into_iter().all(|v| v),
                _ => false,
            };
            if matches {
                self.registers[a + 3] |= 8;
            }
        }
    }
    fn tick(&mut self) {
        let r = &mut self.registers;
        let sec = decimal(r[0] & 0x7f) + 1;
        r[0] = 0x80 | bcd(sec % 60);
        if sec >= 60 {
            let min = decimal(r[1]) + 1;
            r[1] = bcd(min % 60);
            if min >= 60 {
                let h = (hour(r[2]) + 1) % 24;
                r[2] = if r[2] & 0x40 != 0 {
                    0x40 | if h >= 12 { 0x20 } else { 0 }
                        | bcd(if h.is_multiple_of(12) { 12 } else { h % 12 })
                } else {
                    bcd(h)
                };
                if h == 0 {
                    r[3] = (r[3] & !7) | ((r[3] & 7) % 7 + 1);
                    let month = decimal(r[5] & 0x1f);
                    let days = match month {
                        4 | 6 | 9 | 11 => 30,
                        2 => {
                            if decimal(r[6]).is_multiple_of(4) {
                                29
                            } else {
                                28
                            }
                        }
                        _ => 31,
                    };
                    let date = decimal(r[4]) + 1;
                    r[4] = bcd(if date > days { 1 } else { date });
                    if date > days {
                        r[5] = bcd(if month >= 12 { 1 } else { month + 1 });
                        if month >= 12 {
                            r[6] = bcd((decimal(r[6]) + 1) % 100);
                        }
                    }
                }
            }
        }
        self.derived();
        self.alarms();
    }
    fn advance(&mut self, time_ps: u64) {
        let delta = time_ps.saturating_sub(self.time_ps);
        self.time_ps = time_ps;
        if self.status.st {
            let fraction = self.fraction_ps + delta % SECOND_PS;
            self.fraction_ps = fraction % SECOND_PS;
            for _ in 0..delta / SECOND_PS + fraction / SECOND_PS {
                self.tick();
            }
        }
    }
    fn write(&mut self, value: u8) {
        let a = self.pointer as usize;
        let old = self.registers[a];
        let writable = match a {
            // Assumption: writes to reserved bytes are ignored. The datasheet
            // only says reserved; HR09 checks read-back, never writes.
            0x09 | 0x10 | 0x17 => 0,
            3 => 0x1f,
            5 => 0x1f,
            0x0c | 0x13 => 0x3f,
            0x0d => 0xf7,
            0x14 => 0x77,
            _ => mask(a),
        };
        self.registers[a] = (old & !writable) | (value & writable);
        if a == 0 {
            self.fraction_ps = 0;
        }
        if a == 3 && old & 0x10 != 0 && value & 0x10 == 0 {
            self.registers[0x18..0x20].fill(0);
        }
        if a == 0x0d || a == 0x14 {
            self.registers[a] &= !8;
        }
        self.status.last_address = Some(self.pointer);
        self.status.bytes_written += 1;
        self.derived();
        self.alarms();
        self.increment_pointer();
    }
    fn received(&mut self, kind: Receive, byte: u8) {
        let (next, ack) = match kind {
            Receive::Address => {
                self.status.last_bus_address = Some(byte >> 1);
                self.status.last_read = byte & 1 != 0;
                if byte >> 1 != ADDRESS {
                    (Next::Wait, false)
                } else if byte & 1 == 0 {
                    (Next::Receive(Receive::Pointer), true)
                } else if self.pointer < CAPACITY as u8 {
                    if self.pointer < 0x20 {
                        self.buffer_time();
                    }
                    (Next::Send, true)
                } else {
                    (Next::Wait, false)
                }
            }
            Receive::Pointer => {
                self.pointer = byte;
                (Next::Receive(Receive::Data), byte < CAPACITY as u8)
            }
            Receive::Data => {
                if self.pointer < CAPACITY as u8 {
                    self.write(byte);
                    (Next::Receive(Receive::Data), true)
                } else {
                    (Next::Wait, false)
                }
            }
        };
        if !ack {
            self.status.nacks += 1;
        }
        self.phase = Phase::Ack(next, ack, 0);
    }
    fn send(&mut self) {
        let a = self.pointer as usize;
        let byte = if a < 7 {
            self.buffered_time[a]
        } else {
            self.registers[a]
        };
        self.sda = (byte & 0x80 == 0).then_some(false);
        self.phase = Phase::Send(byte, 0);
    }

    /// START/STOP are SDA transitions while SCL stays high. Receive on
    /// rising SCL and answer on falling SCL, including the ninth ACK bit.
    /// Unknown pads abort until another START; there is no clock stretching.
    pub fn feed(&mut self, time_ps: u64, pins: Pins) {
        self.advance(time_ps);
        let resolve = |v| if v == 3 { 1 } else { v };
        let pins = Pins {
            scl: resolve(pins.scl),
            sda: resolve(pins.sda),
        };
        let old = self.previous;
        self.previous = pins;
        if pins.scl > 1 || pins.sda > 1 {
            self.phase = Phase::Wait;
            self.sda = None;
            self.status.active = false;
            return;
        }
        if old.scl == 1 && pins.scl == 1 && old.sda <= 1 && old.sda != pins.sda {
            self.sda = None;
            self.status.active = pins.sda == 0;
            self.phase = if pins.sda == 0 {
                Phase::Receive(Receive::Address, 0, 0)
            } else {
                Phase::Wait
            };
        } else if old.scl == 0 && pins.scl == 1 {
            match self.phase {
                Phase::Receive(kind, byte, n) => {
                    let byte = (byte << 1) | pins.sda;
                    if n == 7 {
                        self.received(kind, byte);
                    } else {
                        self.phase = Phase::Receive(kind, byte, n + 1);
                    }
                }
                Phase::Ack(next, ack, 1) => self.phase = Phase::Ack(next, ack, 2),
                Phase::Send(byte, n) => {
                    self.phase = Phase::Send(byte, n + 1);
                    if n == 7 {
                        self.status.last_address = Some(self.pointer);
                        self.status.bytes_read += 1;
                        self.increment_pointer();
                    }
                }
                Phase::HostAck => {
                    self.phase = if pins.sda == 0 {
                        Phase::NextByte
                    } else {
                        self.status.nacks += 1;
                        Phase::Wait
                    };
                }
                _ => {}
            }
        } else if old.scl == 1 && pins.scl == 0 {
            match self.phase {
                Phase::Ack(next, ack, 0) => {
                    self.sda = ack.then_some(false);
                    self.phase = Phase::Ack(next, ack, 1);
                }
                Phase::Ack(next, _, 2) => {
                    self.sda = None;
                    match next {
                        Next::Receive(kind) => self.phase = Phase::Receive(kind, 0, 0),
                        Next::Send => self.send(),
                        Next::Wait => self.phase = Phase::Wait,
                    }
                }
                Phase::Send(_, 8) => {
                    self.sda = None;
                    self.phase = Phase::HostAck;
                }
                Phase::Send(byte, n) => self.sda = (byte & (0x80 >> n) == 0).then_some(false),
                Phase::NextByte => self.send(),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Resolve the master's drain and the RTC drain, and feed back answers
    // at the same SCL level as the runner's 1 ps input update.
    struct Host {
        rtc: Rtc,
        scl: u8,
        low: bool,
        time: u64,
    }
    impl Host {
        fn new(rtc: Rtc) -> Self {
            Self {
                rtc,
                scl: 1,
                low: false,
                time: 0,
            }
        }
        fn line(&self) -> u8 {
            u8::from(!self.low && self.rtc.sda().is_none())
        }
        fn feed(&mut self) {
            self.time += 1_000;
            self.rtc.feed(
                self.time,
                Pins {
                    scl: self.scl,
                    sda: self.line(),
                },
            );
            self.rtc.feed(
                self.time + 1,
                Pins {
                    scl: self.scl,
                    sda: self.line(),
                },
            );
        }
        fn set(&mut self, scl: u8, low: bool) {
            self.scl = scl;
            self.low = low;
            self.feed();
        }
        fn start(&mut self) {
            self.set(0, false);
            self.set(1, false);
            self.set(1, true);
            self.set(0, true);
        }
        fn stop(&mut self) {
            self.set(0, true);
            self.set(1, true);
            self.set(1, false);
        }
        fn write(&mut self, byte: u8) -> bool {
            for bit in (0..8).rev() {
                self.set(0, byte & (1 << bit) == 0);
                self.set(1, self.low);
            }
            self.set(0, false);
            self.set(1, false);
            let ack = self.line() == 0;
            self.set(0, false);
            ack
        }
        fn read(&mut self, ack: bool) -> u8 {
            self.read_tail(0, 8, ack)
        }
        fn read_tail(&mut self, mut byte: u8, bits: usize, ack: bool) -> u8 {
            for _ in 0..bits {
                self.set(1, false);
                byte = (byte << 1) | self.line();
                self.set(0, false);
            }
            self.set(0, ack);
            self.set(1, ack);
            self.set(0, false);
            byte
        }
        fn pointer(&mut self, p: u8) {
            self.start();
            assert!(self.write(0xde));
            assert!(self.write(p));
        }
        fn random(&mut self, p: u8, n: usize) -> Vec<u8> {
            self.pointer(p);
            self.start();
            assert!(self.write(0xdf));
            let bytes = (0..n).map(|i| self.read(i + 1 < n)).collect();
            self.stop();
            bytes
        }
        fn store(&mut self, p: u8, bytes: &[u8]) {
            self.pointer(p);
            for &b in bytes {
                assert!(self.write(b));
            }
            self.stop();
        }
        fn seconds(&mut self, n: u64) {
            self.time += n * SECOND_PS;
            self.feed();
        }
    }
    #[test]
    fn framing_pointer_repeated_start_sequential_and_current_read() {
        let mut h = Host::new(Rtc::default());
        assert_eq!(h.random(0, 7), [0x80, 0, 0, 0x25, 1, 1, 0x26]);
        assert_eq!(h.rtc.status().bytes_written, 0);
        assert_eq!(h.rtc.status().bytes_read, 7);
        assert_eq!(h.rtc.status().nacks, 1);
        assert!(!h.rtc.active());
        assert_eq!(h.rtc.sda(), None);
        h.store(0x20, &[0x12, 0xa5, 0x5a]);
        h.pointer(0x20);
        h.stop();
        h.start();
        assert!(h.write(0xdf));
        assert_eq!(h.read(false), 0x12);
        h.stop();
        h.start();
        assert!(h.write(0xdf));
        assert_eq!(h.read(false), 0xa5);
        h.stop();
        assert_eq!(h.random(0x20, 3), [0x12, 0xa5, 0x5a]);
        h.store(0x5f, &[0x99, 0x88]);
        assert_eq!(h.random(0x5f, 2), [0x99, 0x88]);
        assert_eq!(h.random(0x1f, 2), [0, 0x80]);
    }
    #[test]
    fn address_and_invalid_pointer_nacks_and_partial_start_stop() {
        let mut h = Host::new(Rtc::default());
        for control in [0xa0, 0xa1, 0xdc, 0xdd] {
            h.start();
            assert!(!h.write(control));
            assert_eq!(h.rtc.sda(), None);
            h.stop();
        }
        h.start();
        assert!(h.write(0xde));
        assert!(!h.write(0x60));
        assert!(!h.write(0x55));
        h.stop();
        h.set(1, true);
        h.set(0, true);
        h.set(1, false);
        h.stop();
        assert_eq!(h.random(0, 1), [0x80]);
        h.start();
        assert!(h.write(0xdf));
        h.rtc.feed(h.time + 2, Pins { scl: 2, sda: 1 });
        assert_eq!(h.rtc.sda(), None);
        assert!(!h.rtc.active());
        h.rtc.feed(h.time + 3, Pins { scl: 1, sda: 2 });
        h.rtc.feed(h.time + 4, Pins { scl: 1, sda: 0 });
        assert!(!h.rtc.active());
        assert_eq!(h.random(0, 1), [0x80]);
    }
    #[test]
    fn writes_set_time_freeze_restart_and_read_only_fields() {
        let mut h = Host::new(Rtc::default());
        h.store(0, &[0x56, 0x34, 0x12, 0x0a, 0x28, 0x02, 0x24]);
        h.seconds(20);
        assert_eq!(h.rtc.status().time, [2024, 2, 28, 12, 34, 56]);
        assert!(!h.rtc.status().oscrun);
        h.store(0, &[0xd6]);
        h.seconds(1);
        assert_eq!(h.rtc.status().time, [2024, 2, 28, 12, 34, 57]);
        assert_eq!(h.random(3, 3), [0x2a, 0x28, 0x22]);
        h.store(0x18, &[1, 2, 3, 4, 5, 6, 7, 8]);
        h.store(3, &[0x1a]);
        h.store(3, &[0x0a]);
        assert_eq!(h.random(0x18, 8), [0; 8]);
        h.store(7, &[0xff, 0xa5, 0xff]);
        assert_eq!(h.random(7, 3), [0xff, 0xa5, 0]);
        h.store(0x0c, &[0xff, 0x80]);
        h.store(0x13, &[0xff, 0xff]);
        let r = h.rtc.image();
        assert_eq!(r[0x0c] & 0x40, 0);
        assert_eq!(r[0x13] & 0x40, 0);
        assert_eq!(r[0x14] & 0x80, 0x80);
    }
    #[test]
    fn reserved_bytes_keep_the_image_and_ignore_writes() {
        let mut image = Rtc::default().image();
        image[0x10] = 0x01;
        image[0x17] = 0x01;
        let mut h = Host::new(Rtc::new(image));
        for (address, expected) in [(0x09, 0x00), (0x10, 0x01), (0x17, 0x01)] {
            assert_eq!(h.random(address, 1), [expected]);
            h.store(address, &[0xff]);
            assert_eq!(h.random(address, 1), [expected]);
        }
        // Preservation is image-fed, not a hard-coded fit to the HR09 bytes.
        image[0x09] = 0xa5;
        image[0x10] = 0x5a;
        image[0x17] = 0x81;
        let mut h = Host::new(Rtc::new(image));
        for address in [0x09, 0x10, 0x17] {
            assert_eq!(h.random(address, 1), [image[address as usize]]);
            h.store(address, &[0]);
            assert_eq!(h.random(address, 1), [image[address as usize]]);
        }
    }
    #[test]
    fn calendar_rollovers_and_twelve_hour_noon_midnight() {
        let mut h = Host::new(Rtc::default());
        for (year, month, date, expected) in [
            (0x24, 2, 0x28, [2024, 2, 29]),
            (0x24, 2, 0x29, [2024, 3, 1]),
            (0x26, 2, 0x28, [2026, 3, 1]),
            (0x26, 4, 0x30, [2026, 5, 1]),
            (0x99, 0x12, 0x31, [2000, 1, 1]),
        ] {
            h.store(0, &[0xd9, 0x59, 0x23, 7, date, month, year]);
            h.seconds(1);
            assert_eq!(&h.rtc.status().time[..3], expected);
            assert_eq!(&h.rtc.status().time[3..], [0, 0, 0]);
            assert_eq!(h.rtc.image()[3] & 7, 1);
        }
        for (value, expected, date) in [(0x51, 0x72, 1), (0x71, 0x52, 2)] {
            h.store(0, &[0xd9, 0x59, value, 1, 1, 1, 0x26]);
            h.seconds(1);
            assert_eq!(h.rtc.image()[2], expected);
            assert_eq!(h.rtc.image()[4], date);
        }
    }
    #[test]
    fn buffered_reads_fractional_seconds_and_checkpoint_mid_byte() {
        let mut h = Host::new(Rtc::default());
        h.store(0, &[0xd9, 0x59, 0x23, 1, 0x31, 0x12, 0x26]);
        h.pointer(0);
        h.start();
        assert!(h.write(0xdf));
        let saved = h.rtc.clone();
        let time = h.time;
        assert_eq!(h.read(true), 0xd9);
        h.seconds(1);
        assert_eq!(h.read(false), 0x59);
        h.stop();
        assert_eq!(h.rtc.status().time, [2027, 1, 1, 0, 0, 0]);
        h.rtc = saved;
        h.time = time;
        assert_eq!(h.read(true), 0xd9);
        h.seconds(1);
        assert_eq!(h.read(false), 0x59);
        h.stop();
        assert_eq!(h.random(0, 3), [0x80, 0, 0]);
        h.store(0x20, &[0xa5]);
        h.pointer(0x20);
        h.start();
        assert!(h.write(0xdf));
        let mut prefix = 0;
        for _ in 0..3 {
            h.set(1, false);
            prefix = (prefix << 1) | h.line();
            h.set(0, false);
        }
        let saved = h.rtc.clone();
        let time = h.time;
        assert_eq!(h.read_tail(prefix, 5, false), 0xa5);
        h.stop();
        let status = h.rtc.status();
        h.rtc = saved;
        h.time = time;
        assert_eq!(h.read_tail(prefix, 5, false), 0xa5);
        h.stop();
        assert_eq!(h.rtc.status(), status);
        let mut rtc = Rtc::default();
        rtc.feed(SECOND_PS - 1, Pins { scl: 1, sda: 1 });
        assert_eq!(rtc.image()[0], 0x80);
        rtc.feed(SECOND_PS, Pins { scl: 1, sda: 1 });
        assert_eq!(rtc.image()[0], 0x81);
    }
    #[test]
    fn alarms_match_masks_and_clear_without_software_setting_the_flag() {
        let mut h = Host::new(Rtc::default());
        h.store(0x0a, &[1, 0, 0, 0, 1, 1]);
        h.store(0x11, &[0x30, 0x30, 0x12, 0x70, 2, 2]);
        h.seconds(1);
        assert_ne!(h.rtc.image()[0x0d] & 8, 0);
        assert_eq!(h.rtc.image()[0x14] & 8, 0);
        h.store(0x0a, &[0x30]);
        h.store(0x0d, &[8]);
        assert_eq!(h.rtc.image()[0x0d] & 8, 0);
    }
}
