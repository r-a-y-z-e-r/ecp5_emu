//! The ULX3S around the chip: the balls of its LEDs, buttons, FTDI UART and
//! 25 MHz oscillator, after the official `ulx3s_v20.lpf` (the board revision
//! 3.0.8 was checked against a physical board); the balls of its HDMI, OLED,
//! SD card and SDRAM, after the v3.0.8 PCB (tools/check_peripheral_balls.py);
//! and which of them a configured design uses, from its decoded PIO modes
//! (decode/pio.rs).
use crate::prepared::ConfiguredDevice;
use anyhow::Result;

pub const PACKAGE: &str = "CABGA381";
/// The 25 MHz oscillator.
pub const CLOCK: &str = "G2";
pub const CLOCK_PERIOD_NS: u64 = 40;
/// `led[0]` .. `led[7]`, lit when high.
pub const LEDS: [&str; 8] = ["B2", "C2", "C1", "D2", "D1", "E2", "E1", "H3"];

/// SW1..SW4, `sw[0]` first (leftmost lever in the 3D board).
/// In assets/ulx3s/ulx3s_v3.0.8.kicad_pcb, footprint SW1 pads 8/7/6/5
/// use nets SW1/SW2/SW3/SW4 to U1 E8/D8/D7/E7. Pads 1..4 share
/// /blinkey/SWPU, through R51 (130 Ohm) to 2V5_3V3. ON connects the
/// ball to that supply (1); OFF floats, leaving the pad's pull to decide.
/// No other part is on these four nets; ON is toward the board's top edge.
pub const SWITCHES: [&str; 4] = ["E8", "D8", "D7", "E7"];

/// GPDI positive pairs: `gpdi_dp[0..3]`, blue, green, red, pixel clock.
/// These and the peripheral balls below were checked against U1 in the
/// ULX3S v3.0.8 PCB; its HDMI nets are named FPDI.
pub const GPDI_DP: [&str; 4] = ["A16", "A14", "A12", "A17"];
/// `gpdi_dn[0..3]`, the negative halves of the same pairs.
pub const GPDI_DN: [&str; 4] = ["B16", "C14", "A13", "B18"];
/// Audio jack DAC bits, least significant first, from
/// assets/ulx3s/ulx3s_v3.0.8.kicad_pcb (title-block revision v3.0.7).
/// R14..R17 connect these balls to /analog/AUDIO_L, AUDIO1 pad 2.
pub const AUDIO_L: [&str; 4] = ["E4", "D3", "C3", "B3"];
/// R18..R21 to /analog/AUDIO_R, AUDIO1 pad 3, in the same PCB.
pub const AUDIO_R: [&str; 4] = ["A3", "B5", "D5", "C5"];
/// R57..R60 to /analog/AUDIO_V, AUDIO1 pad 4, in the same PCB.
pub const AUDIO_V: [&str; 4] = ["H5", "F2", "F5", "E5"];
/// Each channel's ladder resistance in Ohms, bit 0 first. Only the
/// resistors and jack meet these nodes; nominal FPGA highs are 3.3 V.
pub const AUDIO_OHMS: [f64; 4] = [1100.0, 549.0, 270.0, 130.0];
/// SSD1331 OLED SPI clock, MOSI, data/command, reset and chip select.
pub const OLED_CLK: &str = "P4";
pub const OLED_MOSI: &str = "P3";
pub const OLED_DC: &str = "P1";
pub const OLED_RESN: &str = "P2";
pub const OLED_CSN: &str = "N2";
/// SD card clock, command, data bits 0..3, card detect and write protect.
pub const SD_CLK: &str = "H2";
pub const SD_CMD: &str = "J1";
pub const SD_D: [&str; 4] = ["J3", "H1", "K1", "K2"];
pub const SD_CDN: &str = "N5";
pub const SD_WP: &str = "P5";
/// IS25LP128F configuration flash. SCK is dedicated CCLK, driven through USRMCLK.
pub const FLASH_SCK: &str = "U3";
pub const FLASH_CSN: &str = "R2";
pub const FLASH_MOSI: &str = "W2";
pub const FLASH_MISO: &str = "V2";
pub const FLASH_WPN: &str = "Y2";
pub const FLASH_HOLDN: &str = "W1";

