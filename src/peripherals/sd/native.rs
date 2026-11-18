//! Native SD-bus protocol, SD Physical Layer Simplified v2.00 chapters 3..5.
//! Media belongs to SdCard; this engine owns only protocol and transfer state.
use super::{crc7, BLOCK_SIZE};
use std::collections::{BTreeMap, VecDeque};

/// Resolved CMD and DAT pads: 0, 1, X=2, Z=3, sampled in emulated time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pins {
    pub clk: u8,
    pub cmd: u8,
    pub dat: [u8; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Idle = 0,
    Ready = 1,
    Ident = 2,
    Stby = 3,
    Tran = 4,
    Data = 5,
    Rcv = 6,
    Prg = 7,
}
impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Ready => "ready",
            Self::Ident => "ident",
            Self::Stby => "stby",
            Self::Tran => "tran",
            Self::Data => "data",
            Self::Rcv => "rcv",
            Self::Prg => "prg",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub state: State,
    pub rca: u16,
    pub width: u8,
    pub dat3_pullup: bool,
    pub high_speed: bool,
    pub last_command: Option<u8>,
    pub last_argument: u32,
    pub blocks_read: u64,
    pub blocks_written: u64,
    pub crc_errors: u64,
    pub illegal_commands: u64,
    pub commands: u64,
    pub responses: u64,
    pub version: u64,
}
pub(super) struct Media<'a> {
    pub base: &'a [u8],
    pub written: &'a mut BTreeMap<u32, [u8; BLOCK_SIZE]>,
    pub blocks: u32,
}
impl Media<'_> {
    fn read(&self, block: u32) -> [u8; BLOCK_SIZE] {
        if let Some(b) = self.written.get(&block) {
            return *b;
        }
        let mut b = [0; BLOCK_SIZE];
        let at = block as usize * BLOCK_SIZE;
        if at < self.base.len() {
            let end = (at + BLOCK_SIZE).min(self.base.len());
            b[..end - at].copy_from_slice(&self.base[at..end]);
        }
        b
    }
}
#[derive(Clone)]
struct Receive {
    block: u32,
    multi: bool,
    started: bool,
    symbols: usize,
    bytes: Vec<u8>,
    shift: u8,
    crc: [u16; 4],
    received: [u16; 4],
}
#[derive(Clone)]
pub struct Native {
    pub status: Status,
    previous: Option<Pins>,
    command: u64,
    bits: u8,
    app: bool,
    polls: u8,
    errors: u32,
    cmd_tx: VecDeque<bool>,
    cmd_delay: u8,
    cmd_drive: Option<bool>,
    dat_tx: VecDeque<[Option<bool>; 4]>,
    dat_drive: [Option<bool>; 4],
    pending: Option<(Vec<u8>, Option<u32>)>,
    read_block: Option<u32>,
    multi: Option<u32>,
    rx: Option<Receive>,
    busy_after_response: bool,
    release_data: bool,
    write_failed: bool,
    after_write: Option<(u32, bool)>,
    pub(super) spi_command: Option<[u8; 6]>,
}
impl Default for Native {
    fn default() -> Self {
        Self {
            status: Status {
                state: State::Idle,
                rca: 0,
                width: 1,
                dat3_pullup: true,
                high_speed: false,
                last_command: None,
                last_argument: 0,
                blocks_read: 0,
                blocks_written: 0,
                crc_errors: 0,
                illegal_commands: 0,
                commands: 0,
                responses: 0,
                version: 0,
            },
            previous: None,
            command: 0,
            bits: 0,
            app: false,
            polls: 0,
            errors: 0,
            cmd_tx: VecDeque::new(),
            cmd_delay: 0,
            cmd_drive: None,
            dat_tx: VecDeque::new(),
            dat_drive: [None; 4],
            pending: None,
            read_block: None,
            multi: None,
            rx: None,
            busy_after_response: false,
            release_data: false,
            write_failed: false,
            after_write: None,
            spi_command: None,
        }
    }
}
/// CRC16, one bit on one DAT line, MSB first, initial remainder zero.
pub fn crc_bit(crc: u16, bit: u8) -> u16 {
    (crc << 1)
        ^ if ((crc >> 15) as u8 ^ bit) & 1 != 0 {
            0x1021
        } else {
            0
        }
}
/// Fictional MID=00 placeholder, OID=XE, PNM=EMU00, no serial, January 2026.
/// This is not an assertion about the SD Association's manufacturer registry.
pub fn cid() -> [u8; 16] {
    let mut b = [0; 16];
    b[1..3].copy_from_slice(b"XE");
    b[3..8].copy_from_slice(b"EMU00");
    b[8] = 0x10;
    b[13] = 1;
    b[14] = 0xa1;
    b[15] = (crc7(&b[..15]) << 1) | 1;
    b
}
/// CSD v2.0, rounded native capacity in 512 KiB units. No erase/lock classes.
pub fn csd(blocks: u32) -> [u8; 16] {
    let mut b = [
        0x40, 0x0e, 0, 0x32, 0x51, 0x59, 0, 0, 0, 0, 0x7f, 0x80, 0x0a, 0x40, 0, 0,
    ];
    let size = blocks.div_ceil(1024) - 1;
    b[7] = ((size >> 16) & 63) as u8;
    b[8] = (size >> 8) as u8;
    b[9] = size as u8;
    b[15] = (crc7(&b[..15]) << 1) | 1;
    b
}
/// Frame payload with start, per-line CRC16 and end; DAT0 carries a 1-bit bus.
pub fn data_frame(bytes: &[u8], width: u8) -> Vec<[Option<bool>; 4]> {
    let mut out = Vec::new();
    let mut crc = [0; 4];
    let active = width as usize;
    let row = |symbol: u8| std::array::from_fn(|i| (i < active).then_some(symbol & (1 << i) != 0));
    out.push(row(0));
    for &byte in bytes {
        for shift in (0..8 / width).rev() {
            let symbol = (byte >> (shift * width)) & ((1 << width) - 1);
            out.push(row(symbol));
            for (i, c) in crc.iter_mut().enumerate().take(active) {
                *c = crc_bit(*c, (symbol >> i) & 1);
            }
        }
    }
    for bit in (0..16).rev() {
        out.push(std::array::from_fn(|i| {
            (i < active).then_some(crc[i] & (1 << bit) != 0)
        }));
    }
    out.push(row((1 << width) - 1));
    out
}
impl Native {
    pub fn active(&self) -> bool {
        self.bits > 0
            || self.cmd_delay > 0
            || self.cmd_drive.is_some()
            || !self.cmd_tx.is_empty()
            || self.pending.is_some()
            || !self.dat_tx.is_empty()
            || self.dat_drive.iter().any(Option::is_some)
            || self.rx.is_some()
            || self.multi.is_some()
            || self.busy_after_response
            || self.write_failed
    }
    /// Push-pull only while transmitting; otherwise all transmitters release.
    pub fn outputs(&self) -> [Option<bool>; 5] {
        [
            self.cmd_drive,
            self.dat_drive[0],
            self.dat_drive[1],
            self.dat_drive[2],
            self.dat_drive[3],
        ]
    }
    /// Ideal weak DAT3 bias. Release when the host pulls low/unknown or is
    /// receiving a write. It is not another push-pull data transmitter.
    pub fn dat3_bias(&self) -> bool {
        self.status.dat3_pullup
            && self.rx.is_none()
            && self.previous.is_none_or(|p| matches!(p.dat[3], 1 | 3))
    }
    fn card_status(&self, app: bool) -> u32 {
        self.errors
            | ((self.status.state as u32) << 9)
            | if matches!(self.status.state, State::Tran | State::Stby) {
                1 << 8
            } else {
                0
            }
            | if app { 1 << 5 } else { 0 }
    }
    fn response(&mut self, index: u8, argument: u32, no_crc: bool) {
        let mut b = [0; 6];
        b[0] = index;
        b[1..5].copy_from_slice(&argument.to_be_bytes());
        b[5] = if no_crc {
            255
        } else {
            (crc7(&b[..5]) << 1) | 1
        };
        self.queue_response(&b);
        if !no_crc && index != 8 {
            self.errors = 0;
        }
    }
    fn long_response(&mut self, bytes: [u8; 16]) {
        let mut b = vec![0x3f];
        b.extend(bytes);
        self.queue_response(&b);
    }
    fn queue_response(&mut self, bytes: &[u8]) {
        self.cmd_tx = bytes
            .iter()
            .flat_map(|b| (0..8).rev().map(move |bit| b & (1 << bit) != 0))
            .collect();
        // Fixed NCR=4, NID=5. Simplified v2.00 omits its timing tables;
        // corroborated by MC9328MX1 Table 31; no variable card latency.
        self.cmd_delay = if matches!(self.status.last_command, Some(2 | 41)) {
            5
        } else {
            4
        };
        self.status.responses += 1;
    }
    fn fail(&mut self, crc: bool) {
        self.errors |= if crc { 1 << 23 } else { 1 << 22 };
        if crc {
            self.status.crc_errors += 1;
        } else {
            self.status.illegal_commands += 1;
        }
        self.status.version += 1;
    }
    fn read_data(&mut self, bytes: Vec<u8>, block: Option<u32>) {
        self.pending = Some((bytes, block));
        self.status.state = State::Data;
    }
    fn stop_data(&mut self) {
        self.release_data = true;
        self.dat_tx.clear();
        self.pending = None;
        self.read_block = None;
        self.multi = None;
        self.rx = None;
        self.after_write = None;
        self.write_failed = false;
    }
    fn command(&mut self, bytes: [u8; 6], dat3: u8, media: &mut Media<'_>) {
        let cmd = bytes[0] & 63;
        let arg = u32::from_be_bytes(bytes[1..5].try_into().unwrap());
        self.status.last_command = Some(cmd);
        self.status.last_argument = arg;
        self.status.commands += 1;
        self.status.version += 1;
        if bytes[0] & 0xc0 != 0x40 || bytes[5] & 1 == 0 || bytes[5] >> 1 != crc7(&bytes[..5]) {
            self.app = false;
            self.fail(true);
            return;
        }
        if cmd == 0 && dat3 == 0 {
            self.spi_command = Some(bytes);
            return;
        }
        if cmd == 0 {
            self.stop_data();
            self.cmd_tx.clear();
            self.cmd_delay = 0;
            self.cmd_drive = None;
            self.dat_drive = [None; 4];
            self.status.state = State::Idle;
            self.status.rca = 0;
            self.status.width = 1;
            self.status.dat3_pullup = true;
            self.status.high_speed = false;
            self.app = false;
            self.polls = 0;
            self.errors = 0;
            return;
        }
        let app = std::mem::take(&mut self.app);
        let state = self.status.state;
        let addressed = ((arg >> 16) as u16) == self.status.rca;
        if !app && matches!(cmd, 9 | 10 | 13 | 55) && !addressed {
            return;
        }
        if app {
            match cmd {
                41 if state == State::Idle => {
                    if arg == 0 {
                        self.response(63, 0x00ff8000, true);
                        return;
                    }
                    self.polls = self.polls.saturating_add(1);
                    let ready = self.polls >= 3 && arg & 0x40000000 != 0 && arg & 0x00ff8000 != 0;
                    if ready {
                        self.status.state = State::Ready;
                    }
                    self.response(63, if ready { 0xc0ff8000 } else { 0x00ff8000 }, true);
                }
                6 if state == State::Tran && matches!(arg & 3, 0 | 2) => {
                    self.status.width = if arg & 3 == 2 { 4 } else { 1 };
                    self.response(cmd, self.card_status(false), false);
                }
                42 if state == State::Tran => {
                    self.status.dat3_pullup = arg & 1 != 0;
                    self.response(cmd, self.card_status(false), false);
                }
                51 if state == State::Tran => {
                    self.response(cmd, self.card_status(false), false);
                    self.read_data(vec![2, 5, 0, 0, 0, 0, 0, 0], None);
                }
                13 if state == State::Tran => {
                    let mut data = vec![0; 64];
                    data[0] = if self.status.width == 4 { 0x80 } else { 0 };
                    self.response(cmd, self.card_status(false), false);
                    self.read_data(data, None);
                }
                _ => self.fail(false),
            }
            return;
        }
        match cmd {
            8 if state == State::Idle && arg & 0xf00 == 0x100 => {
                self.response(cmd, arg & 0xfff, false)
            }
            55 if matches!(
                state,
                State::Idle | State::Stby | State::Tran | State::Data | State::Rcv | State::Prg
            ) && addressed =>
            {
                self.app = true;
                self.response(cmd, self.card_status(true), false);
            }
            2 if state == State::Ready => {
                self.status.state = State::Ident;
                self.long_response(cid());
            }
            3 if matches!(state, State::Ident | State::Stby) => {
                self.status.rca = 1;
                self.status.state = State::Stby;
                let s = self.card_status(false);
                self.response(
                    cmd,
                    0x10000 | (s & 0x1fff) | ((s >> 8) & 0xc000) | ((s >> 6) & 0x2000),
                    false,
                );
            }
            9 | 10 if state == State::Stby && addressed => {
                self.long_response(if cmd == 9 { csd(media.blocks) } else { cid() })
            }
            7 if matches!(state, State::Stby | State::Tran) => {
                if addressed && self.status.rca != 0 {
                    self.status.state = State::Tran;
                    self.response(cmd, self.card_status(false), false);
                    self.busy_after_response = true;
                } else {
                    self.stop_data();
                    self.status.state = State::Stby;
                }
            }
            13 if addressed
                && matches!(
                    state,
                    State::Stby | State::Tran | State::Data | State::Rcv | State::Prg
                ) =>
            {
                let s = self.card_status(false);
                self.response(cmd, s, false);
                self.errors = 0;
            }
            16 if state == State::Tran => {
                // SDHC memory accesses remain 512 bytes regardless of CMD16
                // (§4.3.14); shorter lengths matter only for unmodelled CMD42.
                if arg > 512 {
                    self.errors |= 1 << 29;
                }
                self.response(cmd, self.card_status(false), false);
            }
            17 | 18 if state == State::Tran => {
                if arg >= media.blocks {
                    self.errors |= 1 << 31;
                    self.response(cmd, self.card_status(false), false);
                    return;
                }
                self.response(cmd, self.card_status(false), false);
                self.multi = (cmd == 18).then_some(arg + 1);
                self.read_data(media.read(arg).to_vec(), Some(arg));
            }
            12 if matches!(state, State::Data | State::Rcv | State::Prg) => {
                self.stop_data();
                self.status.state = State::Tran;
                self.response(cmd, self.card_status(false), false);
                self.busy_after_response = true;
            }
            24 | 25 if state == State::Tran => {
                if arg >= media.blocks {
                    self.errors |= 1 << 31;
                    self.response(cmd, self.card_status(false), false);
                    return;
                }
                self.response(cmd, self.card_status(false), false);
                self.start_write(arg, cmd == 25);
            }
            6 if state == State::Tran => {
                let function = arg & 15;
                let set = arg >> 31 != 0;
                let selected = if function <= 1 { function as u8 } else { 15 };
                if set && selected <= 1 {
                    self.status.high_speed = selected == 1;
                }
                let mut data = vec![0; 64];
                data[1] = 100;
                data[13] = 3;
                data[16] = selected; // 25/50 MHz, function group 1.
                self.response(cmd, self.card_status(false), false);
                self.read_data(data, None);
            }
            _ => self.fail(false),
        }
    }
    fn start_write(&mut self, block: u32, multi: bool) {
        self.write_failed = false;
        self.status.state = State::Rcv;
        self.rx = Some(Receive {
            block,
            multi,
            started: false,
            symbols: 0,
            bytes: Vec::with_capacity(512),
            shift: 0,
            crc: [0; 4],
            received: [0; 4],
        });
    }
    fn receive_data(&mut self, dat: [u8; 4], media: &mut Media<'_>) {
        let width = self.status.width as usize;
        let payload = 4096 / width;
        let Some(rx) = &mut self.rx else {
            return;
        };
        if dat[..width].iter().any(|&d| d > 1) {
            self.rx = None;
            self.fail(true);
            self.status.state = State::Tran;
            return;
        }
        if !rx.started {
            if dat[..width].iter().all(|&d| d == 0) {
                rx.started = true;
            }
            return;
        }
        if rx.symbols < payload {
            let mut symbol = 0;
            for (i, &bit) in dat.iter().enumerate().take(width) {
                symbol |= bit << i;
                rx.crc[i] = crc_bit(rx.crc[i], bit);
            }
            rx.shift = (rx.shift << width) | symbol;
            if (rx.symbols + 1).is_multiple_of(8 / width) {
                rx.bytes.push(rx.shift);
            }
        } else if rx.symbols < payload + 16 {
            for (i, &bit) in dat.iter().enumerate().take(width) {
                rx.received[i] = (rx.received[i] << 1) | u16::from(bit);
            }
        } else {
            let accepted = dat[..width].iter().all(|&d| d == 1)
                && rx.crc[..width] == rx.received[..width]
                && rx.block < media.blocks;
            let (block, multi) = (rx.block, rx.multi);
            if accepted {
                media
                    .written
                    .insert(block, rx.bytes.as_slice().try_into().unwrap());
                self.status.blocks_written += 1;
            } else {
                self.status.crc_errors += 1;
                self.errors |= 1 << 23;
                // §4.3.4: after a multi-block CRC error, ignore further
                // data and wait in Rcv for CMD12, rather than retrying it.
                self.write_failed = multi;
            }
            self.after_write = Some((if accepted { block + 1 } else { block }, multi));
            self.rx = None;
            self.status.state = State::Prg;
            self.dat_tx.extend([[None; 4]; 2]);
            for bit in (0..5).rev() {
                self.dat_tx.push_back([
                    Some((if accepted { 5u8 } else { 11u8 }) & (1 << bit) != 0),
                    None,
                    None,
                    None,
                ]);
            }
            if accepted {
                self.dat_tx.extend([[Some(false), None, None, None]; 8]);
            }
            self.status.version += 1;
            return;
        }
        rx.symbols += 1;
    }
    pub(super) fn feed(&mut self, _ps: u64, pins: Pins, media: &mut Media<'_>) {
        let pins = Pins {
            clk: if pins.clk == 3 { 1 } else { pins.clk },
            cmd: if pins.cmd == 3 { 1 } else { pins.cmd },
            dat: pins.dat.map(|d| if d == 3 { 1 } else { d }),
        };
        let old = self.previous.replace(pins);
        if old == Some(pins) {
            return;
        }
        let rising = pins.clk == 1 && old.is_some_and(|p| p.clk == 0);
        let falling = pins.clk == 0 && old.is_some_and(|p| p.clk == 1);
        if rising {
            if self.cmd_drive.is_none() && self.cmd_tx.is_empty() && self.cmd_delay == 0 {
                if pins.cmd > 1 {
                    self.bits = 0;
                } else if self.bits > 0 || pins.cmd == 0 {
                    self.command = (self.command << 1) | u64::from(pins.cmd);
                    self.bits += 1;
                    if self.bits == 48 {
                        let b = self.command.to_be_bytes();
                        self.bits = 0;
                        self.command(b[2..].try_into().unwrap(), pins.dat[3], media);
                    }
                }
            }
            if self.rx.as_ref().is_some_and(|rx| rx.started)
                || (self.cmd_drive.is_none()
                    && self.cmd_tx.is_empty()
                    && self.cmd_delay == 0
                    && self.dat_drive.iter().all(Option::is_none))
            {
                self.receive_data(pins.dat, media);
            }
        }
        if falling {
            if self.cmd_delay > 0 {
                self.cmd_delay -= 1;
                self.cmd_drive = None;
            } else {
                self.cmd_drive = self.cmd_tx.pop_front();
            }
            if self.release_data {
                self.dat_drive = [None; 4];
                self.release_data = false;
            }
            let command_busy =
                self.cmd_drive.is_some() || self.cmd_delay > 0 || !self.cmd_tx.is_empty();
            let was_driving = self.dat_drive.iter().any(Option::is_some);
            if let Some(row) = self.dat_tx.pop_front() {
                self.dat_drive = row;
                return;
            }
            self.dat_drive = [None; 4];
            if was_driving {
                let state = self.status.state;
                if self.read_block.take().is_some() {
                    self.status.blocks_read += 1;
                    self.status.version += 1;
                }
                if let Some((next, multi)) = self.after_write.take() {
                    if multi {
                        if self.write_failed {
                            self.status.state = State::Rcv;
                        } else {
                            self.start_write(next, true);
                        }
                    } else {
                        self.status.state = State::Tran;
                    }
                } else if self.multi.is_none() {
                    self.status.state = State::Tran;
                }
                if self.status.state != state {
                    self.status.version += 1;
                }
            }
            if command_busy {
                return;
            }
            if self.busy_after_response {
                self.busy_after_response = false;
                self.dat_tx.extend([[Some(false), None, None, None]; 8]);
            }
            if self.pending.is_none() {
                if let Some(block) = self.multi {
                    if block < media.blocks {
                        self.multi = Some(block + 1);
                        self.pending = Some((media.read(block).to_vec(), Some(block)));
                    } else {
                        self.multi = None;
                        self.status.state = State::Tran;
                    }
                }
            }
            if let Some((data, block)) = self.pending.take() {
                self.read_block = block;
                self.dat_tx.extend([[None; 4]; 4]);
                self.dat_tx.extend(data_frame(&data, self.status.width));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::sdcard::SdCard;
    struct Host {
        card: SdCard,
        time: u64,
    }
    impl Host {
        fn new() -> Self {
            let mut card = SdCard::demo();
            card.native_interface();
            let mut h = Self { card, time: 0 };
            h.feed(0, None, [None; 4]);
            h
        }
        fn resolved(&self, cmd: Option<bool>, dat: [Option<bool>; 4]) -> Pins {
            let drive = self.card.bus_outputs();
            let resolve = |host: Option<bool>, card: Option<bool>| match (host, card) {
                (Some(a), Some(b)) if a != b => 2,
                (Some(a), _) | (_, Some(a)) => u8::from(a),
                _ => 1,
            };
            Pins {
                clk: 0,
                cmd: resolve(cmd, drive[0]),
                dat: std::array::from_fn(|i| resolve(dat[i], drive[i + 1])),
            }
        }
        fn feed(&mut self, clk: u8, cmd: Option<bool>, dat: [Option<bool>; 4]) -> Pins {
            let mut p = self.resolved(cmd, dat);
            p.clk = clk;
            self.time += 1000;
            self.card.feed_bus(self.time, p);
            let mut feedback = self.resolved(cmd, dat);
            feedback.clk = clk;
            self.card.feed_bus(self.time + 1, feedback);
            p
        }
        fn cycle(&mut self, cmd: Option<bool>, dat: [Option<bool>; 4]) -> Pins {
            self.feed(0, cmd, dat);
            let p = self.feed(1, cmd, dat);
            self.feed(0, cmd, dat);
            p
        }
        fn clocks(&mut self, n: usize) {
            for _ in 0..n {
                self.cycle(None, [None; 4]);
            }
        }
        fn send(&mut self, cmd: u8, arg: u32, bad: bool) {
            let mut b = [0u8; 6];
            b[0] = 0x40 | cmd;
            b[1..5].copy_from_slice(&arg.to_be_bytes());
            b[5] = (crc7(&b[..5]) << 1) | 1;
            if bad {
                b[5] ^= 2;
            }
            for byte in b {
                for bit in (0..8).rev() {
                    self.cycle(Some(byte & (1 << bit) != 0), [None; 4]);
                }
            }
        }
        fn response(&mut self, n: usize) -> Vec<u8> {
            let mut bits = Vec::new();
            for _ in 0..64 {
                if self.cycle(None, [None; 4]).cmd == 0 {
                    bits.push(0);
                    break;
                }
            }
            assert!(
                !bits.is_empty(),
                "response timeout {:?}",
                self.card.status()
            );
            for _ in 1..n {
                bits.push(self.cycle(None, [None; 4]).cmd);
            }
            let b: Vec<_> = bits
                .chunks(8)
                .map(|c| c.iter().fold(0, |v, &bit| (v << 1) | bit))
                .collect();
            assert_eq!(b.last().unwrap() & 1, 1);
            if n == 48 && b[0] != 63 {
                assert_eq!(b[5] >> 1, crc7(&b[..5]));
            }
            b
        }
        fn cmd(&mut self, cmd: u8, arg: u32, n: usize) -> Vec<u8> {
            self.send(cmd, arg, false);
            self.response(n)
        }
        fn app(&mut self, cmd: u8, arg: u32) -> Vec<u8> {
            self.cmd(55, 0x10000, 48);
            self.cmd(cmd, arg, 48)
        }
        fn init(&mut self) {
            self.send(0, 0, false);
            self.clocks(8);
            assert_eq!(self.cmd(8, 0x1aa, 48)[1..5], 0x1aau32.to_be_bytes());
            for i in 0..3 {
                self.cmd(55, 0, 48);
                let r = self.cmd(41, 0x40ff8000, 48);
                assert_eq!(r[1] & 0x80 != 0, i == 2);
            }
            assert_eq!(&self.cmd(2, 0, 136)[1..], &cid());
            let r = self.cmd(3, 0, 48);
            assert_eq!(&r[1..3], &[0, 1]);
            assert_eq!(&self.cmd(9, 0x10000, 136)[1..], &csd(131072));
            self.cmd(7, 0x10000, 48);
            self.clocks(1);
            assert_eq!(self.cycle(None, [None; 4]).dat[0], 0, "R1b DAT0 busy");
            self.clocks(20);
            assert_eq!(self.card.status().native_state, Some(State::Tran));
        }
        fn data(&mut self, len: usize, width: u8) -> Vec<u8> {
            let mut start = None;
            for _ in 0..128 {
                let p = self.cycle(None, [None; 4]);
                if p.dat[0] == 0 {
                    start = Some(p);
                    break;
                }
            }
            let p = start.expect("data timeout");
            assert!(p.dat[..width as usize].iter().all(|&b| b == 0));
            let mut result = Vec::new();
            let mut byte = 0;
            let mut crc = [0; 4];
            for n in 0..len * 8 / width as usize {
                let p = self.cycle(None, [None; 4]);
                let mut symbol = 0;
                for (i, &b) in p.dat.iter().enumerate().take(width as usize) {
                    assert!(b < 2);
                    symbol |= b << i;
                    crc[i] = crc_bit(crc[i], b);
                }
                byte = (byte << width) | symbol;
                if (n + 1).is_multiple_of(8 / width as usize) {
                    result.push(byte);
                }
            }
            let mut got = [0u16; 4];
            for _ in 0..16 {
                let p = self.cycle(None, [None; 4]);
                for (i, &b) in p.dat.iter().enumerate().take(width as usize) {
                    got[i] = (got[i] << 1) | u16::from(b);
                }
            }
            assert_eq!(got[..width as usize], crc[..width as usize]);
            let p = self.cycle(None, [None; 4]);
            assert!(p.dat[..width as usize].iter().all(|&b| b == 1));
            result
        }
    }
    #[test]
    fn four_line_crc_matches_independent_byte_crc_and_wire_order() {
        let bytes = [0x96; 512];
        let rows = data_frame(&bytes, 4);
        assert_eq!(rows[1], [Some(true), Some(false), Some(false), Some(true)]);
        assert_eq!(rows[2], [Some(false), Some(true), Some(true), Some(false)]);
        for line in 0..4 {
            let stream: Vec<u8> = rows[1..1025]
                .chunks(8)
                .map(|bits| {
                    bits.iter()
                        .fold(0, |b, r| (b << 1) | u8::from(r[line].unwrap()))
                })
                .collect();
            let expected = crate::peripherals::sdcard::crc16(&stream);
            let actual = rows[1025..1041]
                .iter()
                .fold(0u16, |crc, r| (crc << 1) | u16::from(r[line].unwrap()));
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn framing_responses_identification_and_default_registers() {
        let mut h = Host::new();
        h.init();
        let cid = cid();
        assert_eq!(cid[0], 0);
        assert_eq!(&cid[1..8], b"XEEMU00");
        assert_eq!(crc7(&cid[..15]), cid[15] >> 1);
        let csd = csd(131072);
        let size = (u32::from(csd[7] & 63) << 16) | (u32::from(csd[8]) << 8) | u32::from(csd[9]);
        assert_eq!((size + 1) * 512 * 1024, h.card.capacity_bytes() as u32);
        assert_eq!(&h.cmd(13, 0x10000, 48)[1..5], 0x900u32.to_be_bytes());
        let r = h.cmd(16, 1, 48);
        assert_eq!(
            u32::from_be_bytes(r[1..5].try_into().unwrap()) & (1 << 29),
            0
        );
        let r = h.cmd(16, 513, 48);
        assert_ne!(
            u32::from_be_bytes(r[1..5].try_into().unwrap()) & (1 << 29),
            0
        );
        h.send(7, 0, false);
        h.clocks(70);
        assert_eq!(h.card.status().native_state, Some(State::Stby));
        assert_eq!(&h.cmd(10, 0x10000, 136)[1..], &cid);
    }
    #[test]
    fn one_and_four_bit_reads_register_data_and_multi_stop() {
        let mut h = Host::new();
        h.init();
        h.cmd(17, 0, 48);
        let data = h.data(512, 1);
        assert_eq!(&data[3..11], b"ECP5_EMU");
        assert_eq!(h.card.status().blocks_read, 1);
        h.app(6, 2);
        h.cmd(17, 0, 48);
        assert_eq!(h.data(512, 4), data);
        assert_eq!(h.card.status().mode, "sd4");
        h.app(51, 0);
        assert_eq!(h.data(8, 4), [2, 5, 0, 0, 0, 0, 0, 0]);
        h.app(13, 0);
        assert_eq!(h.data(64, 4)[0], 0x80);
        h.cmd(6, 0x00fffff1, 48);
        assert_eq!(h.data(64, 4)[16], 1);
        assert!(!h.card.native.status.high_speed);
        h.cmd(6, 0x80fffff1, 48);
        let status = h.data(64, 4);
        assert_eq!(status[13], 3);
        assert_eq!(status[16], 1);
        assert!(h.card.native.status.high_speed);
        h.cmd(18, 0, 48);
        assert_eq!(h.data(512, 4), data);
        assert_eq!(h.data(512, 4), [0; 512]);
        h.cmd(12, 0, 48);
        h.clocks(20);
        assert_eq!(h.card.status().native_state, Some(State::Tran));
    }
    #[test]
    fn writes_crc_status_busy_and_same_media_snapshot() {
        for width in [1, 4] {
            let mut h = Host::new();
            h.init();
            if width == 4 {
                h.app(6, 2);
            }
            for bad in [false, true] {
                h.cmd(24, 1, 48);
                let data = [0x96; 512];
                let mut rows = data_frame(&data, width);
                if bad {
                    rows[4096 / width as usize + 1][0] =
                        Some(!rows[4096 / width as usize + 1][0].unwrap());
                }
                for row in rows {
                    h.cycle(None, row);
                }
                let mut token = None;
                for _ in 0..16 {
                    if h.cycle(None, [None; 4]).dat[0] == 0 {
                        let mut n = 0;
                        for _ in 0..4 {
                            n = (n << 1) | h.cycle(None, [None; 4]).dat[0];
                        }
                        token = Some(n);
                        break;
                    }
                }
                assert_eq!(token, Some(if bad { 11 } else { 5 }));
                if !bad {
                    assert_eq!(h.cycle(None, [None; 4]).dat[0], 0);
                }
                h.clocks(20);
                h.cmd(17, 1, 48);
                assert_eq!(h.data(512, width), data);
            }
            assert_eq!(h.card.status().blocks_written, 1);
            assert_eq!(h.card.status().crc_errors, 1);
            let saved = h.card.clone();
            h.cmd(24, 2, 48);
            h.card = saved;
            assert_eq!(h.card.status().native_state, Some(State::Tran));
        }
    }
    #[test]
    fn multi_write_and_mid_transfer_restore() {
        let mut h = Host::new();
        h.init();
        h.app(6, 2);
        h.cmd(25, 2, 48);
        for (byte, bad) in [(0x36, false), (0xa9, false), (0x77, true)] {
            let mut rows = data_frame(&[byte; 512], 4);
            if bad {
                rows[1025][0] = Some(!rows[1025][0].unwrap());
            }
            for row in rows {
                h.cycle(None, row);
            }
            let mut token = 0;
            for _ in 0..16 {
                if h.cycle(None, [None; 4]).dat[0] == 0 {
                    for _ in 0..4 {
                        token = (token << 1) | h.cycle(None, [None; 4]).dat[0];
                    }
                    break;
                }
            }
            assert_eq!(token, if bad { 11 } else { 5 });
            if !bad {
                for _ in 0..8 {
                    assert_eq!(h.cycle(None, [None; 4]).dat[0], 0);
                }
            }
            h.clocks(4);
            assert_eq!(h.card.status().native_state, Some(State::Rcv));
        }
        // A valid block after the CRC error is ignored until a stop command.
        for row in data_frame(&[0xff; 512], 4) {
            h.cycle(None, row);
        }
        h.clocks(20);
        assert_eq!(h.card.status().blocks_written, 2);
        assert_eq!(h.card.status().crc_errors, 1);
        h.cmd(12, 0, 48);
        h.clocks(20);
        assert_eq!(h.card.status().native_state, Some(State::Tran));
        assert_eq!(h.card.status().blocks_written, 2);
        for (block, byte) in [(2, 0x36), (3, 0xa9)] {
            h.cmd(17, block, 48);
            let saved = h.card.clone();
            let first = h.data(512, 4);
            h.card = saved;
            assert_eq!(h.data(512, 4), first);
            assert_eq!(first, [byte; 512]);
        }
    }
    #[test]
    fn crc_illegal_no_response_pullup_and_powerup_mode_choice() {
        let mut h = Host::new();
        h.init();
        h.send(38, 0, false);
        for _ in 0..70 {
            assert_eq!(h.cycle(None, [None; 4]).cmd, 1);
        }
        let s = h.cmd(13, 0x10000, 48);
        assert_ne!(
            u32::from_be_bytes(s[1..5].try_into().unwrap()) & (1 << 22),
            0
        );
        h.send(13, 0x10000, true);
        h.clocks(70);
        assert_eq!(h.card.status().crc_errors, 1);
        h.app(42, 0);
        assert!(!h.card.status().dat3_pullup);
        assert!(!h.card.dat3_bias());
        h.send(7, 0, false);
        h.clocks(70);
        assert_eq!(h.card.status().native_state, Some(State::Stby));
        let mut card = SdCard::demo();
        let frame = [0x40, 0, 0, 0, 0, 0x95];
        let mut ps = 0;
        for byte in frame {
            for bit in (0..8).rev() {
                for clk in [0, 1, 0] {
                    ps += 1000;
                    card.feed_bus(
                        ps,
                        Pins {
                            clk,
                            cmd: u8::from(byte & (1 << bit) != 0),
                            dat: [1, 1, 1, 0],
                        },
                    );
                }
            }
        }
        assert_eq!(card.status().mode, "spi");
    }
    #[test]
    fn cmd0_low_latches_the_unchanged_spi_bit_engine() {
        use crate::peripherals::sdcard::SdPins;
        let mut spi = SdCard::demo();
        let mut bus = spi.clone();
        bus.native_interface();
        let mut bytes = vec![0x40, 0, 0, 0, 0, 0x95];
        bytes.extend([255; 8]);
        bytes.extend([0x48, 0, 0, 1, 0xaa, 0x87]);
        bytes.extend([255; 16]);
        let mut ps = 0;
        for byte in bytes {
            for bit in (0..8).rev() {
                for clk in [0, 1, 0] {
                    ps += 1000;
                    let mosi = (byte >> bit) & 1;
                    spi.feed(ps, SdPins { cs: 0, clk, mosi });
                    bus.feed_bus(
                        ps,
                        Pins {
                            clk,
                            cmd: mosi,
                            dat: [1, 1, 1, 0],
                        },
                    );
                    assert_eq!(u8::from(bus.bus_outputs()[1].unwrap_or(true)), spi.miso());
                }
            }
        }
        assert_eq!(bus.status().mode, "spi");
        assert_eq!(bus.status().last_command, Some(8));
    }
}
