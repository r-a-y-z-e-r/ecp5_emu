//! The ULX3S reference designs in examples/ulx3s: each one is built with
//! Yosys/nextpnr (examples/ulx3s/build.sh) and runs in the emulator with its
//! signals named as in the Verilog. These are the Stage 1 exit designs.
mod board;
use board::{decode_uart, BIT, PERIOD_PS};
use ecp5_emu::{
    compile::{self, Compiled, Reference, Request, Scope, Startup},
    database::Database,
    image::ConfigurationImage,
    kernel::{Options, Time},
    logic::Logic,
    model::DeviceModel,
    names::Names,
    prepared::{load_catalogue, ConfiguredDevice},
};
use std::{path::PathBuf, process::Command, sync::Arc};

struct Design {
    compiled: Compiled,
    names: Names,
}

/// Builds `examples/ulx3s/<name>` and compiles it with `probes` given as
/// HDL names (buses expand to their bits) and `pins` given as balls. The
/// clock and the `inputs` (HDL names) start at 0 and can be driven.
fn design(name: &str, probes: &[&str], pins: &[&str], inputs: &[&str]) -> Design {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = root.join("target/reference").join(name);
    let status = Command::new(root.join("examples/ulx3s/build.sh"))
        .arg(root.join("examples/ulx3s").join(name))
        .arg(&out)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "building {name}; logs in {}",
        out.display()
    );
    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let names = Names::load(&out.join("top_routed.json"), &model, "CABGA381").unwrap();
    let image = Arc::new(ConfigurationImage::load(&out.join("top.config"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );
    let clock = names.expand("clk_25mhz").unwrap().remove(0).1;
    let mut probes: Vec<Reference> = probes
        .iter()
        .flat_map(|p| names.expand(p).unwrap())
        .map(|(_, r)| r)
        .chain(pins.iter().map(|&b| board::pin(b)))
        .collect();
    probes.dedup();
    let compiled = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: std::iter::once(clock)
                .chain(inputs.iter().map(|i| names.expand(i).unwrap().remove(0).1))
                .map(|r| (r, Logic::Zero))
                .collect(),
            probes,
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap_or_else(|e| panic!("{name}: {e:#}"));
    Design { compiled, names }
}

impl Design {
    fn bits(&self, name: &str) -> Vec<Logic> {
        self.names
            .expand(name)
            .unwrap()
            .iter()
            .map(|(_, r)| self.compiled.value(r).unwrap())
            .collect()
    }
    /// The bus value, each bit at its own index: synthesis may drop bits.
    fn number(&self, name: &str) -> u64 {
        self.names
            .expand(name)
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, (bit, r))| {
                let index = bit
                    .rsplit_once('[')
                    .and_then(|(_, i)| i.strip_suffix(']')?.parse().ok())
                    .unwrap_or(i);
                let v = self.compiled.value(r).unwrap();
                u64::from(v.bit().unwrap_or_else(|| panic!("{bit} is {v}"))) << index
            })
            .sum()
    }
    /// One 25 MHz cycle: rising edge at (2n-1)·20 ns, falling edge at n·40 ns;
    /// `at_high` runs between them.
    fn cycle(&mut self, n: u64, mut at_high: impl FnMut(&mut Self)) {
        let clock = self.compiled.drivers["pin:G2"];
        let rise = Time::from_ps((2 * n - 1) * PERIOD_PS / 2);
        let fall = Time::from_ps(n * PERIOD_PS);
        let kernel = &mut self.compiled.kernel;
        kernel.schedule(rise, clock, Logic::One).unwrap();
        kernel.run_until(rise).unwrap();
        kernel
            .run_until(Time::from_ps((2 * n - 1) * PERIOD_PS / 2 + PERIOD_PS / 4))
            .unwrap();
        at_high(self);
        let kernel = &mut self.compiled.kernel;
        kernel.schedule(fall, clock, Logic::Zero).unwrap();
        kernel.run_until(fall).unwrap();
    }
}

/// The PLL multiplies the 25 MHz clock by 4. It learns the reference period
/// at the second reference rising edge (60 ns), so CLKOP rises at 70, 80, …
/// ns, and after n reference cycles (n·40 ns) the counter is 4n − 6.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn blinky_pll_counts_at_the_pll_rate() {
    let mut d = design("blinky_pll", &["count", "led", "locked"], &[], &[]);
    for n in 1..=1000 {
        d.cycle(n, |_| {});
    }
    assert_eq!(d.bits("locked"), [Logic::One]);
    let count = d.number("count");
    assert_eq!(count, 4 * 1000 - 6);
    // The LEDs are the counter's top byte.
    assert_eq!(d.number("led"), count >> 24);
}