/// MCP7940NT U7 pads 6 (SCL) and 5 (SDA), via FPDI_SCL/FPDI_SDA
/// to U1 E12/B19 in assets/ulx3s/ulx3s_v3.0.8.kicad_pcb.
/// R22/R23 are 3.3 kOhm pull-ups; U11 PCA9306 shares this bus with HDMI DDC.
pub const RTC_SCL: &str = "E12";
pub const RTC_SDA: &str = "B19";

/// MAX11125 SPI, traced through U8 pads 19 (SCLK), 20 (CS), 21 (DIN),
/// 24 (DOUT) to U1 in assets/ulx3s/ulx3s_v3.0.8.kicad_pcb.
pub const ADC_SCLK: &str = "P17";
pub const ADC_CSN: &str = "R17";
pub const ADC_MOSI: &str = "R16";
pub const ADC_MISO: &str = "U16";
/// Datasheet AIN0..AIN7 (MAX11120–MAX11128 Rev 5, pp.15–16) are U8 pads
/// 26,27,28,1,2,3,4,5. In the v3.0.8 PCB those nets are respectively
/// GN14,GP14,GN15,GP15,GN16,GP16,GN17,GP17, shared with these U1 balls.
/// Thus AIN0 is GN14, not GP14. U8 REF+ (pad 15) and VDD (17,18) use
/// /analog/ADC3V3; REF- (13) is GND: nominal VREF is 3.3 V, external.
pub const ADC_AIN: [&str; 8] = ["U17", "U18", "P16", "N17", "M17", "N16", "L17", "L16"];

/// 48LC16M16A2 SDRAM clock, clock enable and active-low commands.
pub const SDRAM_CLK: &str = "F19";
pub const SDRAM_CKE: &str = "F20";
pub const SDRAM_CSN: &str = "P20";
pub const SDRAM_WEN: &str = "T20";
pub const SDRAM_RASN: &str = "R20";
pub const SDRAM_CASN: &str = "T19";
/// SDRAM bank address, data masks, address and data, least significant first.
pub const SDRAM_BA: [&str; 2] = ["P19", "N20"];
pub const SDRAM_DQM: [&str; 2] = ["U19", "E20"];
pub const SDRAM_A: [&str; 13] = [
    "M20", "M19", "L20", "L19", "K20", "K19", "K18", "J20", "J19", "H20", "N19", "G20", "G19",
];
pub const SDRAM_D: [&str; 16] = [
    "J16", "L18", "M18", "N18", "P18", "T18", "T17", "U20", "E19", "D20", "D19", "C20", "E18",
    "F18", "J18", "J17",
];

