//! The monitor's write-protected 24C02-style EDID EEPROM on shared DDC.
//! The ideal open-drain model does not simulate the PCA9306 electrically.
use super::rtc::Pins;
use anyhow::{ensure, Result};

pub const ADDRESS: u8 = 0x50;
pub const CAPACITY: usize = 256;

/// A fictional, unassigned PNP mnemonic: eXperimental EMulator. XEM was
/// absent from the UEFI PNP registry checked on 2026-11-17. EMU is real.
pub const VENDOR: &str = "XEM";

fn timing(clock: u16, h: [u16; 4], v: [u16; 4], size: [u16; 2], flags: u8) -> [u8; 18] {
    let [ha, hb, ho, hw] = h;
    let [va, vb, vo, vw] = v;
    [
        clock as u8,
        (clock >> 8) as u8,
        ha as u8,
        hb as u8,
        ((ha >> 8) << 4 | (hb >> 8)) as u8,
        va as u8,
        vb as u8,
        ((va >> 8) << 4 | (vb >> 8)) as u8,
        ho as u8,
        hw as u8,
        ((vo & 15) << 4 | (vw & 15)) as u8,
        ((ho >> 8) << 6 | (hw >> 8) << 4 | (vo >> 4) << 2 | (vw >> 4)) as u8,
        size[0] as u8,
        size[1] as u8,
        ((size[0] >> 8) << 4 | (size[1] >> 8)) as u8,
        0,
        0,
        flags,
    ]
}

