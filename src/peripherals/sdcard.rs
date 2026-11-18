//! SDHC card in SPI and native modes on the ULX3S (SD Physical Layer v2.0), sampled at
//! the package pins. No executor is required.
//! See docs/PERIPHERALS_SD.md for timing, commands and limits.
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
#[path = "sd/native.rs"]
pub mod native;

pub const CLK_BALL: &str = crate::board::SD_CLK;
pub const MOSI_BALL: &str = crate::board::SD_CMD;
pub const MISO_BALL: &str = crate::board::SD_D[0];
pub const CS_BALL: &str = crate::board::SD_D[3];

pub const BLOCK_SIZE: usize = 512;
pub const MIN_CAPACITY_BYTES: usize = 64 * 1024 * 1024;
const NCR_BYTES: usize = 1;
const READ_GAP_BYTES: usize = 2;
const WRITE_BUSY_BYTES: usize = 16;

/// 0 and 1 are levels, 2 is X and 3 is Z.
/// CS = SD_D[3] (K2), SCLK = SD_CLK (H2), MOSI = SD_CMD (J1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdPins {
    pub cs: u8,
    pub clk: u8,
    pub mosi: u8,
}

/// High-level card state exposed for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdState {
    Idle,
    Ready,
    Reading,
    Writing,
}

/// Snapshot of card status and diagnostic counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdStatus {
    pub state: SdState,
    pub last_command: Option<u8>,
    pub last_argument: u32,
    pub blocks_read: u64,
    pub blocks_written: u64,
    pub crc_errors: u64,
    pub illegal_commands: u64,
    pub version: u64,
    pub mode: &'static str,
    pub native_state: Option<native::State>,
    pub rca: u16,
    pub bus_width: u8,
    pub dat3_pullup: bool,
    pub commands: u64,
    pub responses: u64,
}

#[derive(Clone)]
enum TransferMode {
    Command,
    SingleRead,
    MultiRead {
        next_block: u32,
    },
    WriteWaitToken {
        block: u32,
        multi: bool,
    },
    WriteData {
        block: u32,
        multi: bool,
        buf: Vec<u8>,
    },
    WriteBusy {
        next_multi_block: Option<u32>,
    },
}

/// One card image and write overlay, with SPI mode 0 and native SD protocols.
#[derive(Clone)]
pub struct SdCard {
    native: native::Native,
    native_interface: bool,
    spi_selected: bool,
    base: Arc<Vec<u8>>,
    written: BTreeMap<u32, [u8; BLOCK_SIZE]>,
    num_blocks: u32,
    previous: Option<SdPins>,
    in_bits: u8,
    in_shift: u8,
    out_byte: u8,
    next_out_byte: Option<u8>,
    miso_level: u8,
    cmd_buf: [u8; 6],
    cmd_len: usize,
    tx_queue: VecDeque<u8>,
    mode: TransferMode,
    initialized: bool,
    crc_on: bool,
    app_cmd: bool,
    acmd41_polls: u8,
    block_len: u32,
    state: SdState,
    last_command: Option<u8>,
    last_argument: u32,
    blocks_read: u64,
    blocks_written: u64,
    crc_errors: u64,
    illegal_commands: u64,
    version: u64,
}

impl Default for SdCard {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl SdCard {
    /// Creates a block-addressed SDHC card backed by `image`.
    /// Capacity is `image.len()` rounded up to 512 bytes, at least 64 MB.
    pub fn new(image: Vec<u8>) -> Self {
        let rounded = image.len().div_ceil(BLOCK_SIZE) * BLOCK_SIZE;
        let capacity = rounded.max(MIN_CAPACITY_BYTES);
        let num_blocks = u32::try_from(capacity / BLOCK_SIZE).expect("SDHC block count fits u32");
        Self {
            native: native::Native::default(),
            native_interface: false,
            spi_selected: false,
            base: Arc::new(image),
            written: BTreeMap::new(),
            num_blocks,
            previous: None,
            in_bits: 0,
            in_shift: 0,
            out_byte: 0xff,
            next_out_byte: None,
            miso_level: 1,
            cmd_buf: [0; 6],
            cmd_len: 0,
            tx_queue: VecDeque::new(),
            mode: TransferMode::Command,
            initialized: false,
            crc_on: false,
            app_cmd: false,
            acmd41_polls: 0,
            block_len: BLOCK_SIZE as u32,
            state: SdState::Idle,
            last_command: None,
            last_argument: 0,
            blocks_read: 0,
            blocks_written: 0,
            crc_errors: 0,
            illegal_commands: 0,
            version: 0,
        }
    }

    /// The level the card currently drives on DO (MISO).
    /// Returns 1 while idle or deselected (board pull-up).
    pub fn miso(&self) -> u8 {
        self.miso_level
    }

    /// Whether CS was low at the last sample: the card may answer on DO at
    /// the next SCLK falling edge, so a runner feeds it edge by edge.
    pub fn selected(&self) -> bool {
        if self.native_interface && !self.spi_selected {
            self.native.active()
        } else {
            self.previous.is_some_and(|p| p.cs == 0)
        }
    }