/// One signal pad on the two 2x20 expansion connectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeaderPin {
    pub name: &'static str,
    pub ball: &'static str,
    pub connector: u8,
    pub pad: u8,
}
/// J1/J2 signal pads, in connector order, GN then GP. Evidence:
/// assets/ulx3s/ulx3s_v3.0.8.kicad_pcb (title block v3.0.7), J1/J2
/// pads 5..18 and 23..36, through nets GN0..27/GP0..27 to U1.
/// GP/GN11..13 also reach ESP32 U9 pins 6..11; it is assumed silent.
/// GP/GN14..17 share ADC AIN0..7. Power/ground/5 V pads are not signals.
pub const HEADER: [HeaderPin; 56] = [
    HeaderPin {
        name: "GN0",
        ball: "C11",
        connector: 1,
        pad: 5,
    },
    HeaderPin {
        name: "GP0",
        ball: "B11",
        connector: 1,
        pad: 6,
    },
    HeaderPin {
        name: "GN1",
        ball: "A11",
        connector: 1,
        pad: 7,
    },
    HeaderPin {
        name: "GP1",
        ball: "A10",
        connector: 1,
        pad: 8,
    },
    HeaderPin {
        name: "GN2",
        ball: "B10",
        connector: 1,
        pad: 9,
    },
    HeaderPin {
        name: "GP2",
        ball: "A9",
        connector: 1,
        pad: 10,
    },
    HeaderPin {
        name: "GN3",
        ball: "C10",
        connector: 1,
        pad: 11,
    },
    HeaderPin {
        name: "GP3",
        ball: "B9",
        connector: 1,
        pad: 12,
    },
    HeaderPin {
        name: "GN4",
        ball: "A8",
        connector: 1,
        pad: 13,
    },
    HeaderPin {
        name: "GP4",
        ball: "A7",
        connector: 1,
        pad: 14,
    },
    HeaderPin {
        name: "GN5",
        ball: "B8",
        connector: 1,
        pad: 15,
    },
    HeaderPin {
        name: "GP5",
        ball: "C8",
        connector: 1,
        pad: 16,
    },
    HeaderPin {
        name: "GN6",
        ball: "C7",
        connector: 1,
        pad: 17,
    },
    HeaderPin {
        name: "GP6",
        ball: "C6",
        connector: 1,
        pad: 18,
    },
    HeaderPin {
        name: "GN7",
        ball: "B6",
        connector: 1,
        pad: 23,
    },
    HeaderPin {
        name: "GP7",
        ball: "A6",
        connector: 1,
        pad: 24,
    },
    HeaderPin {
        name: "GN8",
        ball: "A5",
        connector: 1,
        pad: 25,
    },
    HeaderPin {
        name: "GP8",
        ball: "A4",
        connector: 1,
        pad: 26,
    },
    HeaderPin {
        name: "GN9",
        ball: "B1",
        connector: 1,
        pad: 27,
    },
    HeaderPin {
        name: "GP9",
        ball: "A2",
        connector: 1,
        pad: 28,
    },
    HeaderPin {
        name: "GN10",
        ball: "B4",
        connector: 1,
        pad: 29,
    },
    HeaderPin {
        name: "GP10",
        ball: "C4",
        connector: 1,
        pad: 30,
    },
    HeaderPin {
        name: "GN11",
        ball: "E3",
        connector: 1,
        pad: 31,
    },
    HeaderPin {
        name: "GP11",
        ball: "F4",
        connector: 1,
        pad: 32,
    },
    HeaderPin {
        name: "GN12",
        ball: "F3",
        connector: 1,
        pad: 33,
    },
    HeaderPin {
        name: "GP12",
        ball: "G3",
        connector: 1,
        pad: 34,
    },
    HeaderPin {
        name: "GN13",
        ball: "G5",
        connector: 1,
        pad: 35,
    },
    HeaderPin {
        name: "GP13",
        ball: "H4",
        connector: 1,
        pad: 36,
    },
    HeaderPin {
        name: "GN14",
        ball: "U17",
        connector: 2,
        pad: 5,
    },
    HeaderPin {
        name: "GP14",
        ball: "U18",
        connector: 2,
        pad: 6,
    },
    HeaderPin {
        name: "GN15",
        ball: "P16",
        connector: 2,
        pad: 7,
    },
    HeaderPin {
        name: "GP15",
        ball: "N17",
        connector: 2,
        pad: 8,
    },
    HeaderPin {
        name: "GN16",
        ball: "M17",
        connector: 2,
        pad: 9,
    },
    HeaderPin {
        name: "GP16",
        ball: "N16",
        connector: 2,
        pad: 10,
    },
    HeaderPin {
        name: "GN17",
        ball: "L17",
        connector: 2,
        pad: 11,
    },
    HeaderPin {
        name: "GP17",
        ball: "L16",
        connector: 2,
        pad: 12,
    },
    HeaderPin {
        name: "GN18",
        ball: "H17",
        connector: 2,
        pad: 13,
    },
    HeaderPin {
        name: "GP18",
        ball: "H18",
        connector: 2,
        pad: 14,
    },
    HeaderPin {
        name: "GN19",
        ball: "G18",
        connector: 2,
        pad: 15,
    },
    HeaderPin {
        name: "GP19",
        ball: "F17",
        connector: 2,
        pad: 16,
    },
    HeaderPin {
        name: "GN20",
        ball: "E17",
        connector: 2,
        pad: 17,
    },
    HeaderPin {
        name: "GP20",
        ball: "D18",
        connector: 2,
        pad: 18,
    },
    HeaderPin {
        name: "GN21",
        ball: "D17",
        connector: 2,
        pad: 23,
    },
    HeaderPin {
        name: "GP21",
        ball: "C18",
        connector: 2,
        pad: 24,
    },
    HeaderPin {
        name: "GN22",
        ball: "C15",
        connector: 2,
        pad: 25,
    },
    HeaderPin {
        name: "GP22",
        ball: "B15",
        connector: 2,
        pad: 26,
    },
    HeaderPin {
        name: "GN23",
        ball: "C17",
        connector: 2,
        pad: 27,
    },
    HeaderPin {
        name: "GP23",
        ball: "B17",
        connector: 2,
        pad: 28,
    },
    HeaderPin {
        name: "GN24",
        ball: "D16",
        connector: 2,
        pad: 29,
    },
    HeaderPin {
        name: "GP24",
        ball: "C16",
        connector: 2,
        pad: 30,
    },
    HeaderPin {
        name: "GN25",
        ball: "E14",
        connector: 2,
        pad: 31,
    },
    HeaderPin {
        name: "GP25",
        ball: "D14",
        connector: 2,
        pad: 32,
    },
    HeaderPin {
        name: "GN26",
        ball: "C13",
        connector: 2,
        pad: 33,
    },
    HeaderPin {
        name: "GP26",
        ball: "B13",
        connector: 2,
        pad: 34,
    },
    HeaderPin {
        name: "GN27",
        ball: "E13",
        connector: 2,
        pad: 35,
    },
    HeaderPin {
        name: "GP27",
        ball: "D13",
        connector: 2,
        pad: 36,
    },
];
/// External levels in HEADER order; None releases the external driver.
pub type HeaderLevels = [Option<bool>; 56];

