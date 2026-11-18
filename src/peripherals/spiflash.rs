//! ISSI IS25LP128F configuration flash (16 MB, three-byte addresses).
//! SPI modes 0/3; writes affect only copy-on-write memory, never an image file.
//! Command/bit references below use ISSI Rev. A5, 10 October 2019;
//! docs/PERIPHERALS_FLASH.md links the datasheet and lists modelling assumptions.
use std::{fmt, sync::Arc};

pub const CAPACITY: usize = 16 * 1024 * 1024;
pub const JEDEC: [u8; 3] = [0x9d, 0x60, 0x18];
pub const FIRMWARE_OFFSET: usize = 0x200000;
const SECTOR_BYTES: usize = 4096;
const SECTORS: usize = CAPACITY / SECTOR_BYTES;

/// End, exclusive, of the conservative FPGA configuration area: 0x250000.
/// Trellis devices.json: 13294 * 1136 / 8 CRAM bytes; the repository's
/// core_model_summary.json: 208 EBRs, each 2048 * 9 bits. Bitstream.cpp's
/// serialise_chip adds 3 bytes/frame, 14 bytes/EBR and 73 bytes of fixed
/// commands/header (including optional SPI_MODE). Round up to 64 KB.
/// Assumption: standard Trellis serialization, without arbitrary-length
/// metadata comments or extra user-inserted commands; see the arithmetic doc.
pub const CONFIGURATION_END: usize =
    (13294_usize * 1136 / 8 + 208 * 2048 * 9 / 8 + 13294 * 3 + 208 * 14 + 73).div_ceil(65536)
        * 65536;

// Datasheet §9.6 AC Characteristics (unnumbered table), pp. 166–167, and
// §9.9 Program/Erase Performance, p. 171: typical times, at 25°C / typical VCC.
// The erase rows are named tEC there; these names distinguish their sizes.
// Assumption: partial pages also use the full-page tPP interval.
pub const T_PP_PS: u64 = 200_000_000; // 0.2 ms; §8.10, §9.6 tPP.
pub const T_SE_PS: u64 = 100_000_000_000; // 100 ms; §8.13, §9.6 tEC.
pub const T_BE32_PS: u64 = 140_000_000_000; // 0.14 s; §8.14, §9.6 tEC.
pub const T_BE64_PS: u64 = 170_000_000_000; // 0.17 s; §8.14, §9.6 tEC.
pub const T_CE_PS: u64 = 35_000_000_000_000; // 35 s; §8.15, §9.6 tEC.
pub const T_WRSR_PS: u64 = 2_000_000_000; // 2 ms; §8.19, §9.6 tW.

// §6.1, Tables 6.1/6.2: volatile WIP/WEL, then BP0..3, QE and SRWD.
const WIP: u8 = 1;
const WEL: u8 = 2;
const BP: u8 = 0x3c;

/// Pad levels: 0/1 are known, 2 is X, 3 is Z. CS high ends a transaction.
#[derive(Clone, Copy, Debug)]
pub struct Pins {
    pub cs: u8,
    pub clk: u8,
    pub mosi: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigWrite {
    pub command: u8,
    /// Affected address envelope, start inclusive and end exclusive.
    pub start: u32,
    pub end: u32,
}
impl fmt::Display for ConfigWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "command {:02X} range [{:06X},{:06X}): this would overwrite the board's FPGA configuration (bitstream) on a real ULX3S",
            self.command, self.start, self.end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub jedec: [u8; 3],
    pub selected: bool,
    pub power_down: bool,
    pub register: u8,
    pub last_command: Option<u8>,
    pub last_address: Option<u32>,
    pub bytes_read: u64,
    /// Rejected write/control commands: unsupported, malformed, no WEL,
    /// protected or busy. Each command counts at most once; accepted WREN,
    /// WRDI, WRSR, programs and erases do not increment this counter.
    pub ignored_writes: u64,
    /// Completed page programs and erase commands (not bytes or sectors).
    pub programs: u64,
    pub erases: u64,
    pub protected: u64,
    /// Commands refused while WIP=1, including reads other than permitted polls.
    pub busy_ignored: u64,
    /// Well-formed array-write attempts touching the configuration area,
    /// including attempts rejected for WEL, BP protection or busy.
    pub config_writes: u64,
    pub first_config_write: Option<Arc<ConfigWrite>>,
    pub version: u64,
}

#[derive(Clone, Copy)]
enum Output {
    Id,
    Device,
    Manufacturer,
    Memory,
    Status,
    Function,
    Extended,
}
#[derive(Clone, Copy)]
enum Phase {
    Command,
    Address(u8),
    Dummy(u8),
    Output(Output),
    Program,
    Register,
    Ignore,
}

type Sector = [u8; SECTOR_BYTES];
type SectorIndex = Vec<Option<Arc<Sector>>>;
enum Operation {
    Program {
        page: usize,
        data: Arc<[u8; 256]>,
        seen: Box<[bool; 256]>,
    },
    Erase {
        start: usize,
        end: usize,
    },
    Status(u8),
}
struct Pending {
    finish_ps: u64,
    operation: Operation,
}

#[derive(Clone)]
pub struct SpiFlash {
    base: Arc<[u8]>,
    /// Cloning a checkpoint shares this 4096-entry index. A later mutation
    /// copies at most 32 KB of pointers and one 4 KB sector per changed sector.
    sectors: Arc<SectorIndex>,
    erased_sector: Arc<Sector>,
    base_erased: bool,
    status: Status,
    previous: Pins,
    phase: Phase,
    shift: u8,
    bits: u8,
    address: u32,
    address_ready: bool,
    output: u8,
    output_left: u8,
    output_index: usize,
    output_started: bool,
    miso: u8,
    reset_enabled: bool,
    action: Option<u8>,
    clocks: u64,
    valid: bool,
    command_busy: bool,
    rejected: bool,
    page: Arc<[u8; 256]>,
    seen: [bool; 256],
    page_bytes: usize,
    status_byte: Option<u8>,
    pending: Option<Arc<Pending>>,
    extended_errors: u8,
}