    /// A card with a boot sector in block 0, for a window opened without an
    /// image: a jump, the OEM name `ECP5_EMU`, a volume label, and the 55 AA
    /// signature at 510. The rest of its 64 MB reads 0.
    pub fn demo() -> Self {
        let mut block = vec![0u8; BLOCK_SIZE];
        block[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
        block[3..11].copy_from_slice(b"ECP5_EMU");
        block[43..54].copy_from_slice(b"ULX3S CARD ");
        block[510] = 0x55;
        block[511] = 0xaa;
        Self::new(block)
    }

    /// Materializes the full card contents (at least 64 MB).
    pub fn image(&self) -> Vec<u8> {
        let len = self.capacity_bytes() as usize;
        let mut out = vec![0u8; len];
        let copy_len = self.base.len().min(len);
        out[..copy_len].copy_from_slice(&self.base[..copy_len]);
        for (&block, data) in &self.written {
            let offset = block as usize * BLOCK_SIZE;
            if offset + BLOCK_SIZE <= len {
                out[offset..offset + BLOCK_SIZE].copy_from_slice(data);
            }
        }
        out
    }

    pub fn capacity_bytes(&self) -> u64 {
        u64::from(if self.native_interface && !self.spi_selected {
            self.num_blocks.div_ceil(1024) * 1024
        } else {
            self.num_blocks
        }) * BLOCK_SIZE as u64
    }

    pub fn status(&self) -> SdStatus {
        if self.native_interface && !self.spi_selected {
            let n = &self.native.status;
            return SdStatus {
                state: match n.state {
                    native::State::Idle => SdState::Idle,
                    native::State::Data => SdState::Reading,
                    native::State::Rcv | native::State::Prg => SdState::Writing,
                    _ => SdState::Ready,
                },
                last_command: n.last_command,
                last_argument: n.last_argument,
                blocks_read: n.blocks_read,
                blocks_written: n.blocks_written,
                crc_errors: n.crc_errors,
                illegal_commands: n.illegal_commands,
                version: n.version,
                mode: if n.width == 4 { "sd4" } else { "sd1" },
                native_state: Some(n.state),
                rca: n.rca,
                bus_width: n.width,
                dat3_pullup: n.dat3_pullup,
                commands: n.commands,
                responses: n.responses,
            };
        }
        SdStatus {
            state: self.state,
            last_command: self.last_command,
            last_argument: self.last_argument,
            blocks_read: self.blocks_read,
            blocks_written: self.blocks_written,
            crc_errors: self.crc_errors,
            illegal_commands: self.illegal_commands,
            version: self.version,
            mode: "spi",
            native_state: None,
            rca: 0,
            bus_width: 1,
            dat3_pullup: false,
            commands: self.native.status.commands,
            responses: self.native.status.responses,
        }
    }

    /// Monotonic counter incremented whenever `status()` changes.
    pub fn version(&self) -> u64 {
        self.status().version
    }

    /// Select the pad-fed CMD/DAT interface; CMD0 still chooses the wire protocol.
    pub fn native_interface(&mut self) {
        self.native_interface = true;
    }
    /// Card transmitters only. DAT3's passive bias is reported separately.
    pub fn bus_outputs(&self) -> [Option<bool>; 5] {
        if self.spi_selected {
            [None, Some(self.miso() == 1), None, None, None]
        } else {
            self.native.outputs()
        }
    }
    pub fn dat3_bias(&self) -> bool {
        !self.spi_selected && self.native.dat3_bias()
    }
    /// Feed the same card through native CMD/DAT pads. SPI latches until a
    /// new card (power cycle); its old byte engine and image stay unchanged.
    pub fn feed_bus(&mut self, ps: u64, pins: native::Pins) {
        self.native_interface = true;
        if self.spi_selected {
            self.feed(
                ps,
                SdPins {
                    cs: pins.dat[3],
                    clk: pins.clk,
                    mosi: pins.cmd,
                },
            );
            return;
        }
        let mut media = native::Media {
            base: &self.base,
            written: &mut self.written,
            blocks: self.num_blocks.div_ceil(1024) * 1024,
        };
        self.native.feed(ps, pins, &mut media);
        if let Some(command) = self.native.spi_command.take() {
            self.spi_selected = true;
            self.native.status.commands = 0;
            self.native.status.responses = 0;
            self.cmd_buf = command;
            self.execute_command();
            self.previous = Some(SdPins {
                cs: 0,
                clk: 1,
                mosi: 1,
            });
            self.in_bits = 0;
            self.in_shift = 0;
            self.cmd_len = 0;
            self.out_byte = 255;
            self.miso_level = 1;
            self.next_out_byte = Some(self.tx_queue.pop_front().unwrap_or(255));
        }
    }
    /// Samples the SPI pins in time order; acts on edges only.
    pub fn feed(&mut self, _time_ps: u64, pins: SdPins) {
        let previous = self.previous.replace(pins);
        if previous == Some(pins) {
            return;
        }
        if pins.cs != 0 {
            if previous.is_some_and(|p| p.cs == 0) {
                self.deselect();
            }
            self.miso_level = 1;
            return;
        }

        // CS just went low (or started low on first sample): show first DO bit.
        if previous.is_none_or(|p| p.cs != 0) {
            self.in_bits = 0;
            self.in_shift = 0;
            self.out_byte = self.tx_queue.pop_front().unwrap_or(0xff);
            self.next_out_byte = None;
            self.miso_level = (self.out_byte >> 7) & 1;
        }

        if pins.clk == 1 && previous.is_some_and(|p| p.clk == 0) {
            if pins.mosi > 1 {
                self.in_bits = 0;
                return;
            }
            self.in_shift = (self.in_shift << 1) | pins.mosi;
            self.in_bits += 1;
            if self.in_bits == 8 {
                self.in_bits = 0;
                let byte = self.in_shift;
                self.next_out_byte = Some(self.on_byte(byte));
            }
        } else if pins.clk == 0 && previous.is_some_and(|p| p.clk == 1) {
            if let Some(next) = self.next_out_byte.take() {
                self.out_byte = next;
                self.miso_level = (self.out_byte >> 7) & 1;
            } else if self.in_bits > 0 {
                self.miso_level = (self.out_byte >> (7 - self.in_bits)) & 1;
            }
        }
    }

    fn deselect(&mut self) {
        self.in_bits = 0;
        self.in_shift = 0;
        self.cmd_len = 0;
        self.next_out_byte = None;
        self.tx_queue.clear();
        self.out_byte = 0xff;
        self.miso_level = 1;
        let base_state = if self.initialized {
            SdState::Ready
        } else {
            SdState::Idle
        };
        if !matches!(self.mode, TransferMode::Command) || self.state != base_state {
            self.mode = TransferMode::Command;
            if self.state != base_state {
                self.state = base_state;
                self.version += 1;
            }
        }
    }

    fn on_byte(&mut self, byte: u8) -> u8 {
        match &mut self.mode {
            TransferMode::Command => {
                if !self.tx_queue.is_empty() {
                    return self.tx_queue.pop_front().unwrap_or(0xff);
                }
                if self.cmd_len > 0 || (byte & 0xc0) == 0x40 {
                    self.cmd_buf[self.cmd_len] = byte;
                    self.cmd_len += 1;
                    if self.cmd_len == 6 {
                        self.cmd_len = 0;
                        self.execute_command();
                    }
                }
                self.tx_queue.pop_front().unwrap_or(0xff)
            }
            TransferMode::SingleRead => {
                if let Some(next) = self.tx_queue.pop_front() {
                    next
                } else {
                    self.blocks_read += 1;
                    self.mode = TransferMode::Command;
                    self.state = SdState::Ready;
                    self.version += 1;
                    0xff
                }
            }
            TransferMode::MultiRead { .. } => {
                if self.cmd_len > 0 || (byte & 0xc0) == 0x40 {
                    self.cmd_buf[self.cmd_len] = byte;
                    self.cmd_len += 1;
                    if self.cmd_len == 6 {
                        self.cmd_len = 0;
                        let cmd = self.cmd_buf[0] & 0x3f;
                        if cmd == 12 || cmd == 0 {
                            self.tx_queue.clear();
                            self.mode = TransferMode::Command;
                            self.execute_command();
                            return self.tx_queue.pop_front().unwrap_or(0xff);
                        }
                    }
                }
                if let Some(next) = self.tx_queue.pop_front() {
                    next
                } else if let TransferMode::MultiRead { next_block } = &mut self.mode {
                    self.blocks_read += 1;
                    let block = *next_block;
                    if block < self.num_blocks {
                        *next_block = block + 1;
                        self.version += 1;
                        self.enqueue_read_block(block);
                        self.tx_queue.pop_front().unwrap_or(0xff)
                    } else {
                        self.mode = TransferMode::Command;
                        self.state = SdState::Ready;
                        self.version += 1;
                        0xff
                    }
                } else {
                    0xff
                }
            }
            TransferMode::WriteWaitToken { block, multi } => {
                if !self.tx_queue.is_empty() {
                    return self.tx_queue.pop_front().unwrap_or(0xff);
                }
                let (block, multi) = (*block, *multi);
                match byte {
                    0xff => 0xff,
                    0xfe if !multi => {
                        self.mode = TransferMode::WriteData {
                            block,
                            multi: false,
                            buf: Vec::with_capacity(BLOCK_SIZE + 2),
                        };
                        0xff
                    }
                    0xfc if multi => {
                        self.mode = TransferMode::WriteData {
                            block,
                            multi: true,
                            buf: Vec::with_capacity(BLOCK_SIZE + 2),
                        };
                        0xff
                    }
                    0xfd if multi => {
                        for _ in 0..WRITE_BUSY_BYTES {
                            self.tx_queue.push_back(0x00);
                        }
                        self.mode = TransferMode::WriteBusy {
                            next_multi_block: None,
                        };
                        self.tx_queue.pop_front().unwrap_or(0xff)
                    }
                    _ => 0xff,
                }
            }
            TransferMode::WriteData { block, multi, buf } => {
                buf.push(byte);
                if buf.len() < BLOCK_SIZE + 2 {
                    return 0xff;
                }
                let block = *block;
                let multi = *multi;
                let crc_ok = if self.crc_on {
                    let expected = crc16(&buf[..BLOCK_SIZE]);
                    let got = u16::from_be_bytes([buf[BLOCK_SIZE], buf[BLOCK_SIZE + 1]]);
                    expected == got
                } else {
                    true
                };
                if !crc_ok {
                    self.crc_errors += 1;
                    self.version += 1;
                    if multi {
                        self.mode = TransferMode::WriteWaitToken { block, multi: true };
                    } else {
                        self.mode = TransferMode::Command;
                        self.state = SdState::Ready;
                    }
                    return 0x0b;
                }
                if block >= self.num_blocks {
                    if !multi {
                        self.mode = TransferMode::Command;
                        self.state = SdState::Ready;
                        self.version += 1;
                    }
                    return 0x0d;
                }
                let mut sector = [0u8; BLOCK_SIZE];
                sector.copy_from_slice(&buf[..BLOCK_SIZE]);
                self.written.insert(block, sector);
                self.blocks_written += 1;
                self.version += 1;
                for _ in 0..WRITE_BUSY_BYTES {
                    self.tx_queue.push_back(0x00);
                }
                self.mode = TransferMode::WriteBusy {
                    next_multi_block: multi.then_some(block + 1),
                };
                0x05
            }
            TransferMode::WriteBusy { next_multi_block } => {
                if let Some(next) = self.tx_queue.pop_front() {
                    next
                } else if let Some(block) = *next_multi_block {
                    self.mode = TransferMode::WriteWaitToken { block, multi: true };
                    0xff
                } else {
                    self.mode = TransferMode::Command;
                    self.state = SdState::Ready;
                    self.version += 1;
                    0xff
                }
            }
        }
    }

    fn execute_command(&mut self) {
        self.native.status.commands += 1;
        self.native.status.responses += 1;
        let cmd = self.cmd_buf[0] & 0x3f;
        let arg = u32::from_be_bytes([
            self.cmd_buf[1],
            self.cmd_buf[2],
            self.cmd_buf[3],
            self.cmd_buf[4],
        ]);
        let crc_byte = self.cmd_buf[5];
        self.last_command = Some(cmd);
        self.last_argument = arg;
        self.version += 1;

        for _ in 0..NCR_BYTES {
            self.tx_queue.push_back(0xff);
        }

        let check_crc = self.crc_on || cmd == 0 || cmd == 8;
        if check_crc && ((crc_byte >> 1) != crc7(&self.cmd_buf[..5]) || (crc_byte & 1) == 0) {
            self.app_cmd = false;
            self.crc_errors += 1;
            self.tx_queue.push_back(self.idle_bit() | 0x08);
            return;
        }

        let was_app = self.app_cmd;
        self.app_cmd = false;

        if was_app {
            match cmd {
                41 => {
                    if self.acmd41_polls < 2 {
                        self.acmd41_polls += 1;
                        self.tx_queue.push_back(0x01);
                    } else {
                        self.initialized = true;
                        self.state = SdState::Ready;
                        self.tx_queue.push_back(0x00);
                    }
                }
                _ => {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(self.idle_bit() | 0x04);
                }
            }
            return;
        }

        match cmd {
            0 => {
                self.spi_selected = true;
                self.initialized = false;
                self.crc_on = false;
                self.app_cmd = false;
                self.acmd41_polls = 0;
                self.block_len = BLOCK_SIZE as u32;
                self.state = SdState::Idle;
                self.mode = TransferMode::Command;
                self.tx_queue.push_back(0x01);
            }
            8 => {
                self.tx_queue.push_back(self.idle_bit());
                self.tx_queue.extend(arg.to_be_bytes());
            }
            9 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else {
                    self.tx_queue.push_back(0x00);
                    let csd = self.csd_v2();
                    self.enqueue_data_packet(&csd);
                }
            }
            10 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else {
                    self.tx_queue.push_back(0x00);
                    let cid = self.cid();
                    self.enqueue_data_packet(&cid);
                }
            }
            12 => {
                // Handled in MultiRead when stopping a stream; outside MultiRead it is idle/ready R1.
                self.state = if self.initialized {
                    SdState::Ready
                } else {
                    SdState::Idle
                };
                self.tx_queue.push_back(self.idle_bit());
            }
            13 => {
                self.tx_queue.push_back(self.idle_bit());
                self.tx_queue.push_back(0x00);
            }
            16 => {
                if arg == BLOCK_SIZE as u32 {
                    self.block_len = arg;
                    self.tx_queue.push_back(self.idle_bit());
                } else {
                    self.tx_queue.push_back(self.idle_bit() | 0x40);
                }
            }
            17 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else if arg >= self.num_blocks {
                    self.tx_queue.push_back(0x20);
                } else {
                    self.state = SdState::Reading;
                    self.mode = TransferMode::SingleRead;
                    self.tx_queue.push_back(0x00);
                    self.enqueue_read_block(arg);
                }
            }
            18 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else if arg >= self.num_blocks {
                    self.tx_queue.push_back(0x20);
                } else {
                    self.state = SdState::Reading;
                    self.mode = TransferMode::MultiRead {
                        next_block: arg + 1,
                    };
                    self.tx_queue.push_back(0x00);
                    self.enqueue_read_block(arg);
                }
            }
            24 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else if arg >= self.num_blocks {
                    self.tx_queue.push_back(0x20);
                } else {
                    self.state = SdState::Writing;
                    self.mode = TransferMode::WriteWaitToken {
                        block: arg,
                        multi: false,
                    };
                    self.tx_queue.push_back(0x00);
                }
            }
            25 => {
                if !self.initialized {
                    self.illegal_commands += 1;
                    self.tx_queue.push_back(0x05);
                } else if arg >= self.num_blocks {
                    self.tx_queue.push_back(0x20);
                } else {
                    self.state = SdState::Writing;
                    self.mode = TransferMode::WriteWaitToken {
                        block: arg,
                        multi: true,
                    };
                    self.tx_queue.push_back(0x00);
                }
            }
            55 => {
                self.app_cmd = true;
                self.tx_queue.push_back(self.idle_bit());
            }
            58 => {
                let ocr: u32 = if self.initialized {
                    0xc0ff_8000
                } else {
                    0x00ff_8000
                };
                self.tx_queue.push_back(self.idle_bit());
                self.tx_queue.extend(ocr.to_be_bytes());
            }
            59 => {
                self.crc_on = (arg & 1) != 0;
                self.tx_queue.push_back(self.idle_bit());
            }
            _ => {
                self.illegal_commands += 1;
                self.tx_queue.push_back(self.idle_bit() | 0x04);
            }
        }
    }

    fn idle_bit(&self) -> u8 {
        u8::from(!self.initialized)
    }

    fn read_block_bytes(&self, block: u32) -> [u8; BLOCK_SIZE] {
        if let Some(sector) = self.written.get(&block) {
            return *sector;
        }
        let mut out = [0u8; BLOCK_SIZE];
        let start = block as usize * BLOCK_SIZE;
        if start < self.base.len() {
            let end = (start + BLOCK_SIZE).min(self.base.len());
            out[..end - start].copy_from_slice(&self.base[start..end]);
        }
        out
    }

    fn enqueue_read_block(&mut self, block: u32) {
        let data = self.read_block_bytes(block);
        self.enqueue_data_packet(&data);
    }

    fn enqueue_data_packet(&mut self, data: &[u8]) {
        for _ in 0..READ_GAP_BYTES {
            self.tx_queue.push_back(0xff);
        }
        self.tx_queue.push_back(0xfe);
        self.tx_queue.extend(data);
        let crc = crc16(data);
        self.tx_queue.extend(crc.to_be_bytes());
    }

    fn csd_v2(&self) -> [u8; 16] {
        // Capacity = (C_SIZE + 1) * 512 KiB = (C_SIZE + 1) * 1024 blocks.
        let c_size = (self.num_blocks / 1024).saturating_sub(1);
        let mut csd = [0u8; 16];
        csd[0] = 0x40; // CSD_STRUCTURE = 1 (v2.0)
        csd[1] = 0x0e; // TAAC
        csd[2] = 0x00; // NSAC
        csd[3] = 0x32; // TRAN_SPEED (25 MHz)
        csd[4] = 0x5b; // CCC[11:4]
        csd[5] = 0x59; // CCC[3:0] | READ_BL_LEN=9
        csd[6] = 0x00;
        csd[7] = ((c_size >> 16) & 0x3f) as u8;
        csd[8] = ((c_size >> 8) & 0xff) as u8;
        csd[9] = (c_size & 0xff) as u8;
        csd[10] = 0x7f; // ERASE_BLK_EN=1, SECTOR_SIZE[6:1]=0x3f
        csd[11] = 0x80; // SECTOR_SIZE[0]=1
        csd[12] = 0x0a; // R2W_FACTOR=2, WRITE_BL_LEN[3:2]=2
        csd[13] = 0x40; // WRITE_BL_LEN[1:0]=1
        csd[14] = 0x00;
        csd[15] = (crc7(&csd[..15]) << 1) | 1;
        csd
    }

    fn cid(&self) -> [u8; 16] {
        let mut cid = [0u8; 16];
        cid[0] = 0x03; // MID
        cid[1..3].copy_from_slice(b"EC"); // OID
        cid[3..8].copy_from_slice(b"ECP5 "); // PNM
        cid[8] = 0x10; // PRV 1.0
        cid[9..13].copy_from_slice(&[0x00, 0x00, 0x00, 0x01]); // PSN
        cid[13] = 0x01; // MDT
        cid[14] = 0x6b;
        cid[15] = (crc7(&cid[..15]) << 1) | 1;
        cid
    }
}