/// Strict named input settings. Empty/absent releases all pins. Values
/// are 0, 1 or z; unknown names, duplicates and malformed entries fail.
pub fn parse_header(value: Option<&str>) -> Result<HeaderLevels> {
    let mut levels = [None; 56];
    let Some(value) = value.filter(|s| !s.is_empty()) else {
        return Ok(levels);
    };
    let mut seen = [false; 56];
    for entry in value.split(',') {
        let (name, value) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("ECP5_HEADER needs NAME=0|1|z entries"))?;
        let index = HEADER
            .iter()
            .position(|p| p.name == name.trim())
            .ok_or_else(|| anyhow::anyhow!("ECP5_HEADER: unknown header name {}", name.trim()))?;
        anyhow::ensure!(
            !seen[index],
            "ECP5_HEADER: duplicate {}",
            HEADER[index].name
        );
        seen[index] = true;
        levels[index] = match value.trim() {
            "0" => Some(false),
            "1" => Some(true),
            "z" => None,
            _ => anyhow::bail!("ECP5_HEADER: {} needs 0, 1 or z", HEADER[index].name),
        };
    }
    Ok(levels)
}
/// The window and CLI read the same strict setting, without mutating it.
pub fn header_from_env() -> Result<HeaderLevels> {
    let value = std::env::var("ECP5_HEADER");
    match value {
        Ok(value) => parse_header(Some(&value)),
        Err(std::env::VarError::NotPresent) => parse_header(None),
        Err(error) => Err(anyhow::anyhow!(
            "ECP5_HEADER must contain NAME=0|1|z entries: {error}"
        )),
    }
}
/// Fast pure inputs are two-valued: release uses the pull, NONE uses 0.
/// A bidirectional pad has an external tristate driver and releases to Z.
pub fn header_level(
    direction: Direction,
    pull: Option<bool>,
    external: Option<bool>,
) -> Option<bool> {
    external.or_else(|| (direction == Direction::Input).then_some(pull.unwrap_or(false)))
}
/// Initial external drivers only for header balls the design reads.
pub fn header_inputs(
    device: &ConfiguredDevice,
    levels: &HeaderLevels,
) -> Result<Vec<(&'static str, Option<bool>)>> {
    let mut inputs = Vec::new();
    for (i, pin) in HEADER.iter().enumerate() {
        if let Some(direction) = direction(device, pin.ball)?.filter(|d| d.receives()) {
            inputs.push((
                pin.ball,
                header_level(direction, input_pull(device, pin.ball)?, levels[i]),
            ));
        }
    }
    Ok(inputs)
}