impl SpiFlash {
    /// Pad an image with erased bytes to 16 MB; excess bytes are discarded.
    pub fn new(image: Vec<u8>) -> Self {
        Self::with_status(image, 0)
    }

    /// Configure BP/QE/SRWD from the source image. WIP/WEL are volatile and
    /// start clear (§6.1, Tables 6.1/6.2); WRSR changes bits 2..7 in memory.
    /// TBS defaults to top protection (§6.2, Tables 6.5/6.6). WP# is assumed
    /// high, as in the reference designs, so SRWD alone does not block WRSR
    /// (§7.1, Table 7.1). WP#/HOLD pad timing is not modelled.
    pub fn with_status(mut image: Vec<u8>, register: u8) -> Self {
        image.resize(CAPACITY, 0xff);
        Self {
            base: image.into(),
            sectors: Arc::new(vec![None; SECTORS]),
            erased_sector: Arc::new([0xff; SECTOR_BYTES]),
            base_erased: false,
            status: Status {
                jedec: JEDEC,
                selected: false,
                power_down: false,
                register: register & !3,
                last_command: None,
                last_address: None,
                bytes_read: 0,
                ignored_writes: 0,
                programs: 0,
                erases: 0,
                protected: 0,
                busy_ignored: 0,
                config_writes: 0,
                first_config_write: None,
                version: 0,
            },
            previous: Pins {
                cs: 1,
                clk: 0,
                mosi: 1,
            },
            phase: Phase::Command,
            shift: 0,
            bits: 0,
            address: 0,
            address_ready: false,
            output: 0xff,
            output_left: 0,
            output_index: 0,
            output_started: false,
            miso: 1,
            reset_enabled: false,
            action: None,
            clocks: 0,
            valid: true,
            command_busy: false,
            rejected: false,
            page: Arc::new([0xff; 256]),
            seen: [false; 256],
            page_bytes: 0,
            status_byte: None,
            pending: None,
            extended_errors: 0,
        }
    }
    /// A deterministic erased image with the reference XIP firmware at 0x200000.
    /// RVFW is followed by little-endian length and CRC-32, then the payload.
    pub fn demo() -> Self {
        let hex = include_str!("../../examples/ulx3s/picorv32_flash/firmware.hex");
        let payload: Vec<u8> = hex
            .split_whitespace()
            .map(|v| u8::from_str_radix(v, 16).expect("committed flash firmware hex"))
            .collect();
        let mut image = vec![0xff; FIRMWARE_OFFSET];
        image.extend_from_slice(b"RVFW");
        image.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        image.extend_from_slice(&super::video::crc32(&payload).to_le_bytes());
        image.extend(payload);
        Self::new(image)
    }
    /// ECP5_FLASH_IMAGE names a raw flash image, else use the demo firmware.
    pub fn from_env() -> Self {
        match std::env::var_os("ECP5_FLASH_IMAGE").map(std::fs::read) {
            Some(Ok(image)) => Self::new(image),
            Some(Err(e)) => {
                eprintln!("SPI flash: ECP5_FLASH_IMAGE not read ({e}); demo image instead");
                Self::demo()
            }
            None => Self::demo(),
        }
    }
    /// Materialize the current image on explicit request. With no changes,
    /// return the shared base; after writes this allocates 16 MB. Snapshots
    /// clone the model directly and never call this method.
    pub fn image(&self) -> Arc<[u8]> {
        if !self.base_erased && self.sectors.iter().all(Option::is_none) {
            return Arc::clone(&self.base);
        }
        (0..CAPACITY).map(|a| self.read(a)).collect()
    }
    fn read(&self, address: usize) -> u8 {
        self.sectors[address / SECTOR_BYTES].as_ref().map_or_else(
            || {
                if self.base_erased {
                    0xff
                } else {
                    self.base[address]
                }
            },
            |sector| sector[address % SECTOR_BYTES],
        )
    }
    pub fn miso(&self) -> u8 {
        self.miso
    }
    pub fn selected(&self) -> bool {
        self.status.selected
    }
    pub fn status(&self) -> Status {
        self.status.clone()
    }
    pub fn version(&self) -> u64 {
        self.status.version
    }

