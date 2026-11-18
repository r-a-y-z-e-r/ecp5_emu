//! The routed SD reader bitstream, sampled at every clock edge against the
//! SPI-mode SDHC model, checked by the line printed on the UART.
mod common;
use ecp5_emu::{
    compile::{self, Reference, Request, Scope, Startup},
    database::Database,
    fast::FastSim,
    image::ConfigurationImage,
    kernel::{Options, StartsUnknown, Time},
    logic::Logic,
    model::DeviceModel,
    peripherals::sdcard::{
        SdCard, SdPins, SdState, CLK_BALL, CS_BALL, MIN_CAPACITY_BYTES, MISO_BALL, MOSI_BALL,
    },
    prepared::{load_catalogue, ConfiguredDevice},
    uart::Receiver,
};
use std::{fmt::Write, path::PathBuf, sync::Arc, time::Instant};

#[test]
#[ignore = "requires clang, yosys/nextpnr and a local Trellis database"]
fn sd_read_initialises_card_and_prints_block_zero() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let out = common::reference::build("sd_read");

    let db = Database::discover(None).unwrap();
    let evidence = root.join("docs/inventory/LFE5U-85F");
    let model = Arc::new(DeviceModel::load(&db, &evidence).unwrap());
    let image = Arc::new(ConfigurationImage::load(&out.join("top.config"), &db, None).unwrap());
    let device = Arc::new(
        ConfiguredDevice::bind(
            Arc::clone(&model),
            image,
            load_catalogue(&model, &evidence).unwrap(),
        )
        .unwrap(),
    );

    let references: Vec<_> = [CS_BALL, CLK_BALL, MOSI_BALL, "L4"]
        .map(|ball| Reference::Pin(ball.into()))
        .into();
    let mut c = compile::compile(
        device,
        Request {
            package: "CABGA381".into(),
            scope: Scope::Observed,
            startup: Startup::Configured,
            drives: vec![
                (Reference::Pin("G2".into()), Logic::Zero),
                (Reference::Pin(MISO_BALL.into()), Logic::One),
            ],
            probes: references.clone(),
            options: Options {
                trace_capacity: 0,
                ..Options::default()
            },
        },
    )
    .unwrap();

    let probes: Vec<_> = references.iter().map(|r| c.probes[&r.label()]).collect();
    let clock = c.drivers["pin:G2"];
    let miso_driver = c.drivers[&format!("pin:{MISO_BALL}")];

    let mut disk = vec![0u8; MIN_CAPACITY_BYTES];
    let header: [u8; 16] = [
        0xeb, 0x3c, 0x90, 0x45, 0x43, 0x50, 0x35, 0x5f, 0x45, 0x4d, 0x55, 0x00, 0x02, 0x08, 0x01,
        0x00,
    ];
    disk[..16].copy_from_slice(&header);
    disk[510] = 0x55;
    disk[511] = 0xaa;

    let mut head_hex = String::with_capacity(32);
    for b in header {
        write!(head_hex, "{b:02X}").unwrap();
    }
    let expected = format!("SD OCR C0FF8000 B0 {head_hex} 55AA\r\n");

    let mut sd = SdCard::new(disk);
    let mut receiver = Receiver::new(115_200);
    let mut line = Vec::new();
    let mut edge = 0u64;

    let sample = |sd: &mut SdCard, receiver: &mut Receiver, row: &[u8], time_ps: u64| {
        sd.feed(
            time_ps,
            SdPins {
                cs: row[0],
                clk: row[1],
                mosi: row[2],
            },
        );
        assert!(row[3] <= 1, "UART is unknown at {time_ps} ps");
        receiver.sample(time_ps / 1000, row[3] == 1)
    };
    let levels = |c: &compile::Compiled| -> Vec<u8> {
        probes
            .iter()
            .map(|&p| match c.kernel.value(p).unwrap() {
                Logic::Zero => 0,
                Logic::One => 1,
                Logic::X => 2,
                Logic::Z => 3,
            })
            .collect()
    };

    sample(&mut sd, &mut receiver, &levels(&c), 0);
    let mut prev_miso = sd.miso();

    let mut fast = loop {
        match FastSim::new(&c.kernel) {
            Ok(fast) => break fast,
            Err(e) if e.downcast_ref::<StartsUnknown>().is_some() && edge < 2000 => {
                for level in [Logic::One, Logic::Zero] {
                    edge += 1;
                    let time_ps = edge * 20_000;
                    let time = Time::from_ps(time_ps);
                    c.kernel.schedule(time, clock, level).unwrap();
                    c.kernel.run_until(time).unwrap();
                    assert!(sample(&mut sd, &mut receiver, &levels(&c), time_ps).is_none());
                    let miso = sd.miso();
                    if miso != prev_miso {
                        prev_miso = miso;
                        let miso_time = Time::from_ps(time_ps + 1);
                        c.kernel
                            .schedule(
                                miso_time,
                                miso_driver,
                                if miso == 1 { Logic::One } else { Logic::Zero },
                            )
                            .unwrap();
                        c.kernel.run_until(miso_time).unwrap();
                    }
                }
            }
            Err(e) => panic!("{e:#}"),
        }
    };
    println!("SD warm start: {} cycles", edge / 2);

    fast.clock(clock, Time::from_ps(40_000)).unwrap();
    let start = Instant::now();
    let mut uart_line = None;

    while edge < 2_000_000 {
        let rows = fast.run_edges_sampled(1, &probes).unwrap();
        edge += 1;
        let now = fast.now().unwrap();
        let (now_ps, den) = now.ratio();
        assert_eq!(den, 1);
        let byte = sample(&mut sd, &mut receiver, &rows, now_ps);
        let miso = sd.miso();
        if miso != prev_miso {
            prev_miso = miso;
            fast.set_at(
                now.checked_add(Time::from_ps(1)).unwrap(),
                &[(miso_driver, miso == 1)],
            )
            .unwrap();
        }
        if let Some(byte) = byte {
            line.push(byte);
            if byte == b'\n' {
                uart_line = Some(String::from_utf8(line.clone()).unwrap());
                break;
            }
        }
    }

    let seconds = start.elapsed().as_secs_f64();
    let got = uart_line.expect("timed out waiting for SD UART line");
    println!(
        "SD UART: {:?} ({} cycles in {seconds:.3} s, {:.2} MHz)",
        got.trim_end(),
        edge / 2,
        (edge as f64 / 2.0) / seconds / 1e6
    );
    assert_eq!(got, expected);
    assert_eq!(receiver.framing_errors, 0);
    let st = sd.status();
    assert_eq!(st.state, SdState::Ready);
    assert_eq!(st.blocks_read, 1);
    assert_eq!(st.crc_errors, 0);
    assert_eq!(st.illegal_commands, 0);
}