pub struct Button {
    pub name: &'static str,
    pub ball: &'static str,
    /// The level while it is not pressed.
    pub idle: bool,
}

/// `btn[0]` .. `btn[6]`. PWR pulls its line low when pressed; the others
/// drive it high.
pub const BUTTONS: [Button; 7] = [
    Button {
        name: "PWR",
        ball: "D6",
        idle: true,
    },
    Button {
        name: "FIRE1",
        ball: "R1",
        idle: false,
    },
    Button {
        name: "FIRE2",
        ball: "T1",
        idle: false,
    },
    Button {
        name: "UP",
        ball: "R18",
        idle: false,
    },
    Button {
        name: "DOWN",
        ball: "V1",
        idle: false,
    },
    Button {
        name: "LEFT",
        ball: "U1",
        idle: false,
    },
    Button {
        name: "RIGHT",
        ball: "H16",
        idle: false,
    },
];

/// The FTDI UART: the chip sends on `ftdi_rxd` and receives on `ftdi_txd`.
pub const UART_TX: &str = "L4";
pub const UART_RX: &str = "M1";
pub const BAUD: u64 = 115_200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Input,
    Output,
    Bidirectional,
}

impl Direction {
    pub fn drives(self) -> bool {
        self != Direction::Input
    }
    pub fn receives(self) -> bool {
        self != Direction::Output
    }
}

/// How the design uses a ball of the package: none when its PIO is
/// unwritten or the ball has no user-mode driver. The dedicated CCLK ball
/// is an output when USRMCLK owns it. The complement pad of a differential
/// output is an output.
pub fn direction(device: &ConfiguredDevice, ball: &str) -> Result<Option<Direction>> {
    let cclk = crate::compile::cclk_pad(device, PACKAGE)?;
    if ball == cclk.ball {
        return Ok(cclk.direction.map(|_| Direction::Output));
    }
    let model = device.model();
    let Some(id) = model.package_pin(PACKAGE, ball).and_then(|p| p.pio) else {
        return Ok(None);
    };
    let modes = device.pio_modes(id)?;
    let Some(mode) = modes.get(&model.resource(id).key) else {
        return Ok(None);
    };
    if mode.complement_of.is_some() {
        return Ok(Some(Direction::Output));
    }
    if !matches!(mode.status.as_str(), "resolved" | "ambiguous")
        || mode.base_types.is_empty()
        || mode.base_types.iter().any(|v| !v.contains('_'))
    {
        return Ok(None);
    }
    // Identical CRAM bits (e.g. PULLMODE=UP on an output) can match both OUTPUT
    // and BIDIR base types; report the direction common to every match, as
    // compile::Planner::pio does.
    let drives = mode
        .base_types
        .iter()
        .all(|v| v.starts_with("OUTPUT_") || v.starts_with("BIDIR_"));
    let receives = mode
        .base_types
        .iter()
        .all(|v| v.starts_with("INPUT_") || v.starts_with("BIDIR_"));
    Ok(match (drives, receives) {
        (true, true) => Some(Direction::Bidirectional),
        (true, false) => Some(Direction::Output),
        (false, true) => Some(Direction::Input),
        (false, false) => None,
    })
}

