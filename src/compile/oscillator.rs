//! The internal oscillator OSCG (kernel/osc.rs), from the board slice HC06.
use super::*;
use crate::kernel::{OscillatorConfig, Time};

/// The nominal OSCG base frequency, 310 MHz before the divider: nextpnr's
/// timing model. HC06 measured 311.8 MHz on the ULX3S.
pub const NOMINAL_OSCILLATOR_HZ: u64 = 310_000_000;

/// The divider the chip applies for each OSC.DIV bit pattern of the Trellis
/// database, measured on the board (HC06). The database encodes OSC.DIV in
/// six bits, so above 31 several values share one pattern; each entry lists
/// those values. nextpnr writes DIV=128 as 127.
pub const OSCILLATOR_DIVIDERS: &[(&[u32], u64)] = &[
    (&[2], 2),
    (&[3], 3),
    (&[4], 4),
    (&[5], 5),
    (&[6], 6),
    (&[7], 7),
    (&[8], 8),
    (&[9], 9),
    (&[10], 10),
    (&[11], 11),
    (&[12], 12),
    (&[13], 13),
    (&[14], 14),
    (&[15], 15),
    (&[16], 16),
    (&[17], 17),
    (&[18], 18),
    (&[19], 19),
    (&[20], 20),
    (&[21], 21),
    (&[22], 22),
    (&[23], 23),
    (&[24], 24),
    (&[25], 25),
    (&[26], 26),
    (&[27], 27),
    (&[28], 28),
    (&[29], 29),
    (&[30], 30),
    (&[31], 31),
    (&[32, 33], 32),
    (&[34, 35], 34),
    (&[36, 37], 36),
    (&[38, 39], 38),
    (&[40, 41], 40),
    (&[42, 43], 42),
    (&[44, 45], 44),
    (&[46, 47], 46),
    (&[48, 49], 48),
    (&[50, 51], 50),
    (&[52, 53], 52),
    (&[54, 55], 54),
    (&[56, 57], 56),
    (&[58, 59], 58),
    (&[60, 61], 60),
    (&[62, 63], 62),
    (&[64, 65, 66], 64),
    (&[67, 68, 69, 70], 68),
    (&[71, 72, 73, 74], 72),
    (&[75, 76, 77, 78], 76),
    (&[79, 80, 81, 82], 80),
    (&[83, 84, 85, 86], 84),
    (&[87, 88, 89, 90], 88),
    (&[91, 92, 93, 94], 92),
    (&[95, 96, 97, 98], 96),
    (&[99, 100, 101, 102], 100),
    (&[103, 104, 105, 106], 104),
    (&[107, 108, 109, 110], 108),
    (&[111, 112, 113, 114], 112),
    (&[115, 116, 117, 118], 116),
    (&[119, 120, 121, 122], 120),
    (&[123, 124, 125, 126], 124),
    (&[127], 128),
];

impl Planner {
    pub(super) fn execute_oscillator(&mut self, id: ResourceId, port: &str) -> Result<()> {
        let key = self.device.model().resource(id).key.clone();
        ensure!(
            port == "OSC",
            "OSCG output {port} is not implemented: {key}"
        );
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let field = |name: &str| {
            self.device
                .resource_settings(id)
                .and_then(|r| r.fields.iter().find(|f| f.name == name))
                .map(|f| f.candidates.clone())
                .with_context(|| format!("{key} has no {name} setting"))
        };
        ensure!(
            field("OSC.MODE")? == ["OSCG"],
            "{key} OSC.MODE {:?} is not OSCG",
            field("OSC.MODE")?
        );
        let mut values = field("OSC.DIV")?
            .iter()
            .map(|v| v.parse::<u32>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        values.sort_unstable();
        let divider = OSCILLATOR_DIVIDERS
            .iter()
            .find(|(pattern, _)| *pattern == values.as_slice())
            .map(|&(_, divider)| divider)
            .with_context(|| {
                format!("{key} OSC.DIV bits match {values:?}; no measured divider for them")
            })?;
        let half_period = Time::new(divider * 1_000_000_000_000, 2 * self.oscillator_hz)?;
        let output = self.signal(&self.resource_port(id, "OSC")?)?;
        self.builder.oscillator(OscillatorConfig {
            resource: id,
            output,
            half_period,
        })?;
        self.record_resource(id, "OSCG: base frequency from the board profile (nominal 310 MHz unless given) over the divider measured for its OSC.DIV bit pattern (HC06); free-running from time 0, low then rising after half a period; wander not modelled");
        Ok(())
    }
}
