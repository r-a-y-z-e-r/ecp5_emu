//! Digital MT48LC16M16A2: four banks of 8192 rows of 512 sixteen-bit words.
//! Commands and write data use the sample before a rising clock edge. This
//! keeps setup values when a zero-delay controller changes outputs at that edge.
use crate::logic::Logic;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

/// No value means high impedance. For byte masking, also consult `drive_mask`:
/// a zero bit in that mask means the corresponding byte is high impedance.
pub type Drive = Option<u16>;

#[derive(Clone, Copy, Debug)]
pub struct SdramPins {
    pub clk: Logic,
    pub cke: Logic,
    pub csn: Logic,
    pub rasn: Logic,
    pub casn: Logic,
    pub wen: Logic,
    pub ba: [Logic; 2],
    pub a: [Logic; 13],
    pub dqm: [Logic; 2],
    pub dq: [Logic; 16],
}
impl Default for SdramPins {
    fn default() -> Self {
        Self {
            clk: Logic::Zero,
            cke: Logic::One,
            csn: Logic::Zero,
            rasn: Logic::One,
            casn: Logic::One,
            wen: Logic::One,
            ba: [Logic::Zero; 2],
            a: [Logic::Zero; 13],
            dqm: [Logic::Zero; 2],
            dq: [Logic::Z; 16],
        }
    }
}
fn number(bits: &[Logic]) -> Option<u16> {
    bits.iter()
        .enumerate()
        .try_fold(0, |n, (i, b)| Some(n | ((b.bit()? as u16) << i)))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Command {
    Deselect,
    Nop,
    Active,
    Read,
    Write,
    BurstTerminate,
    Precharge,
    Refresh,
    LoadMode,
}
impl Command {
    fn decode(p: &SdramPins) -> Option<Self> {
        if p.csn == Logic::One {
            return Some(Self::Deselect);
        }
        if p.csn != Logic::Zero {
            return None;
        }
        Some(match (p.rasn.bit()?, p.casn.bit()?, p.wen.bit()?) {
            (true, true, true) => Self::Nop,
            (false, true, true) => Self::Active,
            (true, false, true) => Self::Read,
            (true, false, false) => Self::Write,
            (true, true, false) => Self::BurstTerminate,
            (false, true, false) => Self::Precharge,
            (false, false, true) => Self::Refresh,
            (false, false, false) => Self::LoadMode,
        })
    }
    fn label(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::Precharge => "PRECHARGE",
            Self::Refresh => "AUTO REFRESH",
            Self::LoadMode => "LOAD MODE",
            Self::BurstTerminate => "BURST TERMINATE",
            Self::Nop => "NOP",
            Self::Deselect => "DESELECT",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub raw: u16,
    pub burst_length: u16,
    pub interleaved: bool,
    pub cas_latency: u64,
    pub single_write: bool,
}
impl Default for Mode {
    fn default() -> Self {
        Self {
            raw: 0x20,
            burst_length: 1,
            interleaved: false,
            cas_latency: 2,
            single_write: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub commands: BTreeMap<Command, u64>,
    /// Data beats, including masked beats.
    pub reads: u64,
    pub writes: u64,
    pub timing_violations: u64,
    pub violations: BTreeMap<&'static str, u64>,
    /// First occurrence of each diagnostic, bounded independently of run length.
    pub messages: Vec<String>,
    pub open_rows: [Option<u16>; 4],
    pub mode: Mode,
    pub initialised: bool,
    pub bytes_touched: u64,
    pub allocated_rows: usize,
    pub refreshes_in_window: usize,
    pub refresh_shortfalls: u64,
    pub commands_before_init: u64,
    pub unknown_samples: u64,
    pub clock_period_ps: u64,
}
#[derive(Clone, Copy, Default)]
struct Bank {
    row: Option<u16>,
    active: Option<u64>,
    precharged: Option<u64>,
    written: Option<u64>,
    auto_close: Option<u64>,
}
#[derive(Clone)]
struct Row {
    words: Box<[u16; 512]>,
    touched: [u64; 16],
}
#[derive(Clone, Copy)]
struct Burst {
    bank: usize,
    row: u16,
    start: u16,
    beat: u16,
    length: u16,
    interleaved: bool,
    auto: bool,
}
impl Burst {
    fn column(self) -> u16 {
        let mask = self.length - 1;
        let low = if self.interleaved {
            self.start ^ self.beat
        } else {
            self.start.wrapping_add(self.beat)
        };
        ((self.start & !mask) | (low & mask)) & 511
    }
}
#[derive(Clone, Copy)]
struct ReadBeat {
    bank: usize,
    row: u16,
    column: u16,
}

#[derive(Clone)]
pub struct Sdram {
    previous: Option<SdramPins>,
    first_time: Option<u64>,
    last_rise: Option<u64>,
    clock: u64,
    banks: [Bank; 4],
    memory: HashMap<(usize, u16), Arc<Row>>,
    status: Status,
    write: Option<Burst>,
    reads: BTreeMap<u64, ReadBeat>,
    drive: Drive,
    drive_mask: u8,
    dqm: u8,
    last_refresh: Option<u64>,
    last_mode: Option<u64>,
    refreshes: VecDeque<u64>,
    refresh_check: Option<u64>,
    init_stage: u8,
}
impl Default for Sdram {
    fn default() -> Self {
        Self::new()
    }
}
impl Sdram {
    pub fn new() -> Self {
        Self {
            previous: None,
            first_time: None,
            last_rise: None,
            clock: 0,
            banks: [Bank::default(); 4],
            memory: HashMap::new(),
            status: Status::default(),
            write: None,
            reads: BTreeMap::new(),
            drive: None,
            drive_mask: 0,
            dqm: 0,
            last_refresh: None,
            last_mode: None,
            refreshes: VecDeque::new(),
            refresh_check: None,
            init_stage: 0,
        }
    }
    pub fn status(&self) -> Status {
        let mut s = self.status.clone();
        s.open_rows = self.banks.map(|b| b.row);
        s.allocated_rows = self.memory.len();
        s.refreshes_in_window = self.refreshes.len();
        s
    }
    /// Bit 0 is the low byte, bit 1 the high byte. Use this with `Drive` to
    /// represent a partially masked read without inventing values on the pads.
    pub fn drive_mask(&self) -> u8 {
        self.drive_mask
    }
    pub fn peek(&self, bank: usize, row: u16, column: u16) -> u16 {
        if bank >= 4 || row >= 8192 || column >= 512 {
            return 0;
        }
        self.memory
            .get(&(bank, row))
            .map_or(0, |r| r.words[column as usize])
    }
    pub fn poke(&mut self, bank: usize, row: u16, column: u16, value: u16) {
        self.store(bank, row, column, value, 3);
    }
    fn store(&mut self, bank: usize, row: u16, column: u16, value: u16, bytes: u8) {
        if bank >= 4 || row >= 8192 || column >= 512 || bytes == 0 {
            return;
        }
        let r = Arc::make_mut(self.memory.entry((bank, row)).or_insert_with(|| {
            Arc::new(Row {
                words: Box::new([0; 512]),
                touched: [0; 16],
            })
        }));
        for byte in 0..2 {
            if bytes & (1 << byte) == 0 {
                continue;
            }
            let mask = 0xff << (8 * byte);
            r.words[column as usize] = (r.words[column as usize] & !mask) | (value & mask);
            let index = 2 * column as usize + byte;
            let bit = 1u64 << (index % 64);
            if r.touched[index / 64] & bit == 0 {
                self.status.bytes_touched += 1;
            }
            r.touched[index / 64] |= bit;
        }
    }
    fn violation(&mut self, key: &'static str, message: String) {
        self.status.timing_violations += 1;
        let count = self.status.violations.entry(key).or_default();
        *count += 1;
        if *count == 1 && self.status.messages.len() < 32 {
            self.status.messages.push(message);
        }
    }
    fn cycles(&self, ps: u64, minimum: u64) -> u64 {
        let period = self.status.clock_period_ps.max(1);
        ps.div_ceil(period).max(minimum)
    }
    fn check(
        &mut self,
        key: &'static str,
        command: Command,
        previous: Option<u64>,
        origin: &str,
        need: u64,
    ) {
        if let Some(previous) = previous {
            let elapsed = self.clock.saturating_sub(previous);
            if elapsed < need {
                self.violation(
                    key,
                    format!(
                        "{key}: {} {elapsed} clocks after {origin}, needs {need} at {} ns",
                        command.label(),
                        self.status.clock_period_ps as f64 / 1000.0
                    ),
                );
            }
        }
    }
    fn close(&mut self, bank: usize, check: bool) {
        let b = self.banks[bank];
        if check && b.row.is_some() {
            self.check(
                "tRAS",
                Command::Precharge,
                b.active,
                "ACTIVE",
                self.cycles(44_000, 1),
            );
            self.check(
                "tWR",
                Command::Precharge,
                b.written,
                "WRITE data",
                self.cycles(self.status.clock_period_ps + 7_500, 2),
            );
        }
        self.banks[bank].row = None;
        self.banks[bank].auto_close = None;
        self.banks[bank].precharged = Some(self.clock);
    }
    fn auto_close(&mut self, bank: usize, earliest: u64) {
        let b = self.banks[bank];
        let ras = b.active.unwrap_or(0) + self.cycles(44_000, 1);
        let wr = b.written.map_or(0, |n| {
            n + self.cycles(self.status.clock_period_ps + 7_500, 2)
        });
        self.banks[bank].auto_close = Some(earliest.max(ras).max(wr));
    }
    fn refresh_window(&mut self, time: u64) {
        const WINDOW: u64 = 64_000_000_000;
        while self
            .refreshes
            .front()
            .is_some_and(|&t| t.saturating_add(WINDOW) < time)
        {
            self.refreshes.pop_front();
        }
        if self.refresh_check.is_some_and(|t| time >= t) {
            if self.refreshes.len() < 8192 {
                self.status.refresh_shortfalls += 1;
                self.violation(
                    "refresh",
                    format!(
                        "refresh: {} AUTO REFRESH in the last 64 ms, needs 8192",
                        self.refreshes.len()
                    ),
                );
            }
            // Check every refresh interval thereafter, including while CLK stops.
            self.refresh_check = Some(time.saturating_add(WINDOW / 8192));
        }
    }
    /// Feed after every executor instant, including instants without a CLK
    /// transition. X/Z command/address bits are counted and ignored. Unknown
    /// write bytes are left unchanged; they never turn into fabricated data.
    pub fn feed(&mut self, time_ps: u64, pins: &SdramPins) -> Drive {
        self.first_time.get_or_insert(time_ps);
        self.refresh_window(time_ps);
        let previous = self.previous.replace(*pins);
        let Some(setup) = previous else {
            return self.drive;
        };
        if setup.clk != Logic::Zero || pins.clk != Logic::One {
            return self.drive;
        }
        if let Some(last) = self.last_rise.replace(time_ps) {
            self.status.clock_period_ps = time_ps.saturating_sub(last).max(1);
        }
        self.clock += 1;
        if setup.cke != Logic::One {
            // Clock suspend: delay all pipeline work by the suspended edge.
            self.reads = std::mem::take(&mut self.reads)
                .into_iter()
                .map(|(n, b)| (n + 1, b))
                .collect();
            for b in &mut self.banks {
                if let Some(n) = &mut b.auto_close {
                    *n += 1;
                }
            }
            return self.drive;
        }
        for bank in 0..4 {
            if self.banks[bank].auto_close.is_some_and(|n| n <= self.clock) {
                self.close(bank, false);
            }
        }
        let command = Command::decode(&setup);
        if command.is_none() {
            self.status.unknown_samples += 1;
        }
        let command = command.unwrap_or(Command::Nop);
        *self.status.commands.entry(command).or_default() += 1;
        let bank = number(&setup.ba).map(usize::from);
        let address = number(&setup.a);
        let idle = matches!(command, Command::Nop | Command::Deselect);
        if !idle {
            self.check(
                "tRFC",
                command,
                self.last_refresh,
                "AUTO REFRESH",
                self.cycles(66_000, 1),
            );
            self.check("tMRD", command, self.last_mode, "LOAD MODE", 2);
        }
        if !self.status.initialised && !idle {
            let waited = time_ps.saturating_sub(self.first_time.unwrap_or(time_ps)) >= 100_000_000;
            let valid = match (self.init_stage, command) {
                (0, Command::Precharge) if address.is_some_and(|a| a & 1024 != 0) && waited => {
                    self.init_stage = 1;
                    true
                }
                (1 | 2, Command::Refresh) => {
                    self.init_stage += 1;
                    true
                }
                (3, Command::LoadMode) => {
                    self.status.initialised = true;
                    self.refresh_check = Some(time_ps + 64_000_000_000);
                    true
                }
                _ => false,
            };
            if !valid {
                self.status.commands_before_init += 1;
                self.violation("init", format!("init: {} before 100 us NOP, PRECHARGE ALL, two AUTO REFRESH and LOAD MODE (at {time_ps} ps, stage {})", command.label(), self.init_stage));
            }
        }
        if !idle && (bank.is_none() || address.is_none()) {
            self.status.unknown_samples += 1;
        } else if let (Some(bank), Some(a)) = (bank, address) {
            let b = self.banks[bank];
            match command {
                Command::Active => {
                    self.check(
                        "tRP",
                        command,
                        b.precharged,
                        "PRECHARGE",
                        self.cycles(20_000, 2),
                    );
                    self.check("tRC", command, b.active, "ACTIVE", self.cycles(66_000, 1));
                    if b.row.is_some() {
                        self.violation("bank", format!("ACTIVE: bank {bank} already open"));
                    }
                    self.banks[bank] = Bank {
                        row: Some(a & 8191),
                        active: Some(self.clock),
                        written: None,
                        auto_close: None,
                        ..b
                    };
                }
                Command::Read | Command::Write => {
                    self.check("tRCD", command, b.active, "ACTIVE", self.cycles(20_000, 2));
                    if let Some(row) = b.row {
                        if let Some(old) = self.write.take() {
                            if old.auto {
                                self.auto_close(old.bank, self.clock);
                            }
                        }
                        let mode = self.status.mode;
                        let length = if command == Command::Write && mode.single_write {
                            1
                        } else {
                            mode.burst_length
                        };
                        let burst = Burst {
                            bank,
                            row,
                            start: a & 511,
                            beat: 0,
                            length,
                            interleaved: mode.interleaved,
                            auto: a & 1024 != 0,
                        };
                        if command == Command::Read {
                            let first = self.clock + mode.cas_latency - 1;
                            // Old read data continues until the replacement read's first beat.
                            self.reads.retain(|&n, _| n < first);
                            for beat in 0..length {
                                let column = Burst { beat, ..burst }.column();
                                self.reads.insert(
                                    first + u64::from(beat),
                                    ReadBeat { bank, row, column },
                                );
                            }
                            if burst.auto {
                                self.auto_close(bank, self.clock + u64::from(length));
                            }
                        } else {
                            self.reads.clear();
                            self.drive = None;
                            self.drive_mask = 0;
                            self.write = Some(burst);
                        }
                    } else {
                        self.violation(
                            "bank",
                            format!("{}: bank {bank} is closed", command.label()),
                        );
                    }
                }
                Command::BurstTerminate | Command::Precharge => {
                    let all = command == Command::BurstTerminate || a & 1024 != 0;
                    let cutoff = self.clock + self.status.mode.cas_latency - 1;
                    self.reads
                        .retain(|&n, b| n < cutoff || (!all && b.bank != bank));
                    if self.write.is_some_and(|w| all || w.bank == bank) {
                        if let Some(w) = self.write.take() {
                            if w.auto {
                                self.auto_close(w.bank, self.clock);
                            }
                        }
                    }
                    if command == Command::Precharge {
                        for k in 0..4 {
                            if all || k == bank {
                                self.close(k, true);
                            }
                        }
                    }
                }
                Command::Refresh | Command::LoadMode => {
                    for k in 0..4 {
                        if self.banks[k].row.is_some() {
                            self.violation(
                                "bank",
                                format!("{}: bank {k} is open", command.label()),
                            );
                        }
                        self.check(
                            "tRP",
                            command,
                            self.banks[k].precharged,
                            "PRECHARGE",
                            self.cycles(20_000, 2),
                        );
                    }
                    if command == Command::Refresh {
                        self.last_refresh = Some(self.clock);
                        self.refreshes.push_back(time_ps);
                    } else {
                        let length = match a & 7 {
                            0 => 1,
                            1 => 2,
                            2 => 4,
                            3 => 8,
                            7 => 512,
                            _ => 1,
                        };
                        let cl = u64::from((a >> 4) & 7);
                        if ![2, 3].contains(&cl)
                            || ![0, 1, 2, 3, 7].contains(&(a & 7))
                            || a & 0x1d80 != 0
                            || (length == 512 && a & 8 != 0)
                        {
                            self.violation("mode",format!("LOAD MODE: unsupported mode 0x{a:04x}; using CL2/sequential for reserved fields"));
                        }
                        self.status.mode = Mode {
                            raw: a,
                            burst_length: length,
                            interleaved: a & 8 != 0 && length != 512,
                            cas_latency: if cl == 3 { 3 } else { 2 },
                            single_write: a & 512 != 0,
                        };
                        self.last_mode = Some(self.clock);
                    }
                }
                _ => {}
            }
        }
        if let Some(mut w) = self.write.take() {
            let mask = (0..2).fold(0, |m, byte| {
                m | (u8::from(
                    setup.dqm[byte] == Logic::Zero
                        && number(&setup.dq[8 * byte..8 * byte + 8]).is_some(),
                ) << byte)
            });
            let value =
                number(&setup.dq[..8]).unwrap_or(0) | (number(&setup.dq[8..]).unwrap_or(0) << 8);
            self.store(w.bank, w.row, w.column(), value, mask);
            self.status.writes += 1;
            self.banks[w.bank].written = Some(self.clock);
            w.beat += 1;
            if w.beat < w.length {
                self.write = Some(w);
            } else if w.auto {
                self.auto_close(w.bank, self.clock);
            }
        }
        // Publish one edge before capture. Thus the DQM from the previous
        // edge masks data captured two edges after that DQM sample.
        self.drive_mask = 0;
        self.drive = self.reads.remove(&self.clock).and_then(|r| {
            self.status.reads += 1;
            self.drive_mask = !self.dqm & 3;
            (self.drive_mask != 0).then(|| self.peek(r.bank, r.row, r.column))
        });
        self.dqm = (0..2).fold(0, |m, i| m | (u8::from(setup.dqm[i] != Logic::Zero) << i));
        self.drive
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits<const N: usize>(n: u16) -> [Logic; N] {
        std::array::from_fn(|i| Logic::from_bool(n & (1 << i) != 0))
    }
    struct Rig {
        chip: Sdram,
        time: u64,
        pins: SdramPins,
    }
    impl Rig {
        fn new() -> Self {
            let mut r = Self {
                chip: Sdram::new(),
                time: 0,
                pins: SdramPins::default(),
            };
            r.chip.feed(0, &r.pins);
            r
        }
        fn tick(&mut self, command: Command, bank: u16, a: u16, dq: u16, dqm: u16) -> Drive {
            self.pins.clk = Logic::Zero;
            self.pins.ba = bits(bank);
            self.pins.a = bits(a);
            self.pins.dq = bits(dq);
            self.pins.dqm = bits(dqm);
            let (cs, r, c, w) = match command {
                Command::Deselect => (true, true, true, true),
                Command::Nop => (false, true, true, true),
                Command::Active => (false, false, true, true),
                Command::Read => (false, true, false, true),
                Command::Write => (false, true, false, false),
                Command::Precharge => (false, false, true, false),
                Command::Refresh => (false, false, false, true),
                Command::LoadMode => (false, false, false, false),
                Command::BurstTerminate => (false, true, true, false),
            };
            self.pins.csn = Logic::from_bool(cs);
            self.pins.rasn = Logic::from_bool(r);
            self.pins.casn = Logic::from_bool(c);
            self.pins.wen = Logic::from_bool(w);
            self.chip.feed(self.time + 20_000, &self.pins);
            self.time += 40_000;
            self.pins.clk = Logic::One;
            self.chip.feed(self.time, &self.pins)
        }
        fn nop(&mut self) -> Drive {
            self.tick(Command::Nop, 0, 0, 0, 0)
        }
        fn wait(&mut self, n: usize) {
            for _ in 0..n {
                self.nop();
            }
        }
        fn init(&mut self, mode: u16) {
            self.wait(2500);
            self.tick(Command::Precharge, 0, 1024, 0, 0);
            self.wait(2);
            self.tick(Command::Refresh, 0, 0, 0, 0);
            self.wait(2);
            self.tick(Command::Refresh, 0, 0, 0, 0);
            self.wait(2);
            self.tick(Command::LoadMode, 0, mode, 0, 0);
            self.wait(2);
            assert!(self.chip.status().initialised);
        }
        fn active(&mut self) {
            self.tick(Command::Active, 0, 7, 0, 0);
            self.wait(2);
        }
    }
    #[test]
    fn peripherals_init_and_sparse_memory() {
        let mut r = Rig::new();
        r.init(0x20);
        assert_eq!(r.chip.status().timing_violations, 0);
        assert_eq!(r.chip.status().commands[&Command::Refresh], 2);
        assert_eq!(r.chip.peek(3, 8191, 511), 0);
        r.chip.poke(3, 8191, 511, 0xabcd);
        r.chip.poke(3, 8191, 511, 0x1234);
        assert_eq!(r.chip.peek(3, 8191, 511), 0x1234);
        assert_eq!(r.chip.status().bytes_touched, 2);
        assert_eq!(r.chip.status().allocated_rows, 1);
        r.chip.poke(4, 0, 0, 1);
        assert_eq!(r.chip.peek(4, 0, 0), 0);
    }
    #[test]
    fn peripherals_bursts_at_both_cas_latencies() {
        for cl in [2, 3] {
            for (code, len) in [(0, 1), (1, 2), (2, 4), (3, 8), (7, 512)] {
                let mut r = Rig::new();
                r.init(cl * 16 + code);
                r.active();
                r.tick(Command::Write, 0, 510, 0x1000, 0);
                for beat in 1..len {
                    r.tick(Command::Nop, 0, 0, 0x1000 + beat, 0);
                }
                r.wait(3);
                assert_eq!(r.tick(Command::Read, 0, 510, 0, 0), None);
                for _ in 0..cl - 2 {
                    assert_eq!(r.nop(), None);
                }
                for beat in 0..len {
                    assert_eq!(r.nop(), Some(0x1000 + beat), "CL{cl}, BL{len}, beat{beat}");
                }
                assert_eq!(r.nop(), None);
                assert_eq!(r.chip.status().writes, u64::from(len));
                assert_eq!(r.chip.status().reads, u64::from(len));
                assert_eq!(r.chip.status().timing_violations, 0);
            }
        }
    }
    #[test]
    fn peripherals_dqm_writes_and_read_latency() {
        let mut r = Rig::new();
        r.init(0x23);
        r.active();
        r.chip.poke(0, 7, 0, 0x1234);
        r.tick(Command::Write, 0, 0, 0xabcd, 1);
        r.tick(Command::Nop, 0, 0, 0x5678, 2);
        r.tick(Command::Nop, 0, 0, 0xffff, 3);
        r.tick(Command::BurstTerminate, 0, 0, 0, 0);
        assert_eq!(r.chip.peek(0, 7, 0), 0xab34);
        assert_eq!(r.chip.peek(0, 7, 1), 0x0078);
        assert_eq!(r.chip.peek(0, 7, 2), 0);
        r.tick(Command::Read, 0, 0, 0, 1);
        assert_eq!(r.nop(), Some(0xab34));
        assert_eq!(r.chip.drive_mask(), 2);
        assert_eq!(r.tick(Command::Nop, 0, 0, 0, 3), Some(0x0078));
        assert_eq!(r.chip.drive_mask(), 3);
        assert_eq!(r.nop(), None);
        assert_eq!(r.chip.drive_mask(), 0);
        assert_eq!(r.nop(), Some(0));
        assert_eq!(r.chip.drive_mask(), 3);
    }
    #[test]
    fn peripherals_interleaved_auto_precharge_and_single_write() {
        let mut r = Rig::new();
        r.init(0x22b);
        r.active();
        r.tick(Command::Write, 0, 1024 | 3, 0xface, 0);
        r.wait(4);
        assert_eq!(r.chip.status().writes, 1);
        assert_eq!(r.chip.status().open_rows[0], None);
        r.active();
        for col in 0..8 {
            r.chip.poke(0, 7, col, 100 + col);
        }
        r.tick(Command::Read, 0, 1024 | 3, 0, 0);
        for beat in 0..8 {
            assert_eq!(r.nop(), Some(100 + (3 ^ beat)));
        }
        assert_eq!(r.chip.status().open_rows[0], None);
        assert_eq!(r.nop(), None);
        assert_eq!(r.chip.status().timing_violations, 0);
    }
    #[test]
    fn peripherals_read_interrupt_and_burst_terminate_pipeline() {
        for cl in [2, 3] {
            let mut r = Rig::new();
            r.init(cl * 16 + 3);
            r.active();
            for col in 0..32 {
                r.chip.poke(0, 7, col, col + 100);
            }
            r.tick(Command::Read, 0, 0, 0, 0);
            let mut observed = Vec::new();
            for _ in 0..2 {
                if let Some(v) = r.nop() {
                    observed.push(v);
                }
            }
            if let Some(v) = r.tick(Command::Read, 0, 16, 0, 0) {
                observed.push(v);
            }
            for _ in 0..cl - 2 {
                if let Some(v) = r.nop() {
                    observed.push(v);
                }
            }
            assert_eq!(observed, vec![100, 101, 102]);
            assert_eq!(r.nop(), Some(116));
            r.tick(Command::BurstTerminate, 0, 0, 0, 0);
            for _ in 0..cl - 2 {
                assert!(r.nop().is_some());
            }
            assert_eq!(r.nop(), None);
        }
    }
    #[test]
    fn peripherals_zero_delay_unknown_and_cke_suspend() {
        let mut r = Rig::new();
        r.init(0x20);
        r.active();
        r.chip.poke(0, 7, 0, 42);
        r.tick(Command::Read, 0, 0, 0, 0);
        r.pins.cke = Logic::Zero;
        assert_eq!(r.nop(), None);
        r.pins.cke = Logic::One;
        assert_eq!(r.nop(), Some(42));
        r.pins.clk = Logic::Zero;
        r.pins.csn = Logic::Zero;
        r.pins.rasn = Logic::X;
        r.chip.feed(r.time + 20_000, &r.pins);
        r.pins.clk = Logic::One;
        r.time += 40_000;
        r.chip.feed(r.time, &r.pins);
        assert_eq!(r.chip.status().unknown_samples, 1);
        // Current-edge command is NOP but setup was WRITE.
        r.pins = SdramPins::default();
        r.pins.casn = Logic::Zero;
        r.pins.wen = Logic::Zero;
        r.pins.dq = bits(77);
        r.chip.feed(r.time + 20_000, &r.pins);
        r.pins.clk = Logic::One;
        r.pins.casn = Logic::One;
        r.pins.wen = Logic::One;
        r.time += 40_000;
        r.chip.feed(r.time, &r.pins);
        assert_eq!(r.chip.peek(0, 7, 0), 77);
    }
    #[test]
    fn peripherals_timing_diagnostics_have_numbers_once_per_kind() {
        let mut r = Rig::new();
        r.init(0x20);
        r.tick(Command::Active, 0, 0, 0, 0);
        r.tick(Command::Read, 0, 0, 0, 0);
        r.tick(Command::Active, 1, 0, 0, 0);
        r.tick(Command::Active, 1, 1, 0, 0);
        r.tick(Command::Write, 0, 0, 1, 0);
        r.tick(Command::Precharge, 0, 0, 0, 0);
        r.tick(Command::Active, 0, 0, 0, 0);
        r.tick(Command::Precharge, 0, 0, 0, 0);
        r.tick(Command::Active, 0, 0, 0, 0);
        r.tick(Command::Precharge, 0, 1024, 0, 0);
        r.wait(2);
        r.tick(Command::Refresh, 0, 0, 0, 0);
        r.tick(Command::Refresh, 0, 0, 0, 0);
        r.wait(2);
        r.tick(Command::LoadMode, 0, 0x20, 0, 0);
        r.tick(Command::Active, 0, 0, 0, 0);
        let s = r.chip.status();
        for key in ["tRCD", "tRP", "tRC", "tRAS", "tWR", "tRFC", "tMRD"] {
            assert!(
                s.violations.contains_key(key),
                "missing {key}: {:?}",
                s.messages
            );
            let messages: Vec<_> = s
                .messages
                .iter()
                .filter(|m| m.starts_with(&format!("{key}:")))
                .collect();
            assert_eq!(messages.len(), 1);
            assert!(messages[0].contains("1 clocks"));
            assert!(messages[0].contains("needs 2 at 40 ns"));
        }
    }
    #[test]
    fn peripherals_init_violations_and_rolling_refresh_count() {
        let mut early = Rig::new();
        early.tick(Command::Active, 0, 0, 0, 0);
        assert_eq!(early.chip.status().commands_before_init, 1);
        let mut r = Rig::new();
        r.init(0x20);
        for _ in 0..8192 {
            r.tick(Command::Refresh, 0, 0, 0, 0);
            r.wait(2);
        }
        let boundary = r.chip.refresh_check.unwrap();
        r.chip.feed(boundary, &r.pins);
        assert_eq!(r.chip.status().refresh_shortfalls, 0);
        assert_eq!(r.chip.status().refreshes_in_window, 8192);
        r.chip.feed(boundary + 64_000_000_001, &r.pins);
        assert_eq!(r.chip.status().refreshes_in_window, 0);
        assert_eq!(r.chip.status().refresh_shortfalls, 1);
    }
}