    /// Advance busy time even for identical/deselected samples. Operations
    /// start at CS rising and commit at their typical completion deadline.
    /// Assumption: contents stay old until completion; reset cancels a pending
    /// write without corrupting data (§8.37 permits corruption on real silicon).
    fn advance(&mut self, time_ps: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| time_ps >= p.finish_ps)
        {
            let pending = self.pending.take().unwrap();
            match &pending.operation {
                Operation::Program { page, data, seen } => {
                    let sector_index = page / SECTOR_BYTES;
                    let initial = self.sectors[sector_index].clone().unwrap_or_else(|| {
                        let mut sector = [0xff; SECTOR_BYTES];
                        if !self.base_erased {
                            sector.copy_from_slice(
                                &self.base[sector_index * SECTOR_BYTES
                                    ..(sector_index + 1) * SECTOR_BYTES],
                            );
                        }
                        Arc::new(sector)
                    });
                    let sector =
                        Arc::make_mut(&mut self.sectors)[sector_index].get_or_insert(initial);
                    let sector = Arc::make_mut(sector);
                    for (i, &touched) in seen.iter().enumerate() {
                        if touched {
                            sector[page % SECTOR_BYTES + i] &= data[i];
                        }
                    }
                    self.status.programs += 1;
                }
                Operation::Erase {
                    start: 0,
                    end: CAPACITY,
                } => {
                    // Chip erase needs no 16 MB copy, even across checkpoints.
                    self.base_erased = true;
                    self.sectors = Arc::new(vec![None; SECTORS]);
                    self.status.erases += 1;
                }
                Operation::Erase { start, end } => {
                    let sectors = Arc::make_mut(&mut self.sectors);
                    for sector in &mut sectors[start / SECTOR_BYTES..end / SECTOR_BYTES] {
                        *sector = Some(Arc::clone(&self.erased_sector));
                    }
                    self.status.erases += 1;
                }
                Operation::Status(value) => self.status.register = *value & !3,
            }
            self.status.register &= !(WIP | WEL);
            self.status.version += 1;
        }
    }
    fn reject_write(&mut self) {
        if !self.rejected {
            self.rejected = true;
            self.status.ignored_writes += 1;
            self.status.version += 1;
        }
    }
    fn write_command(command: u8) -> bool {
        matches!(
            command,
            0x06 | 0x04
                | 0x02
                | 0x32
                | 0x38
                | 0xad
                | 0x20
                | 0xd7
                | 0x52
                | 0xd8
                | 0xc7
                | 0x60
                | 0x01
                | 0x31
                | 0x11
                | 0x50
                | 0x42
                | 0x44
                | 0x7e
                | 0x98
                | 0x36
                | 0x39
                | 0xa6
                | 0x68
                | 0xb1
                | 0xc1
        )
    }
    /// §6.1 Table 6.4, standard (not ordering-option B) BP table, TBS=0:
    /// 0 none; 1..8 protect 1,2,4,..128 top 64 KB blocks; 9..15 all.
    /// The OTP TBS bit is fixed at its factory default 0; WRFR stays unsupported.
    /// Assumption: standard ordering, factory TBS and unlocked ASP sectors;
    /// HR07 did not read these options or fuses.
    fn protected(&self, start: usize, end: usize) -> bool {
        let bp = (self.status.register & BP) >> 2;
        let bytes = match bp {
            0 => 0,
            1..=8 => 65536 << (bp - 1),
            _ => CAPACITY,
        };
        bytes != 0 && end > CAPACITY - bytes && start < CAPACITY
    }
    fn warn_configuration(&mut self, command: u8, start: usize, end: usize) {
        if start < CONFIGURATION_END && start < end {
            self.status.config_writes += 1;
            self.status.first_config_write.get_or_insert_with(|| {
                Arc::new(ConfigWrite {
                    command,
                    start: start as u32,
                    end: end as u32,
                })
            });
            self.status.version += 1;
        }
    }
    fn start_operation(&mut self, now: u64, duration: u64, operation: Operation) {
        self.pending = Some(Arc::new(Pending {
            finish_ps: now.saturating_add(duration),
            operation,
        }));
        self.status.register |= WIP;
        self.status.version += 1;
    }
    fn finish(&mut self, now: u64) {
        let Some(command) = self.action.take() else {
            return;
        };
        let complete = self.valid && self.clocks.is_multiple_of(8);
        // §8.10: >256 data bytes replace earlier page-buffer bytes, not
        // additional AND passes. Page wrap is confined to A7..A0.
        let array_write = match command {
            0x02 if complete && self.address_ready && self.page_bytes != 0 => {
                let page = self.address as usize & !255;
                let first = self.seen.iter().position(|&x| x).unwrap();
                let last = self.seen.iter().rposition(|&x| x).unwrap();
                Some((page + first, page + last + 1, T_PP_PS))
            }
            0x20 | 0xd7 | 0x52 | 0xd8 if complete && self.clocks == 32 => {
                let (size, time) = match command {
                    0x20 | 0xd7 => (4096, T_SE_PS),
                    0x52 => (32768, T_BE32_PS),
                    _ => (65536, T_BE64_PS),
                };
                let start = self.address as usize & !(size - 1);
                Some((start, start + size, time))
            }
            0xc7 | 0x60 if complete && self.clocks == 8 => Some((0, CAPACITY, T_CE_PS)),
            _ => None,
        };
        if let Some((start, end, duration)) = array_write {
            // Warn even when BP, WEL or busy prevents the operation.
            self.warn_configuration(command, start, end);
            if self.command_busy {
                return;
            }
            if self.protected(start, end) {
                self.status.protected += 1;
                // §6.3.2 Tables 6.12/6.13: protection plus program/erase error.
                self.extended_errors |= if command == 0x02 { 0x06 } else { 0x0a };
                // Assumption: a BP-rejected operation is instantaneous and
                // preserves WEL; the datasheet gives no BP-rejection latency.
                self.reject_write();
            } else if self.status.register & WEL == 0 {
                self.reject_write();
            } else {
                let operation = if command == 0x02 {
                    Operation::Program {
                        page: self.address as usize & !255,
                        data: Arc::clone(&self.page),
                        seen: Box::new(self.seen),
                    }
                } else {
                    Operation::Erase { start, end }
                };
                self.start_operation(now, duration, operation);
            }
            return;
        }
        if self.command_busy {
            return;
        }
        match command {
            0x06 if complete && self.clocks == 8 => self.status.register |= WEL,
            0x04 if complete && self.clocks == 8 => self.status.register &= !WEL,
            0x01 if complete
                && self.clocks == 16
                && self.status_byte.is_some()
                && self.status.register & WEL != 0 =>
            {
                // §8.19 / Table 6.2: WRSR cannot write WIP/WEL. WP# is high.
                self.start_operation(now, T_WRSR_PS, Operation::Status(self.status_byte.unwrap()));
            }
            0x66 if complete && self.clocks == 8 => self.reset_enabled = true,
            0x99 if complete && self.clocks == 8 => {
                self.pending = None;
                self.status.register &= !(WIP | WEL);
                self.extended_errors = 0;
                self.status.power_down = false;
            }
            0xb9 if complete && self.clocks == 8 => self.status.power_down = true,
            0xab if complete => self.status.power_down = false,
            _ if Self::write_command(command) => self.reject_write(),
            _ => {}
        }
        self.status.version += 1;
    }
    /// Feed in time order, in picoseconds. CS rising starts complete writes;
    /// unknown CS deselects but never commits a write. Unknown clock/MOSI
    /// invalidates this transaction. Identical samples still advance WIP time.
    pub fn feed(&mut self, time_ps: u64, pins: Pins) {
        self.advance(time_ps);
        let old = self.previous;
        self.previous = pins;
        if pins.cs != 0 {
            if self.status.selected {
                if pins.cs == 1 {
                    self.finish(time_ps);
                } else if self.action.is_some_and(Self::write_command) {
                    self.reject_write();
                }
                self.status.selected = false;
                self.status.version += 1;
            }
            self.miso = 1;
            return;
        }
        if old.cs != 0 {
            self.phase = Phase::Command;
            self.shift = 0;
            self.bits = 0;
            self.address = 0;
            self.address_ready = false;
            self.output_left = 0;
            self.output_index = 0;
            self.output_started = false;
            self.action = None;
            self.clocks = 0;
            self.valid = true;
            self.command_busy = false;
            self.rejected = false;
            self.seen = [false; 256];
            self.page_bytes = 0;
            self.status_byte = None;
            self.miso = 1;
            self.status.selected = true;
            self.status.version += 1;
            return;
        }
        if pins.clk > 1 || old.clk > 1 {
            self.phase = Phase::Ignore;
            self.valid = false;
            self.miso = 1;
        } else if old.clk == 0 && pins.clk == 1 {
            self.clocks += 1;
            match self.phase {
                Phase::Output(_) => {
                    if self.output_started && self.output_left == 0 {
                        self.status.bytes_read += 1;
                        self.status.version += 1;
                    }
                }
                Phase::Dummy(left) => {
                    self.phase = if left == 1 {
                        Phase::Output(Output::Memory)
                    } else {
                        Phase::Dummy(left - 1)
                    }
                }
                Phase::Ignore => {}
                _ if pins.mosi > 1 => {
                    self.phase = Phase::Ignore;
                    self.valid = false;
                    self.miso = 1;
                }
                _ => {
                    self.shift = (self.shift << 1) | pins.mosi;
                    self.bits += 1;
                    if self.bits == 8 {
                        let byte = self.shift;
                        self.bits = 0;
                        self.shift = 0;
                        self.byte(byte);
                    }
                }
            }
        } else if old.clk == 1 && pins.clk == 0 {
            if let Phase::Output(kind) = self.phase {
                if self.output_left == 0 {
                    self.output = match kind {
                        Output::Id => JEDEC[self.output_index % 3],
                        Output::Device => 0x17,
                        Output::Manufacturer => {
                            if (self.output_index as u32 + self.address) & 1 == 0 {
                                0x9d
                            } else {
                                0x17
                            }
                        }
                        // Assumption: poll data latch at each outgoing-byte boundary.
                        Output::Status => self.status.register,
                        // §6.2 Tables 6.5/6.6: TBS remains 0 (top); other
                        // function bits are not modelled and read 0 (assumption).
                        Output::Function => 0,
                        Output::Extended => {
                            0xf0 | self.extended_errors | (self.status.register & WIP)
                        }
                        Output::Memory => {
                            let byte = self.read(self.address as usize);
                            self.address = (self.address + 1) & 0xffffff;
                            byte
                        }
                    };
                    self.output_index = self.output_index.wrapping_add(1);
                    self.output_left = 8;
                    self.output_started = true;
                }
                self.miso = self.output >> 7;
                self.output <<= 1;
                self.output_left -= 1;
            }
        }
    }
    fn byte(&mut self, byte: u8) {
        match self.phase {
            Phase::Command => {
                self.status.last_command = Some(byte);
                self.status.last_address = None;
                self.status.version += 1;
                self.phase = Phase::Ignore;
                if self.status.power_down && byte != 0xab {
                    return;
                }
                let reset = self.reset_enabled;
                self.reset_enabled = false;
                // §6.1 WIP paragraph permits register polls and reset while
                // busy. Suspend/resume is deliberately not implemented.
                self.command_busy =
                    self.pending.is_some() && !matches!(byte, 0x05 | 0x48 | 0x81 | 0x66 | 0x99);
                if self.command_busy {
                    self.status.busy_ignored += 1;
                    if Self::write_command(byte) {
                        self.reject_write();
                    }
                    self.status.version += 1;
                }
                // Array writers still collect addresses/data while busy so
                // the configuration-area warning also covers refused writes.
                if self.command_busy
                    && !matches!(byte, 0x02 | 0x20 | 0xd7 | 0x52 | 0xd8 | 0xc7 | 0x60)
                {
                    return;
                }
                match byte {
                    0x9f => self.phase = Phase::Output(Output::Id),
                    0x05 => self.phase = Phase::Output(Output::Status),
                    0x48 => self.phase = Phase::Output(Output::Function),
                    0x81 => self.phase = Phase::Output(Output::Extended),
                    0x03 | 0x0b | 0x90 | 0xab | 0x02 | 0x20 | 0xd7 | 0x52 | 0xd8 => {
                        self.phase = Phase::Address(3);
                        self.action = Some(byte);
                    }
                    0x01 => {
                        self.phase = Phase::Register;
                        self.action = Some(byte);
                    }
                    0x06 | 0x04 | 0x66 | 0xb9 | 0xc7 | 0x60 => self.action = Some(byte),
                    0x99 if reset => self.action = Some(byte),
                    _ if Self::write_command(byte) => self.reject_write(),
                    _ => {}
                }
            }
            Phase::Address(left) => {
                self.address = (self.address << 8) | u32::from(byte);
                if left == 1 {
                    self.address_ready = true;
                    self.status.last_address = Some(self.address);
                    self.status.version += 1;
                    self.phase = match self.status.last_command {
                        Some(0x0b) => Phase::Dummy(8),
                        Some(0x90) => Phase::Output(Output::Manufacturer),
                        Some(0xab) => Phase::Output(Output::Device),
                        Some(0x02) => Phase::Program,
                        Some(0x03) => Phase::Output(Output::Memory),
                        _ => Phase::Ignore,
                    };
                } else {
                    self.phase = Phase::Address(left - 1);
                }
            }
            Phase::Program => {
                let offset = (self.address as usize + self.page_bytes) & 255;
                Arc::make_mut(&mut self.page)[offset] = byte;
                self.seen[offset] = true;
                self.page_bytes = self.page_bytes.saturating_add(1);
            }
            Phase::Register => {
                self.status_byte = Some(byte);
                self.phase = Phase::Ignore;
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Host {
        flash: SpiFlash,
        pins: Pins,
        time: u64,
        idle: u8,
    }
    impl Host {
        fn new(flash: SpiFlash, mode3: bool) -> Self {
            let pins = Pins {
                cs: 1,
                clk: u8::from(mode3),
                mosi: 1,
            };
            let mut h = Self {
                flash,
                pins,
                time: 0,
                idle: u8::from(mode3),
            };
            h.feed();
            h
        }
        fn feed(&mut self) {
            self.time += 10;
            self.flash.feed(self.time, self.pins);
        }
        fn select(&mut self) {
            self.pins.cs = 0;
            self.feed();
        }
        fn end(&mut self) {
            self.pins.cs = 1;
            self.feed();
            self.pins.clk = self.idle;
            self.feed();
        }
        fn byte(&mut self, byte: u8) -> u8 {
            let mut got = 0;
            for bit in (0..8).rev() {
                self.pins.clk = 0;
                self.pins.mosi = (byte >> bit) & 1;
                self.feed();
                // In mode 0 the final falling edge of the previous byte has
                // already presented this bit. In mode 3 it happens here.
                self.pins.clk = 1;
                self.feed();
                got = (got << 1) | self.flash.miso();
            }
            got
        }
        fn command(&mut self, bytes: &[u8], count: usize) -> Vec<u8> {
            self.select();
            for &byte in bytes {
                self.byte(byte);
            }
            let out = (0..count).map(|_| self.byte(0xff)).collect();
            self.end();
            out
        }
    }
    #[test]
    fn ids_status_and_spi_modes() {
        for mode3 in [false, true] {
            let mut h = Host::new(SpiFlash::new(Vec::new()), mode3);
            assert_eq!(h.command(&[0x9f], 6), [0x9d, 0x60, 0x18, 0x9d, 0x60, 0x18]);
            assert_eq!(h.command(&[0x05], 3), [0; 3]);
            assert_eq!(h.command(&[0xab, 0, 0, 0], 2), [0x17; 2]);
            assert_eq!(h.command(&[0x90, 0, 0, 0], 4), [0x9d, 0x17, 0x9d, 0x17]);
            assert_eq!(h.command(&[0x90, 0, 0, 1], 2), [0x17, 0x9d]);
        }
        let mut h = Host::new(SpiFlash::with_status(Vec::new(), 0xa7), false);
        assert_eq!(h.command(&[0x05], 2), [0xa4; 2]);
    }
    #[test]
    fn read_fast_read_padding_and_address_wrap() {
        let mut image = vec![0xff; CAPACITY];
        image[..4].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
        image[CAPACITY - 1] = 0xab;
        let mut h = Host::new(SpiFlash::new(image), false);
        assert_eq!(h.command(&[0x03, 0, 0, 0], 4), [0x12, 0x34, 0x56, 0x78]);
        assert_eq!(h.command(&[0x0b, 0, 0, 1, 0], 3), [0x34, 0x56, 0x78]);
        assert_eq!(h.command(&[0x03, 0xff, 0xff, 0xff], 3), [0xab, 0x12, 0x34]);
        assert_eq!(h.flash.status().bytes_read, 10);
        let small = SpiFlash::new(vec![1]);
        assert_eq!(small.image().len(), CAPACITY);
        assert_eq!(&small.image()[..3], &[1, 0xff, 0xff]);
    }
    #[test]
    fn partial_commands_and_reads_abort_on_cs_or_unknown_clock() {
        let mut h = Host::new(SpiFlash::new(vec![0x12, 0x34]), false);
        h.select();
        for bit in [1, 0, 0] {
            h.pins.clk = 0;
            h.pins.mosi = bit;
            h.feed();
            h.pins.clk = 1;
            h.feed();
        }
        h.end();
        assert!(h.flash.status().last_command.is_none());
        assert_eq!(h.command(&[0x03, 0, 0, 0], 1), [0x12]);
        h.select();
        h.byte(0x9f);
        h.pins.clk = 0;
        h.feed();
        h.pins.clk = 1;
        h.feed();
        let saved = h.flash.clone();
        let pins = h.pins;
        let tail = h.byte(0xff);
        h.flash = saved;
        h.pins = pins;
        assert_eq!(h.byte(0xff), tail);
        h.pins.clk = 2;
        h.feed();
        assert_eq!(h.flash.miso(), 1);
        assert_eq!(h.byte(0xff), 0xff);
        h.end();
        assert_eq!(h.command(&[0x9f], 3), JEDEC);
        // A read restarted without all three address bytes returns idle FF.
        assert!(h.command(&[0x03, 0, 0], 0).is_empty());
        assert_eq!(h.command(&[0x03, 0, 0, 1], 1), [0x34]);
    }

    #[test]
    fn power_down_release_and_reset_sequence() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), true);
        h.command(&[0xb9], 0);
        assert!(h.flash.status().power_down);
        assert_eq!(h.command(&[0x9f], 3), [0xff; 3]);
        assert_eq!(h.command(&[0xab, 0, 0, 0], 1), [0x17]);
        assert!(!h.flash.status().power_down);
        h.command(&[0x66], 0);
        h.command(&[0x99], 0);
        assert!(!h.flash.reset_enabled);
        assert_eq!(h.command(&[0x9f], 3), JEDEC);
    }
    #[test]
    fn deselect_unknowns_and_checkpoint_mid_byte() {
        let mut h = Host::new(SpiFlash::demo(), false);
        h.select();
        h.byte(0x9f);
        let saved = h.flash.clone();
        assert!(Arc::ptr_eq(&saved.base, &h.flash.base));
        assert_eq!(h.byte(0xff), 0x9d);
        h.flash = saved;
        assert_eq!(h.byte(0xff), 0x9d);
        h.end();
        assert_eq!(h.flash.miso(), 1);
        assert!(!h.flash.selected());
        h.select();
        h.pins.mosi = 2;
        h.pins.clk = 0;
        h.feed();
        h.pins.clk = 1;
        h.feed();
        assert_eq!(h.byte(0xff), 0xff);
        h.end();
        assert_eq!(h.command(&[0x9f], 3), JEDEC);
        assert_eq!(
            &h.flash.image()[FIRMWARE_OFFSET..FIRMWARE_OFFSET + 4],
            b"RVFW"
        );
        let image = h.flash.image();
        let len = u32::from_le_bytes(
            image[FIRMWARE_OFFSET + 4..FIRMWARE_OFFSET + 8]
                .try_into()
                .unwrap(),
        ) as usize;
        assert_eq!(
            u32::from_le_bytes(
                image[FIRMWARE_OFFSET + 8..FIRMWARE_OFFSET + 12]
                    .try_into()
                    .unwrap()
            ),
            super::super::video::crc32(&image[FIRMWARE_OFFSET + 12..FIRMWARE_OFFSET + 12 + len])
        );
    }
    impl Host {
        fn until(&mut self, time: u64) {
            assert!(time >= self.time);
            self.time = time;
            self.flash.feed(self.time, self.pins);
        }
        fn finish_write(&mut self) {
            let deadline = self
                .flash
                .pending
                .as_ref()
                .expect("pending write")
                .finish_ps;
            self.until(deadline);
        }
        fn program(&mut self, address: usize, data: &[u8]) {
            self.select();
            for byte in [
                0x02,
                (address >> 16) as u8,
                (address >> 8) as u8,
                address as u8,
            ] {
                self.byte(byte);
            }
            for &byte in data {
                self.byte(byte);
            }
            self.end();
        }
        fn erase(&mut self, opcode: u8, address: usize) {
            self.command(
                &[
                    opcode,
                    (address >> 16) as u8,
                    (address >> 8) as u8,
                    address as u8,
                ],
                0,
            );
        }
        fn enable(&mut self) {
            self.command(&[0x06], 0);
        }
    }
    const DATA: usize = 0x300000;

    #[test]
    fn wel_gates_array_and_status_writes_and_wrdi_clears_it() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        h.program(DATA, &[0x12]);
        h.erase(0x20, DATA);
        h.command(&[0x01, 0xfc], 0);
        assert_eq!(h.flash.status().ignored_writes, 3);
        assert!(h.flash.pending.is_none());
        assert_eq!(h.flash.read(DATA), 0xff);
        h.enable();
        assert_eq!(h.command(&[0x05], 1), [WEL]);
        h.command(&[0x04], 0);
        assert_eq!(h.command(&[0x05], 1), [0]);
        h.program(DATA, &[0]);
        assert_eq!(h.flash.status().ignored_writes, 4);
        h.enable();
        h.program(DATA, &[0x12]);
        assert_eq!(h.flash.status().register & 3, WIP | WEL);
        assert_eq!(h.flash.read(DATA), 0xff, "commit only at completion");
        h.finish_write();
        assert_eq!(h.flash.read(DATA), 0x12);
        assert_eq!(h.flash.status().register & 3, 0);
        assert_eq!(h.flash.status().programs, 1);
    }
    #[test]
    fn page_program_wraps_replaces_latched_bytes_and_only_clears_image_bits() {
        for mode3 in [false, true] {
            let mut h = Host::new(SpiFlash::new(Vec::new()), mode3);
            h.enable();
            h.program(DATA + 254, &[0xf0, 0x0f, 0xaa, 0x55]);
            h.finish_write();
            assert_eq!(
                [
                    h.flash.read(DATA + 254),
                    h.flash.read(DATA + 255),
                    h.flash.read(DATA),
                    h.flash.read(DATA + 1)
                ],
                [0xf0, 0x0f, 0xaa, 0x55]
            );
            assert_eq!(h.flash.read(DATA + 256), 0xff);
            h.enable();
            h.program(DATA + 254, &[0x7f, 0xff]);
            h.finish_write();
            assert_eq!(h.flash.read(DATA + 254), 0x70);
            assert_eq!(h.flash.read(DATA + 255), 0x0f);
            let mut bytes = vec![0; 256];
            bytes.extend([0xff; 256]);
            h.enable();
            h.program(DATA + 512 + 7, &bytes);
            h.finish_write();
            assert!(
                (DATA + 512..DATA + 768).all(|a| h.flash.read(a) == 0xff),
                "last 256 latched bytes win before AND"
            );
            assert_eq!(h.command(&[0x03, 0x30, 0, 0], 2), [0xaa, 0x55]);
        }
    }
    #[test]
    fn each_erase_size_and_alias_erases_only_the_aligned_target() {
        for (opcode, size, time) in [
            (0x20, 4096, T_SE_PS),
            (0xd7, 4096, T_SE_PS),
            (0x52, 32768, T_BE32_PS),
            (0xd8, 65536, T_BE64_PS),
        ] {
            let mut h = Host::new(SpiFlash::new(vec![0; CAPACITY]), false);
            h.enable();
            h.erase(opcode, DATA + 123);
            let deadline = h.flash.pending.as_ref().unwrap().finish_ps;
            assert_eq!(deadline - (h.time - 10), time);
            h.until(deadline - 1);
            assert_eq!(h.flash.status().register & WIP, WIP);
            assert_eq!(h.flash.read(DATA), 0);
            h.until(deadline);
            assert_eq!(h.flash.status().register & 3, 0);
            assert_eq!(h.flash.status().erases, 1);
            assert_eq!(h.flash.read(DATA - 1), 0);
            assert!((DATA..DATA + size).all(|a| h.flash.read(a) == 0xff));
            assert_eq!(h.flash.read(DATA + size), 0);
        }
        for opcode in [0xc7, 0x60] {
            let mut h = Host::new(SpiFlash::new(vec![0; CAPACITY]), false);
            h.enable();
            h.command(&[opcode], 0);
            assert_eq!(
                h.flash.pending.as_ref().unwrap().finish_ps - (h.time - 10),
                T_CE_PS
            );
            h.finish_write();
            assert_eq!((h.flash.read(0), h.flash.read(CAPACITY - 1)), (0xff, 0xff));
            assert_eq!(h.flash.status().erases, 1);
            assert!(h.flash.base_erased);
            assert!(h.flash.sectors.iter().all(Option::is_none));
        }
    }
    #[test]
    fn busy_time_uses_pad_timestamps_and_only_allows_polls_and_reset() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        h.enable();
        h.program(DATA, &[0]);
        let deadline = h.flash.pending.as_ref().unwrap().finish_ps;
        assert_eq!(deadline - (h.time - 10), T_PP_PS);
        assert_eq!(h.command(&[0x05], 1), [3]);
        assert_eq!(h.command(&[0x48], 1), [0]);
        assert_eq!(h.command(&[0x81], 1), [0xf1]);
        assert_eq!(h.command(&[0x9f], 3), [0xff; 3]);
        assert_eq!(h.command(&[0x03, 0x30, 0, 0], 1), [0xff]);
        h.command(&[0x06], 0);
        h.command(&[0x04], 0);
        h.command(&[0xb9], 0);
        h.program(DATA + 1, &[0]);
        assert_eq!(h.flash.status().busy_ignored, 6);
        assert_eq!(h.flash.status().ignored_writes, 3);
        assert!(!h.flash.status().power_down);
        h.until(deadline - 1);
        assert_eq!(h.flash.status().register & 3, 3);
        h.until(deadline);
        assert_eq!(h.command(&[0x05], 1), [0]);
        assert_eq!(h.flash.read(DATA), 0);
        assert_eq!(h.flash.read(DATA + 1), 0xff);
        h.enable();
        h.program(DATA + 1, &[0]);
        h.command(&[0x66], 0);
        h.command(&[0x99], 0);
        assert!(h.flash.pending.is_none());
        assert_eq!(h.flash.status().register & 3, 0);
        h.until(h.time + T_PP_PS);
        assert_eq!(
            h.flash.read(DATA + 1),
            0xff,
            "deterministic reset discards pending writes"
        );
    }
    #[test]
    fn standard_bp_table_protects_top_blocks_and_rejects_chip_erase() {
        for bp in 1..=15 {
            let protected_bytes = if bp <= 8 { 65536 << (bp - 1) } else { CAPACITY };
            let boundary = CAPACITY - protected_bytes;
            let mut h = Host::new(SpiFlash::with_status(Vec::new(), bp << 2), false);
            h.enable();
            h.program(boundary, &[0]);
            h.erase(0x20, boundary);
            h.command(&[0xc7], 0);
            assert!(h.flash.pending.is_none());
            assert_eq!(h.flash.status().protected, 3);
            assert_eq!(h.flash.status().ignored_writes, 3);
            assert_eq!(h.flash.read(boundary), 0xff);
            assert_eq!(h.command(&[0x81], 1), [0xfe]);
            if boundary != 0 {
                h.program(boundary - 1, &[0]);
                h.finish_write();
                assert_eq!(h.flash.read(boundary - 1), 0);
            }
        }
        // HR07's SR 18 has BP1/BP2: the top 32 x 64 KB = 2 MB.
        let flash = SpiFlash::with_status(Vec::new(), 0x18);
        assert!(!flash.protected(0xdfffff, 0xe00000));
        assert!(flash.protected(0xe00000, 0xe00001));
    }
    #[test]
    fn wrsr_obeys_wel_masks_volatile_bits_and_keeps_qe_srwd_in_memory() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        h.command(&[0x01, 0xff], 0);
        assert_eq!(h.flash.status().register, 0);
        h.enable();
        h.command(&[0x01, 0xc7], 0);
        let deadline = h.flash.pending.as_ref().unwrap().finish_ps;
        assert_eq!(deadline - (h.time - 10), T_WRSR_PS);
        h.until(deadline - 1);
        assert_eq!(h.flash.status().register, 3);
        h.until(deadline);
        assert_eq!(h.command(&[0x05], 2), [0xc4; 2]);
        h.enable();
        h.command(&[0x01, 0], 0);
        h.finish_write();
        assert_eq!(
            h.flash.status().register,
            0,
            "WP high permits clearing SRWD"
        );
        assert_eq!(h.flash.status().programs, 0);
    }
    #[test]
    fn configuration_warning_covers_inside_outside_wrap_and_straddling_chip_erase() {
        assert_eq!(CONFIGURATION_END, 0x250000);
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        h.program(CONFIGURATION_END, &[0]);
        assert_eq!(
            h.flash.status().config_writes,
            0,
            "end is exclusive, even without WEL"
        );
        h.program(0, &[0]);
        let first = h.flash.status().first_config_write.unwrap();
        assert_eq!((first.command, first.start, first.end), (0x02, 0, 1));
        assert!(first.to_string().contains(
            "this would overwrite the board's FPGA configuration (bitstream) on a real ULX3S"
        ));
        h.program(CONFIGURATION_END - 1, &[0, 0]);
        assert_eq!(h.flash.status().config_writes, 2);
        assert_eq!(h.flash.read(CONFIGURATION_END), 0xff);
        h.command(&[0xc7], 0); // The only supported aligned erase spanning the boundary.
        assert_eq!(h.flash.status().config_writes, 3);
        assert_eq!(
            h.flash
                .status()
                .first_config_write
                .as_ref()
                .unwrap()
                .as_ref(),
            first.as_ref()
        );
        let mut protected = Host::new(SpiFlash::with_status(Vec::new(), 0x3c), false);
        protected.enable();
        protected.erase(0x20, 0);
        assert_eq!(
            (
                protected.flash.status().protected,
                protected.flash.status().config_writes
            ),
            (1, 1)
        );
        assert_eq!(
            protected.flash.status().first_config_write.unwrap().end,
            4096
        );
        let mut overlap = SpiFlash::new(Vec::new());
        overlap.warn_configuration(0xd8, CONFIGURATION_END - 1, CONFIGURATION_END + 1);
        assert_eq!(overlap.status().config_writes, 1);
    }
    #[test]
    fn busy_and_protected_configuration_attempts_still_warn_once() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        h.enable();
        h.program(DATA, &[0]);
        h.erase(0x20, 0);
        assert_eq!(
            (
                h.flash.status().config_writes,
                h.flash.status().busy_ignored
            ),
            (1, 1)
        );
        assert_eq!(h.flash.status().first_config_write.unwrap().command, 0x20);
    }
    #[test]
    fn snapshots_share_storage_and_restore_before_and_after_program_and_erase() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        let before = h.flash.clone();
        assert!(Arc::ptr_eq(&before.base, &h.flash.base));
        assert!(Arc::ptr_eq(&before.sectors, &h.flash.sectors));
        assert!(std::mem::size_of::<SpiFlash>() < 2048);
        assert_eq!(
            h.flash.sectors.len() * std::mem::size_of::<Option<Arc<Sector>>>(),
            32768
        );
        h.enable();
        h.program(DATA, &[0x12]);
        let during = h.flash.clone();
        assert!(Arc::ptr_eq(
            during.pending.as_ref().unwrap(),
            h.flash.pending.as_ref().unwrap()
        ));
        h.finish_write();
        assert_eq!(h.flash.read(DATA), 0x12);
        assert_eq!(h.flash.base[DATA], 0xff, "base image is immutable");
        assert!(!Arc::ptr_eq(&before.sectors, &h.flash.sectors));
        assert!(Arc::ptr_eq(&before.base, &h.flash.base));
        let after_program = h.flash.clone();
        h.enable();
        h.program(DATA, &[0x01]);
        h.finish_write();
        assert!(!Arc::ptr_eq(
            after_program.sectors[DATA / 4096].as_ref().unwrap(),
            h.flash.sectors[DATA / 4096].as_ref().unwrap()
        ));
        assert_eq!(after_program.read(DATA), 0x12);
        h.flash = after_program.clone();
        h.enable();
        h.erase(0x20, DATA);
        h.finish_write();
        assert_eq!(h.flash.read(DATA), 0xff);
        h.flash = after_program;
        assert_eq!(h.flash.read(DATA), 0x12);
        h.flash = before;
        assert_eq!(h.flash.read(DATA), 0xff);
        h.flash = during;
        h.pins = h.flash.previous; // Resume the cloned in-flight operation at its deadline.
        h.time = 0;
        h.finish_write();
        assert_eq!(h.flash.read(DATA), 0x12);
        let before_chip = h.flash.clone();
        h.enable();
        h.command(&[0xc7], 0);
        h.finish_write();
        assert!(h.flash.base_erased);
        assert_eq!(h.flash.read(DATA), 0xff);
        h.flash = before_chip;
        assert!(!h.flash.base_erased);
        assert_eq!(h.flash.read(DATA), 0x12);
    }
    #[test]
    fn malformed_writes_and_unknown_pads_never_commit_and_unsupported_writes_stay_ignored() {
        let mut h = Host::new(SpiFlash::new(Vec::new()), false);
        for opcode in [
            0x32, 0x38, 0xad, 0x31, 0x11, 0x50, 0x42, 0x44, 0x7e, 0x98, 0x36, 0x39, 0xa6, 0x68,
            0xb1, 0xc1,
        ] {
            h.command(&[opcode, 0, 0, 0, 0], 0);
        }
        assert_eq!(h.flash.status().ignored_writes, 16);
        h.enable();
        h.select();
        h.byte(0x02);
        h.byte(0x30);
        h.byte(0);
        h.byte(0);
        h.byte(0);
        h.pins.cs = 2;
        h.feed();
        assert!(h.flash.pending.is_none());
        assert_eq!(h.flash.read(DATA), 0xff);
        h.pins.cs = 1;
        h.feed();
        h.select();
        h.byte(0x20);
        h.byte(0x30);
        h.byte(0);
        h.end();
        assert!(h.flash.pending.is_none());
    }
}