/// The configured weak input pull: Some(false) for DOWN, Some(true) for
/// UP, and None for NONE, missing or ambiguous settings. Use the same joint
/// PIO decode as `direction`, since PULLMODE shares bits with BASE_TYPE.
pub fn input_pull(device: &ConfiguredDevice, ball: &str) -> Result<Option<bool>> {
    let model = device.model();
    let Some(id) = model.package_pin(PACKAGE, ball).and_then(|p| p.pio) else {
        return Ok(None);
    };
    let modes = device.pio_modes(id)?;
    Ok(modes
        .get(&model.resource(id).key)
        .and_then(|mode| mode.overlays.get("PULLMODE"))
        .filter(|values| values.len() == 1)
        .and_then(|values| match values[0].as_str() {
            "DOWN" => Some(false),
            "UP" => Some(true),
            _ => None,
        }))
}

/// Parse ECP5_SWITCHES in sw[0]-first order; absent means all contacts OFF.
pub fn parse_switches(value: Option<&str>) -> Result<[bool; 4]> {
    let Some(value) = value else {
        return Ok([false; 4]);
    };
    anyhow::ensure!(
        value.len() == 4 && value.bytes().all(|b| b == b'0' || b == b'1'),
        "ECP5_SWITCHES must be exactly four 0/1 characters, sw[0] first (for example 1010)"
    );
    Ok(std::array::from_fn(|i| value.as_bytes()[i] == b'1'))
}

/// Idle levels of the board inputs the configured design reads. Buttons
/// use their released levels, UART RX is high, and a DIP input is high
/// when ON or pulled UP. OFF with no known pull uses the two-valued zero
/// fallback. The caller keeps explicit inputs and peripheral answers.
pub fn idle_inputs(
    device: &ConfiguredDevice,
    switches: [bool; 4],
) -> Result<Vec<(&'static str, bool)>> {
    let receives = |ball: &str| -> Result<bool> {
        Ok(direction(device, ball)?.is_some_and(Direction::receives))
    };
    let mut inputs = Vec::new();
    for button in BUTTONS {
        if receives(button.ball)? {
            inputs.push((button.ball, button.idle));
        }
    }
    for (index, ball) in SWITCHES.into_iter().enumerate() {
        if receives(ball)? {
            inputs.push((
                ball,
                switches[index] || input_pull(device, ball)?.unwrap_or(false),
            ));
        }
    }
    if receives(UART_RX)? {
        inputs.push((UART_RX, true));
    }
    Ok(inputs)
}

/// Every ball of the package that the design uses, in ball order.
pub fn used_balls(device: &ConfiguredDevice) -> Result<Vec<(String, Direction)>> {
    let cclk = crate::compile::cclk_pad(device, PACKAGE)?;
    let mut balls: Vec<String> = device
        .model()
        .package_pins()
        .filter(|p| {
            p.package == PACKAGE
                && (p.pio.is_some() || (p.ball == cclk.ball && cclk.direction.is_some()))
        })
        .map(|p| p.ball.clone())
        .collect();
    balls.sort();
    balls.dedup();
    let mut used = Vec::new();
    for ball in balls {
        if let Some(direction) = direction(device, &ball)? {
            used.push((ball, direction));
        }
    }
    Ok(used)
}

#[cfg(test)]
mod header_tests {
    use super::*;
    #[test]
    fn header_parser_and_release_rules() {
        assert_eq!(parse_header(None).unwrap(), [None; 56]);
        let levels = parse_header(Some("GP0=1, GN3=0,GP1=z")).unwrap();
        assert_eq!(
            (levels[1], levels[6], levels[3]),
            (Some(true), Some(false), None)
        );
        for invalid in [
            "GP28=1",
            "C11=0",
            "GP0=2",
            "GP0=1,GP0=0",
            "GP0=1,",
            "GP0",
            "=1",
        ] {
            assert!(parse_header(Some(invalid)).is_err(), "{invalid}");
        }
        for pull in [None, Some(false), Some(true)] {
            assert_eq!(
                header_level(Direction::Input, pull, None),
                Some(pull.unwrap_or(false))
            );
            assert_eq!(header_level(Direction::Bidirectional, pull, None), None);
            assert_eq!(header_level(Direction::Input, pull, Some(true)), Some(true));
            assert_eq!(
                header_level(Direction::Bidirectional, pull, Some(false)),
                Some(false)
            );
        }
    }
}
