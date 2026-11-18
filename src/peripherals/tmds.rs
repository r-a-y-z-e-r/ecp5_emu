//! DVI 1.0, Figure 3-5. Words are transmitted least significant bit first.

pub const CONTROL: [u16; 4] = [0b1101010100, 0b0010101011, 0b0101010100, 0b1010101011];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Symbol {
    Control(u8),
    Data(u8),
    Guard,
    /// Not a TMDS video/control or video guard band code.
    Invalid,
}

#[derive(Default, Clone)]
pub struct Encoder {
    disparity: i16,
}

impl Encoder {
    /// A control period resets running disparity.
    pub fn control(&mut self, c1c0: u8) -> u16 {
        self.disparity = 0;
        CONTROL[(c1c0 & 3) as usize]
    }
    pub fn data(&mut self, byte: u8) -> u16 {
        let (word, disparity) = encode(byte, self.disparity);
        self.disparity = disparity;
        word
    }
}

const fn encode(byte: u8, disparity: i16) -> (u16, i16) {
    let ones = byte.count_ones();
    let xnor = ones > 4 || (ones == 4 && byte & 1 == 0);
    let mut q = (byte & 1) as u16;
    let mut i = 1;
    while i < 8 {
        let b = ((q >> (i - 1)) ^ ((byte as u16) >> i) ^ xnor as u16) & 1;
        q |= b << i;
        i += 1;
    }
    let qm8 = !xnor;
    let balance = 2 * q.count_ones() as i16 - 8;
    let (invert, next) = if disparity == 0 || balance == 0 {
        (!qm8, disparity + if qm8 { balance } else { -balance })
    } else if (disparity > 0 && balance > 0) || (disparity < 0 && balance < 0) {
        (true, disparity + 2 * qm8 as i16 - balance)
    } else {
        (false, disparity - 2 * (!qm8) as i16 + balance)
    };
    (
        (invert as u16) << 9 | (qm8 as u16) << 8 | if invert { q ^ 255 } else { q },
        next,
    )
}

const fn table() -> [Symbol; 1024] {
    let mut out = [Symbol::Invalid; 1024];
    let mut b = 0;
    while b < 256 {
        let mut d = -1;
        while d <= 1 {
            out[encode(b as u8, d).0 as usize] = Symbol::Data(b as u8);
            d += 1;
        }
        b += 1;
    }
    let mut c = 0;
    while c < 4 {
        out[CONTROL[c] as usize] = Symbol::Control(c as u8);
        c += 1;
    }
    out[0b1011001100] = Symbol::Guard;
    out[0b0100110011] = Symbol::Guard;
    out
}
const DECODE: [Symbol; 1024] = table();

/// The two video guard codes overlap legal video bytes (0xAB and 0x55).
/// A receiver uses the preamble to distinguish them during video.
pub fn data_byte(word: u16) -> Option<u8> {
    match decode(word) {
        Symbol::Data(byte) => Some(byte),
        Symbol::Guard => Some(if word == 0b1011001100 { 0xab } else { 0x55 }),
        _ => None,
    }
}

/// Decode a wire word, rejecting codes an encoder cannot produce.
#[inline]
pub fn decode(word: u16) -> Symbol {
    DECODE
        .get(word as usize)
        .copied()
        .unwrap_or(Symbol::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_disparity() {
        let mut e = Encoder::default();
        for c in 0..4 {
            assert_eq!(decode(e.control(c)), Symbol::Control(c));
            for b in (0..=255).chain((0..=255).rev()).cycle().take(8192) {
                let w = e.data(b);
                assert_eq!(data_byte(w), Some(b));
                assert!(e.disparity.abs() <= 8);
            }
        }
        assert_eq!(decode(0b1011001100), Symbol::Guard);
        assert_eq!(decode(0b0100110011), Symbol::Guard);
        assert_eq!(decode(0xffff), Symbol::Invalid);
        // Flowchart examples: XOR on zero, XNOR on all ones, disparity reversal.
        let mut e = Encoder::default();
        assert_eq!(e.data(0), 0x100);
        assert_eq!(e.data(0), 0x3ff);
        e.control(0);
        assert_eq!(e.data(255), 0x200);
    }
}