/// Synthetic EDID 1.3, no serial number or extensions. Descriptor clocks
/// have 10 kHz resolution: nominal VGA 25.175 MHz rounds to 25.180 MHz.
pub fn default_block() -> [u8; 128] {
    let mut b = [0u8; 128];
    b[..8].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 0]);
    b[8..10].copy_from_slice(&((24u16 << 10) | (5 << 5) | 13).to_be_bytes());
    b[10] = 1; // Fictional product 1; serial remains zero.
    b[17] = 36; // 2026, no manufacture week specified.
    b[18..25].copy_from_slice(&[1, 3, 0x81, 32, 24, 120, 0x0e]);
    // sRGB chromaticities, gamma 2.2, RGB, preferred timing, no GTF claim.
    b[25..35].copy_from_slice(&[0xee, 0x91, 0xa3, 0x54, 0x4c, 0x99, 0x26, 0x0f, 0x50, 0x54]);
    b[35] = 0x20; // Established 640x480 at 60 Hz.
    b[38..54].fill(1); // Eight unused standard timings (01 01).
    b[54..72].copy_from_slice(&timing(
        2518,
        [640, 160, 16, 96],
        [480, 45, 10, 2],
        [320, 240],
        0x18,
    ));
    b[72..90].copy_from_slice(&timing(
        7425,
        [1280, 370, 110, 40],
        [720, 30, 5, 5],
        [320, 180],
        0x1e,
    ));
    b[90..95].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
    b[95..108].copy_from_slice(b"ECP5 EMU\n    ");
    b[108..113].copy_from_slice(&[0, 0, 0, 0xfd, 0]);
    b[113..126].copy_from_slice(&[59, 61, 31, 46, 8, 0, 10, 32, 32, 32, 32, 32, 32]);
    b[127] = 0u8.wrapping_sub(b[..127].iter().fold(0u8, |sum, &n| sum.wrapping_add(n)));
    b
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub present: bool,
    /// Accepted read addresses, including current-address reads.
    pub reads: u64,
    pub bytes_read: u64,
    /// Last pointer written or byte address completely read, before increment.
    pub last_pointer: u8,
    /// Addresses this device declined, rejected data bytes, and final master NACKs.
    /// This is a device counter, not the resolved bus's error count.
    pub nacks: u64,
    pub ignored_writes: u64,
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
pub struct Edid {
    image: [u8; CAPACITY],
    pointer: u8,
    previous: Pins,
    phase: Phase,
    sda: Option<bool>,
    active: bool,
    status: Status,
}
impl Default for Edid {
    fn default() -> Self {
        Self::new(&default_block(), true).unwrap()
    }
}
impl Edid {
    /// Load bytes without changing them or their checksums. A 128-byte source
    /// leaves 80..FF as FF; addressing always wraps at 256, not at 128.
    pub fn new(bytes: &[u8], present: bool) -> Result<Self> {
        ensure!(
            matches!(bytes.len(), 128 | 256),
            "EDID image needs 128 or 256 bytes"
        );
        let mut image = [0xff; CAPACITY];
        image[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            image,
            pointer: 0,
            previous: Pins { scl: 1, sda: 1 },
            phase: Phase::Wait,
            sda: None,
            active: false,
            status: Status {
                present,
                reads: 0,
                bytes_read: 0,
                last_pointer: 0,
                nacks: 0,
                ignored_writes: 0,
            },
        })
    }
    /// ECP5_EDID=off disconnects the monitor's EEPROM. Invalid/unreadable
    /// ECP5_EDID_IMAGE sources fall back to the synthetic base block.
    pub fn from_env() -> Self {
        let present = std::env::var_os("ECP5_EDID").is_none_or(|v| v != "off");
        if let Some(path) = std::env::var_os("ECP5_EDID_IMAGE") {
            match std::fs::read(path).and_then(|bytes| {
                Self::new(&bytes, present).map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "needs 128 or 256 bytes")
                })
            }) {
                Ok(edid) => return edid,
                Err(error) => {
                    eprintln!("EDID: ECP5_EDID_IMAGE not loaded ({error}); synthetic block instead")
                }
            }
        }
        Self::new(&default_block(), present).unwrap()
    }
    /// Read-only contents, for comparing a configured image in memory.
    pub fn image(&self) -> [u8; CAPACITY] {
        self.image
    }
    pub fn status(&self) -> Status {
        self.status.clone()
    }
    pub fn sda(&self) -> Option<bool> {
        self.sda
    }
    pub fn active(&self) -> bool {
        self.active
    }
    fn received(&mut self, kind: Receive, byte: u8) {
        let (next, ack) = match kind {
            Receive::Address => {
                if !self.status.present || byte >> 1 != ADDRESS {
                    (Next::Wait, false)
                } else if byte & 1 == 0 {
                    (Next::Receive(Receive::Pointer), true)
                } else {
                    self.status.reads += 1;
                    (Next::Send, true)
                }
            }
            Receive::Pointer => {
                self.pointer = byte;
                self.status.last_pointer = byte;
                (Next::Receive(Receive::Data), true)
            }
            Receive::Data => {
                // Conservative write protection: NACK the first data byte,
                // leave memory and pointer unchanged, then await START/STOP.
                self.status.ignored_writes += 1;
                (Next::Wait, false)
            }
        };
        if !ack {
            self.status.nacks += 1;
        }
        self.phase = Phase::Ack(next, ack, 0);
    }
    fn send(&mut self) {
        let byte = self.image[usize::from(self.pointer)];
        self.sda = (byte & 0x80 == 0).then_some(false);
        self.phase = Phase::Send(byte, 0);
    }
    /// START, repeated START, STOP and ninth-bit ACK follow the resolved
    /// shared pads. Return only a pull-down; the board supplies the high.
    /// X aborts the transfer. No clock stretching or electrical delays.
    pub fn feed(&mut self, _time_ps: u64, pins: Pins) {
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
            self.active = false;
            return;
        }
        if old.scl == 1 && pins.scl == 1 && old.sda <= 1 && old.sda != pins.sda {
            self.sda = None;
            self.active = pins.sda == 0;
            self.phase = if self.active {
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
                        self.status.last_pointer = self.pointer;
                        self.status.bytes_read += 1;
                        self.pointer = self.pointer.wrapping_add(1);
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
    struct Host {
        edid: Edid,
        scl: u8,
        low: bool,
        time: u64,
    }
    impl Host {
        fn new(edid: Edid) -> Self {
            Self {
                edid,
                scl: 1,
                low: false,
                time: 0,
            }
        }
        fn line(&self) -> u8 {
            u8::from(!self.low && self.edid.sda().is_none())
        }
        fn feed(&mut self) {
            self.time += 1_000;
            self.edid.feed(
                self.time,
                Pins {
                    scl: self.scl,
                    sda: self.line(),
                },
            );
            self.edid.feed(
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
            assert!(self.write(0xa0));
            assert!(self.write(p));
        }
        fn random(&mut self, p: u8, n: usize) -> Vec<u8> {
            self.pointer(p);
            self.start();
            assert!(self.write(0xa1));
            let bytes = (0..n).map(|i| self.read(i + 1 < n)).collect();
            self.stop();
            bytes
        }
    }

    #[test]
    fn synthetic_base_block_decodes_without_a_real_identity() {
        let b = default_block();
        assert_eq!(&b[..8], &[0, 255, 255, 255, 255, 255, 255, 0]);
        assert_eq!(b.iter().fold(0u8, |sum, &n| sum.wrapping_add(n)), 0);
        let vendor = u16::from_be_bytes([b[8], b[9]]);
        let name: String = [10, 5, 0]
            .map(|shift| char::from(b'A' - 1 + ((vendor >> shift) & 31) as u8))
            .iter()
            .collect();
        assert_eq!(name, VENDOR);
        assert_eq!(&b[12..16], &[0; 4]);
        assert_eq!(&b[18..20], &[1, 3]);
        assert_eq!(b[35] & 0x20, 0x20);
        assert_ne!(b[24] & 2, 0);
        assert_eq!(&b[90..95], &[0, 0, 0, 0xfc, 0]);
        assert_eq!(&b[95..108], b"ECP5 EMU\n    ");
        assert_eq!(&b[108..113], &[0, 0, 0, 0xfd, 0]);
        assert_eq!(&b[113..118], &[59, 61, 31, 46, 8]);
        assert_eq!(b[126], 0);
        // Decode the wire format independently of timing()'s packing.
        for (offset, expected) in [
            (54, (640, 480, 800, 525, 25_180_000, 16, 96, 10, 2, 0x18)),
            (72, (1280, 720, 1650, 750, 74_250_000, 110, 40, 5, 5, 0x1e)),
        ] {
            let d = &b[offset..offset + 18];
            let h = u16::from(d[2]) | (u16::from(d[4] & 0xf0) << 4);
            let v = u16::from(d[5]) | (u16::from(d[7] & 0xf0) << 4);
            let hb = u16::from(d[3]) | (u16::from(d[4] & 15) << 8);
            let vb = u16::from(d[6]) | (u16::from(d[7] & 15) << 8);
            let clock = u32::from(u16::from_le_bytes([d[0], d[1]])) * 10_000;
            let ho = u16::from(d[8]) | (u16::from(d[11] >> 6) << 8);
            let hw = u16::from(d[9]) | (u16::from((d[11] >> 4) & 3) << 8);
            let vo = u16::from(d[10] >> 4) | (u16::from((d[11] >> 2) & 3) << 4);
            let vw = u16::from(d[10] & 15) | (u16::from(d[11] & 3) << 4);
            assert_eq!(
                (h, v, h + hb, v + vb, clock, ho, hw, vo, vw, d[17]),
                expected
            );
            let hz = clock as f64 / f64::from(h + hb) / f64::from(v + vb);
            assert!((59.9..=60.1).contains(&hz));
        }
    }

    #[test]
    fn pointer_current_address_sequential_wrap_and_write_protection() {
        let image: Vec<_> = (0..=255).collect();
        let mut h = Host::new(Edid::new(&image, true).unwrap());
        assert_eq!(h.random(254, 4), [254, 255, 0, 1]);
        h.start();
        assert!(h.write(0xa1));
        assert_eq!(h.read(false), 2);
        h.stop();
        assert_eq!(h.edid.status().reads, 2);
        assert_eq!(h.edid.status().bytes_read, 5);
        assert_eq!(h.edid.status().last_pointer, 2);
        h.pointer(7);
        assert!(!h.write(0xab));
        h.stop();
        assert_eq!(h.edid.status().ignored_writes, 1);
        h.start();
        assert!(h.write(0xa1));
        assert_eq!(h.read(false), 7);
        h.stop();
        assert_eq!(h.edid.image().as_slice(), image);
        let mut base = Host::new(Edid::default());
        assert_eq!(base.random(128, 2), [255; 2]);
        assert!(Edid::new(&[0; 127], true).is_err());
    }

    #[test]
    fn disconnected_foreign_and_segment_addresses_nack_and_unknown_aborts() {
        for present in [false, true] {
            let mut h = Host::new(Edid::new(&default_block(), present).unwrap());
            for address in [0x60, 0xde, 0xdf, 0xa0, 0xa1] {
                h.start();
                assert_eq!(h.write(address), present && address >> 1 == ADDRESS);
                h.stop();
            }
            h.start();
            h.set(0, true);
            h.edid.feed(h.time + 1, Pins { scl: 2, sda: 1 });
            assert!(!h.edid.active());
            assert_eq!(h.edid.sda(), None);
        }
    }

    #[test]
    fn repeated_start_stop_and_checkpoint_in_a_transmitted_byte() {
        let mut h = Host::new(Edid::default());
        h.pointer(0);
        h.start();
        assert!(h.write(0xa1));
        let mut prefix = 0;
        for _ in 0..3 {
            h.set(1, false);
            prefix = (prefix << 1) | h.line();
            h.set(0, false);
        }
        let saved = h.edid.clone();
        let pins = (h.scl, h.low, h.time);
        let byte = h.read_tail(prefix, 5, false);
        h.stop();
        let later = h.edid.status();
        h.edid.clone_from(&saved);
        (h.scl, h.low, h.time) = pins;
        assert_eq!(h.read_tail(prefix, 5, false), byte);
        h.stop();
        assert_eq!(h.edid.status(), later);
        h.start();
        assert!(h.write(0xa0));
        h.set(0, true);
        h.set(1, true);
        h.stop();
        assert_eq!(h.random(0, 8), default_block()[..8]);
    }
}
