//! The dedicated CCLK output pad, reached from the package rather than fan-in.
use super::*;

/// The dedicated CCLK ball and its configured user-mode direction.
/// `None` means USRMCLK is disabled; `Some(Output)` means it owns the output
/// driver. Dynamic PADDT (USRMCLKTS) and GOE do not change that direction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CclkPad {
    pub ball: String,
    pub direction: Option<Direction>,
}

/// Query CCLK without compiling a plan. The ball comes from package metadata;
/// configured ownership comes from EFB3_PICB1, independently of OSC.DIV's bits.
pub fn cclk_pad(device: &ConfiguredDevice, package: &str) -> Result<CclkPad> {
    let pins = device
        .model()
        .package_pins()
        .filter(|p| p.package.eq_ignore_ascii_case(package) && p.function == "CCLK")
        .collect::<Vec<_>>();
    ensure!(
        pins.len() == 1,
        "expected one CCLK ball in package {package}"
    );
    Ok(CclkPad {
        ball: pins[0].ball.clone(),
        direction: enabled(device)?.then_some(Direction::Output),
    })
}

pub(super) fn resource(device: &ConfiguredDevice) -> Result<ResourceId> {
    let ids = device
        .model()
        .resources()
        .filter(|(_, r)| r.family == "USRMCLK")
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    ensure!(ids.len() == 1, "expected one USRMCLK resource");
    Ok(ids[0])
}

fn mode_tile(device: &ConfiguredDevice) -> Result<TileId> {
    let tiles = device
        .model()
        .tiles()
        .filter(|(_, t)| t.kind == "EFB3_PICB1")
        .map(|(id, _)| id)
        .collect::<Vec<_>>();
    ensure!(tiles.len() == 1, "expected one CCLK EFB3_PICB1 tile");
    Ok(tiles[0])
}

fn enabled(device: &ConfiguredDevice) -> Result<bool> {
    // nextpnr writes USRMCLK only in EFB3_PICB1. EFB1_PICB1's apparent
    // CCLK.MODE bit (508,1135) belongs to OSC.DIV; nextpnr suppresses that
    // enum with _NONE_ when writing an OSCG. Composing both as replicas
    // falsely rejects a valid oscillator/USRMCLK combination, or makes
    // the oscillator's divider decide CCLK ownership. Never rewrite it.
    let values = device.enum_candidates(mode_tile(device)?, "CCLK.MODE")?;
    match values.as_slice() {
        [mode] if mode == "NONE" => Ok(false),
        [mode] if mode == "USRMCLK" => Ok(true),
        _ => bail!("unresolved EFB3_PICB1 CCLK.MODE: {values:?}"),
    }
}

impl Planner {
    pub(super) fn execute_usrmclk(&mut self, id: ResourceId) -> Result<()> {
        if !self.done_resources.insert(id) {
            return Ok(());
        }
        let tile = mode_tile(&self.device)?;
        self.field(tile, "CCLK.MODE")?;
        let pad = self.pad(id, "PAD_CCLK", false, false)?;
        if enabled(&self.device)? {
            let data = self.input(&self.resource_port(id, "PADDO")?)?;
            let tristate = self.input(&self.resource_port(id, "PADDT")?)?;
            self.pad_output_driver(id, data, tristate, pad)?;
        } else {
            self.report.undriven.push(Wire {
                location: self.device.model().resource(id).location,
                name: "PAD_CCLK".into(),
            });
        }
        self.record_resource(id, "CCLK.MODE from EFB3_PICB1 only (EFB1_PICB1 overlaps OSC.DIV); NONE leaves the pad undriven; USRMCLK drives non-inverted PADDO while PADDT=0 and GOE=1, otherwise Z; no inferred pull, receiver, configuration transport or pad timing; raw composed fields retained");
        Ok(())
    }
}