/// SD command CRC7: G(x) = x^7 + x^3 + 1, big-endian, 7-bit result in bits 6..0.
pub fn crc7(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in bytes {
        let mut data = b;
        for _ in 0..8 {
            let bit = ((data & 0x80) != 0) ^ ((crc & 0x40) != 0);
            crc = (crc << 1) & 0x7f;
            if bit {
                crc ^= 0x09;
            }
            data <<= 1;
        }
    }
    crc
}

/// SD data CRC16-CCITT: G(x) = x^16 + x^12 + x^5 + 1, initial value 0.
pub fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in bytes {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if (crc & 0x8000) != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Host {
        card: SdCard,
        pins: SdPins,
        time: u64,
    }

    impl Host {
        fn new(image: Vec<u8>) -> Self {
            let mut h = Self {
                card: SdCard::new(image),
                pins: SdPins {
                    cs: 1,
                    clk: 0,
                    mosi: 1,
                },
                time: 0,
            };
            h.sample();
            h
        }

        fn sample(&mut self) {
            self.time += 100;
            self.card.feed(self.time, self.pins);
        }

        fn set_cs(&mut self, cs: u8) {
            self.pins.cs = cs;
            self.sample();
        }

        fn xfer(&mut self, tx: u8) -> u8 {
            let mut rx = 0u8;
            for bit in (0..8).rev() {
                self.pins.mosi = (tx >> bit) & 1;
                self.pins.clk = 0;
                self.sample();
                rx = (rx << 1) | self.card.miso();
                self.pins.clk = 1;
                self.sample();
                self.pins.clk = 0;
                self.sample();
            }
            rx
        }

        fn send_cmd_raw(&mut self, cmd: u8, arg: u32, crc_byte: u8) -> u8 {
            self.pins.cs = 0;
            self.sample();
            let frame = [
                0x40 | (cmd & 0x3f),
                (arg >> 24) as u8,
                (arg >> 16) as u8,
                (arg >> 8) as u8,
                arg as u8,
                crc_byte,
            ];
            for b in frame {
                self.xfer(b);
            }
            // Fixed Ncr = 1 byte of 0xFF, then R1.
            assert_eq!(self.xfer(0xff), 0xff);
            self.xfer(0xff)
        }

        fn send_cmd(&mut self, cmd: u8, arg: u32) -> u8 {
            let mut frame = [
                0x40 | (cmd & 0x3f),
                (arg >> 24) as u8,
                (arg >> 16) as u8,
                (arg >> 8) as u8,
                arg as u8,
            ];
            let crc_byte = (crc7(&frame) << 1) | 1;
            let _ = &mut frame;
            self.send_cmd_raw(cmd, arg, crc_byte)
        }

        fn wake(&mut self) {
            self.set_cs(1);
            self.pins.mosi = 1;
            for _ in 0..10 {
                self.xfer(0xff);
            }
        }

        fn init(&mut self) -> u32 {
            self.wake();
            assert_eq!(self.send_cmd(0, 0), 0x01);
            self.set_cs(1);
            assert_eq!(self.send_cmd(8, 0x0000_01aa), 0x01);
            let r7 = [
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
            ];
            assert_eq!(u32::from_be_bytes(r7), 0x0000_01aa);
            self.set_cs(1);

            // CMD58 before init -> 0x00FF8000.
            assert_eq!(self.send_cmd(58, 0), 0x01);
            let ocr_before = [
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
            ];
            assert_eq!(u32::from_be_bytes(ocr_before), 0x00ff_8000);
            self.set_cs(1);

            // First two polls give 0x01, third gives 0x00.
            for expected in [0x01u8, 0x01, 0x00] {
                assert_eq!(self.send_cmd(55, 0), 0x01);
                self.set_cs(1);
                assert_eq!(self.send_cmd(41, 0x4000_0000), expected);
                self.set_cs(1);
            }

            // CMD58 after init -> 0xC0FF8000.
            assert_eq!(self.send_cmd(58, 0), 0x00);
            let ocr_after = [
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
                self.xfer(0xff),
            ];
            self.set_cs(1);
            u32::from_be_bytes(ocr_after)
        }

        fn read_packet(&mut self, len: usize) -> Vec<u8> {
            assert_eq!(self.xfer(0xff), 0xff);
            assert_eq!(self.xfer(0xff), 0xff);
            assert_eq!(self.xfer(0xff), 0xfe);
            let mut data = vec![0u8; len];
            for b in &mut data {
                *b = self.xfer(0xff);
            }
            let crc_hi = self.xfer(0xff);
            let crc_lo = self.xfer(0xff);
            assert_eq!(u16::from_be_bytes([crc_hi, crc_lo]), crc16(&data));
            data
        }
    }

    #[test]
    fn crc_vectors_match_sd_spec() {
        assert_eq!((crc7(&[0x40, 0, 0, 0, 0]) << 1) | 1, 0x95);
        assert_eq!((crc7(&[0x48, 0, 0, 1, 0xaa]) << 1) | 1, 0x87);
        assert_eq!(crc16(&[0xff; 512]), 0x7fa1);
    }

    #[test]
    fn full_init_sequence_and_registers() {
        let mut h = Host::new(Vec::new());
        assert_eq!(h.card.miso(), 1);
        assert_eq!(h.card.capacity_bytes(), 64 * 1024 * 1024);
        assert_eq!(h.card.status().state, SdState::Idle);
        let ocr = h.init();
        assert_eq!(ocr, 0xc0ff_8000);
        assert_eq!(h.card.status().state, SdState::Ready);

        // CMD9 (CSD v2) and CMD10 (CID)
        assert_eq!(h.send_cmd(9, 0), 0x00);
        let csd = h.read_packet(16);
        assert_eq!(csd[0] >> 6, 1); // CSD v2
        assert_eq!((crc7(&csd[..15]) << 1) | 1, csd[15]);
        h.set_cs(1);

        assert_eq!(h.send_cmd(10, 0), 0x00);
        let cid = h.read_packet(16);
        assert_eq!((crc7(&cid[..15]) << 1) | 1, cid[15]);
        h.set_cs(1);

        // CMD13 (R2) and CMD16 (block len)
        assert_eq!(h.send_cmd(13, 0), 0x00);
        assert_eq!(h.xfer(0xff), 0x00);
        h.set_cs(1);
        assert_eq!(h.send_cmd(16, 512), 0x00);
        h.set_cs(1);
        assert_eq!(h.send_cmd(16, 256), 0x40);
        h.set_cs(1);
    }

    #[test]
    fn read_block_zero_and_block_12345() {
        let mut img = vec![0u8; 12346 * 512];
        for i in 0..512 {
            img[i] = (i as u8).wrapping_mul(3).wrapping_add(1);
            img[12345 * 512 + i] = (i as u8) ^ 0xa5;
        }
        img[510] = 0x55;
        img[511] = 0xaa;

        let mut h = Host::new(img.clone());
        h.init();

        assert_eq!(h.send_cmd(17, 0), 0x00);
        let b0 = h.read_packet(512);
        assert_eq!(b0, img[..512]);
        h.set_cs(1);

        assert_eq!(h.send_cmd(17, 12345), 0x00);
        assert_eq!(h.card.status().state, SdState::Reading);
        let b12345 = h.read_packet(512);
        assert_eq!(b12345, img[12345 * 512..12346 * 512]);
        assert_eq!(h.card.status().state, SdState::Ready);
        assert_eq!(h.card.status().blocks_read, 2);
        h.set_cs(1);

        // Beyond image length but within >=64 MB capacity reads zero.
        assert_eq!(h.send_cmd(17, 130_000), 0x00);
        let bz = h.read_packet(512);
        assert_eq!(bz, vec![0u8; 512]);
        h.set_cs(1);
    }

    #[test]
    fn crc_checks_on_cmd0_cmd8_and_cmd59() {
        let mut h = Host::new(Vec::new());
        h.wake();
        // Bad CRC on CMD0
        assert_eq!(h.send_cmd_raw(0, 0, 0x01), 0x09);
        h.set_cs(1);
        assert_eq!(h.send_cmd(0, 0), 0x01);
        h.set_cs(1);
        // Bad CRC on CMD8
        assert_eq!(h.send_cmd_raw(8, 0x1aa, 0x01), 0x09);
        h.set_cs(1);
        assert_eq!(h.card.status().crc_errors, 2);

        h.init();
        // With CRC off, dummy CRC is accepted on CMD58.
        assert_eq!(h.send_cmd_raw(58, 0, 0x01), 0x00);
        for _ in 0..4 {
            h.xfer(0xff);
        }
        h.set_cs(1);

        // Turn CRC on with CMD59(1).
        assert_eq!(h.send_cmd(59, 1), 0x00);
        h.set_cs(1);
        assert_eq!(h.send_cmd_raw(58, 0, 0x01), 0x08);
        h.set_cs(1);
        assert_eq!(h.card.status().crc_errors, 3);

        // Write with bad CRC16 while CRC is on -> rejected (0x0B).
        assert_eq!(h.send_cmd(24, 5), 0x00);
        h.xfer(0xff);
        h.xfer(0xfe);
        for _ in 0..512 {
            h.xfer(0x33);
        }
        h.xfer(0x00);
        h.xfer(0x00);
        assert_eq!(h.xfer(0xff) & 0x1f, 0x0b);
        h.set_cs(1);
        assert_eq!(h.card.status().crc_errors, 4);
        assert_eq!(h.card.status().blocks_written, 0);

        // Turn CRC off with CMD59(0).
        assert_eq!(h.send_cmd(59, 0), 0x00);
        h.set_cs(1);
        assert_eq!(h.send_cmd_raw(58, 0, 0x01), 0x00);
        for _ in 0..4 {
            h.xfer(0xff);
        }
        h.set_cs(1);
    }

    #[test]
    fn multi_block_read_with_cmd12() {
        let mut img = vec![0u8; 10 * 512];
        for b in 0..10u8 {
            for i in 0..512 {
                img[b as usize * 512 + i] = b.wrapping_mul(17).wrapping_add(i as u8);
            }
        }
        let mut h = Host::new(img.clone());
        h.init();

        assert_eq!(h.send_cmd(18, 2), 0x00);
        assert_eq!(h.card.status().state, SdState::Reading);
        let b2 = h.read_packet(512);
        let b3 = h.read_packet(512);
        let b4 = h.read_packet(512);
        assert_eq!(b2, img[2 * 512..3 * 512]);
        assert_eq!(b3, img[3 * 512..4 * 512]);
        assert_eq!(b4, img[4 * 512..5 * 512]);

        // Stop with CMD12 while CS stays low.
        assert_eq!(h.send_cmd(12, 0), 0x00);
        assert_eq!(h.card.status().state, SdState::Ready);
        assert_eq!(h.card.status().blocks_read, 3);
        h.set_cs(1);
    }

    #[test]
    fn write_then_read_back_single_and_multi() {
        let mut h = Host::new(Vec::new());
        h.init();

        let block7: Vec<u8> = (0..512).map(|i| (i as u8).wrapping_add(0x10)).collect();
        assert_eq!(h.send_cmd(24, 7), 0x00);
        assert_eq!(h.card.status().state, SdState::Writing);
        h.xfer(0xff);
        h.xfer(0xfe);
        for &b in &block7 {
            h.xfer(b);
        }
        let c = crc16(&block7);
        h.xfer((c >> 8) as u8);
        h.xfer(c as u8);
        assert_eq!(h.xfer(0xff) & 0x1f, 0x05);
        for _ in 0..16 {
            assert_eq!(h.xfer(0xff), 0x00);
        }
        assert_eq!(h.xfer(0xff), 0xff);
        assert_eq!(h.card.status().state, SdState::Ready);
        h.set_cs(1);

        // Read back via SPI and via image().
        assert_eq!(h.send_cmd(17, 7), 0x00);
        assert_eq!(h.read_packet(512), block7);
        h.set_cs(1);
        assert_eq!(&h.card.image()[7 * 512..8 * 512], &block7[..]);

        // Multi-block write CMD25 at blocks 10, 11 with 0xFC and 0xFD stop tran.
        let b10 = vec![0xaa; 512];
        let b11 = vec![0x55; 512];
        assert_eq!(h.send_cmd(25, 10), 0x00);
        for blk in [&b10, &b11] {
            h.xfer(0xff);
            h.xfer(0xfc);
            for &b in blk {
                h.xfer(b);
            }
            let c = crc16(blk);
            h.xfer((c >> 8) as u8);
            h.xfer(c as u8);
            assert_eq!(h.xfer(0xff) & 0x1f, 0x05);
            for _ in 0..16 {
                assert_eq!(h.xfer(0xff), 0x00);
            }
            assert_eq!(h.xfer(0xff), 0xff);
        }
        h.xfer(0xfd);
        for _ in 0..16 {
            assert_eq!(h.xfer(0xff), 0x00);
        }
        assert_eq!(h.xfer(0xff), 0xff);
        assert_eq!(h.card.status().state, SdState::Ready);
        assert_eq!(h.card.status().blocks_written, 3);
        h.set_cs(1);

        assert_eq!(h.send_cmd(18, 10), 0x00);
        assert_eq!(h.read_packet(512), b10);
        assert_eq!(h.read_packet(512), b11);
        assert_eq!(h.send_cmd(12, 0), 0x00);
        h.set_cs(1);
    }

    #[test]
    fn illegal_command_and_cs_deasserted_mid_command() {
        let mut h = Host::new(Vec::new());
        h.init();

        let v0 = h.card.version();
        assert_eq!(h.send_cmd(42, 0), 0x04);
        h.set_cs(1);
        assert_eq!(h.card.status().illegal_commands, 1);
        assert!(h.card.version() > v0);

        // CMD41 without CMD55 is illegal.
        assert_eq!(h.send_cmd(41, 0), 0x04);
        h.set_cs(1);
        assert_eq!(h.card.status().illegal_commands, 2);

        // CS deasserted mid-command: partial 3 bytes are discarded.
        h.set_cs(0);
        h.xfer(0x40 | 17);
        h.xfer(0x00);
        h.xfer(0x00);
        h.set_cs(1);
        assert_eq!(h.card.miso(), 1);
        // Next full command succeeds cleanly.
        assert_eq!(h.send_cmd(17, 0), 0x00);
        assert_eq!(h.read_packet(512), vec![0u8; 512]);
        h.set_cs(1);
    }
}