/// The message the board printed: three loads, 15 messages each, all equal.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn uart_hello_prints_the_board_message() {
    let mut d = design("uart_hello", &["ftdi_rxd", "led"], &[], &[]);
    // The line idles high before the first frame, which starts at once.
    let mut samples = vec![true];
    for n in 1..=(16 * 10 * BIT) as u64 {
        d.cycle(n, |_| {});
        samples.push(d.bits("ftdi_rxd")[0] == Logic::One);
    }
    let text = decode_uart(&samples);
    assert!(text.starts_with("Hello, ULX3S!\r\n"), "{text:?}");
}

/// The message from block RAM: a ROM copied into a RAM at start, both EBR,
/// and sent from the RAM; the LEDs show the index read back with each byte.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn uart_ram_sends_the_message_from_block_ram() {
    let mut d = design("uart_ram", &["ftdi_rxd", "led"], &[], &[]);
    assert_eq!(d.compiled.report.counts.ebr_blocks, 2);
    let mut samples = vec![true];
    for n in 1..=(24 * 10 * BIT) as u64 {
        d.cycle(n, |_| {});
        samples.push(d.bits("ftdi_rxd")[0] == Logic::One);
    }
    let text = decode_uart(&samples);
    assert!(text.starts_with("Hello from block RAM!\r\n"), "{text:?}");
    // The last character sent was entry 22, so the LEDs show 22.
    assert_eq!(d.number("led"), 22);
}

/// ram32: a 512x32 RAM and ROM as yosys infers them, each one EBR read 36
/// bits at a time (PDPW16KD, DATA_WIDTH_R=36). The ROM words sum to the value
/// rom.hex gives, and the RAM reads back the LFSR words it was written.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn ram32_reads_36_bit_block_ram_words_back() {
    let mut d = design("ram32", &["ftdi_rxd", "led"], &[], &[]);
    let wide: Vec<_> = d.compiled.report.ebr.iter().map(|e| e.widths).collect();
    assert_eq!(wide, [[36, 36], [36, 36]]);
    let rom: u32 = include_str!("../examples/ulx3s/ram32/rom.hex")
        .lines()
        .map(|l| u32::from_str_radix(l, 16).unwrap())
        .fold(0, u32::wrapping_add);
    let mut samples = vec![true];
    // 512 writes, 514 reads, then the 23-character line.
    for n in 1..=(1100 + 24 * 10 * BIT) as u64 {
        d.cycle(n, |_| {});
        samples.push(d.bits("ftdi_rxd")[0] == Logic::One);
    }
    let text = decode_uart(&samples);
    assert_eq!(text, format!("ROM {rom:08X} RAM 0000\r\n"));
    assert_eq!(d.bits("led")[7], Logic::Zero);
}

/// mac: `*` in the DSP blocks (MULT18X18D without registers, as yosys writes
/// them): a signed 16x16 dot product and an unsigned 32x32 product split over
/// four multipliers. The line matches the arithmetic of top.v done here.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn mac_prints_the_dot_product_of_its_dsp_multipliers() {
    let mut d = design("mac", &["ftdi_rxd", "led"], &[], &[]);
    assert!(d.compiled.report.counts.multiplier18 >= 5);
    let (mut x, mut y, mut u, mut w) = (0xACE1_u32, 0x1D2B_u32, 0x9E37_79B9_u32, 0x7F4A_7C15_u32);
    let (mut mac, mut mix) = (0_i64, 0_u32);
    for _ in 0..4096 {
        mac += i64::from(x as u16 as i16) * i64::from(y as u16 as i16);
        mix ^= u.wrapping_mul(w);
        x = (x << 1 & 0xffff) | ((x >> 15 ^ x >> 13 ^ x >> 12 ^ x >> 10) & 1);
        y = (y << 1 & 0xffff) | ((y >> 15 ^ y >> 14 ^ y >> 12 ^ y >> 3) & 1);
        u = u.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        w = (w << 1) | ((w >> 31 ^ w >> 21 ^ w >> 1 ^ w) & 1);
    }
    let expected = format!("MAC {:010X} MIX {mix:08X}\r\n", mac & ((1 << 40) - 1));
    let mut samples = vec![true];
    for n in 1..=(4100 + 30 * 10 * BIT) as u64 {
        d.cycle(n, |_| {});
        samples.push(d.bits("ftdi_rxd")[0] == Logic::One);
    }
    assert_eq!(decode_uart(&samples), expected);
}

/// picorv32_console: the stock PicoRV32 runs its firmware from four EBR, its
/// register file in LUTRAM. The event kernel is too slow to reach the console
/// (fast mode does, in tests/fast_mode.rs), so here the program counter
/// follows an RTL simulation of top.v with iverilog through the start-up
/// code, cycle for cycle: the reset counter, then the loop that clears .bss.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn picorv32_console_starts_its_firmware_as_in_rtl_simulation() {
    let mut d = design("picorv32_console", &["cpu.reg_pc"], &[], &[]);
    let counts = &d.compiled.report.counts;
    assert_eq!((counts.ebr_blocks, counts.lutram_columns), (4, 128));
    // (cycle whose rising edge changed the PC, new PC), from iverilog.
    let rtl = [
        (71, 0x4),
        (75, 0x8),
        (79, 0xc),
        (83, 0x10),
        (88, 0x14),
        (95, 0x18),
        (98, 0x1c),
        (100, 0x10),
        (107, 0x14),
        (114, 0x18),
        (117, 0x1c),
        (119, 0x10),
    ];
    let mut trace = Vec::new();
    let mut pc = d.number("cpu.reg_pc");
    assert_eq!(pc, 0);
    for n in 1..=120 {
        d.cycle(n, |_| {});
        let now = d.number("cpu.reg_pc");
        if now != pc {
            trace.push((n, now));
            pc = now;
        }
    }
    assert_eq!(trace, rtl);
}

/// Bouncing presses count once each; a pulse shorter than the debounce time
/// does not count. The press counter is the LED port: Yosys merges the
/// `presses` register into `led`, so only that name is left.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn debounce_counts_bouncy_presses_once() {
    let mut d = design("debounce", &["led", "level"], &[], &["fire1"]);
    let button = d.compiled.drivers["pin:R1"];
    // (cycles, level): 0 = released, 1 = pressed, 2 = bouncing every 100 cycles.
    let script = [
        (10_000, 0),
        (25_000, 2), // press, bouncing for 1 ms
        (100_000, 1),
        (25_000, 2), // release, bouncing
        (100_000, 0),
        (25_000, 1), // a 1 ms pulse: shorter than 2.6 ms
        (100_000, 0),
        (25_000, 2), // second real press
        (100_000, 1),
    ];
    let mut n = 0;
    let mut level = Logic::Zero;
    let mut presses_after = Vec::new();
    for (cycles, kind) in script {
        for i in 0..cycles {
            n += 1;
            let want = match kind {
                0 => Logic::Zero,
                1 => Logic::One,
                _ => Logic::from_bool((i / 100) % 2 == 0),
            };
            if want != level {
                let t = Time::from_ps((2 * n - 1) * PERIOD_PS / 2 - PERIOD_PS / 4);
                d.compiled.kernel.schedule(t, button, want).unwrap();
                level = want;
            }
            d.cycle(n, |_| {});
        }
        presses_after.push(d.number("led"));
    }
    assert_eq!(presses_after, [0, 0, 1, 1, 1, 1, 1, 1, 2]);
}

/// Lanes 0-2 carry the TMDS control symbol 1101010100 (bit 0 first), lane 3
/// five ones and five zeros, aligned to the symbol; every complement pin is
/// the inverse of its lane.
#[test]
#[ignore = "requires Yosys, nextpnr, the native image reader and a local Trellis database"]
fn ddr_pattern_sends_tmds_symbols_on_complementary_pairs() {
    // gpdi_dn[0..3]: the complement pads of the gpdi_dp pairs.
    let complements = ["B16", "C14", "A13", "B18"];
    let mut d = design("ddr_pattern", &["gpdi_dp"], &complements, &[]);
    let lanes = d.names.expand("gpdi_dp").unwrap();
    let mut streams = vec![Vec::new(); 4];
    let sample = |d: &Design, streams: &mut Vec<Vec<Logic>>| {
        for (lane, (_, reference)) in lanes.iter().enumerate() {
            let p = d.compiled.value(reference).unwrap();
            let n = d.compiled.value(&board::pin(complements[lane])).unwrap();
            assert_eq!(n, p.invert(), "lane {lane} complement");
            streams[lane].push(p);
        }
    };
    for n in 1..=40 {
        // The high phase carries D0, the low phase that follows D1.
        d.cycle(n, |d| sample(d, &mut streams));
        let kernel = &mut d.compiled.kernel;
        kernel
            .run_until(Time::from_ps(n * PERIOD_PS + PERIOD_PS / 4))
            .unwrap();
        sample(&d, &mut streams);
    }
    let bits: Vec<Vec<bool>> = streams
        .iter()
        .map(|s| s[10..].iter().map(|v| *v == Logic::One).collect())
        .collect();
    assert_eq!(bits[0], bits[1]);
    assert_eq!(bits[0], bits[2]);
    // A symbol starts where the clock lane rises.
    let start = (1..bits[3].len())
        .find(|&i| !bits[3][i - 1] && bits[3][i])
        .unwrap();
    let ctrl: Vec<bool> = (0..10).map(|i| 0b1101010100 >> i & 1 == 1).collect();
    for (k, chunk) in bits[0][start..].chunks_exact(10).enumerate() {
        assert_eq!(chunk, ctrl.as_slice(), "symbol {k}");
    }
    for chunk in bits[3][start..].chunks_exact(10) {
        assert_eq!(
            chunk,
            [true, true, true, true, true, false, false, false, false, false]
        );
    }
}
