//! Bind global side effects even when the GSR input is outside probe fan-in.
use super::*;
use crate::kernel::{SystemConfig, WakeSequence};

#[derive(Default, Serialize, serde::Deserialize, Clone)]
pub struct SystemPlan {
    #[serde(default)]
    pub configuration_ports: Vec<ConfigurationPortUse>,
    pub user_gsr_resource: Option<String>,
    pub user_gsr_mode: Option<String>,
    pub sync_candidates: Vec<String>,
    pub wake_sequence: Option<WakeSequence>,
    pub gsr_enabled_ff: usize,
    pub gsr_disabled_ff: usize,
    pub flip_flop_initial: String,
    pub rule: String,
}
impl Planner {
    pub(super) fn system_input(&self, port: SystemPort) -> Result<Input> {
        Ok((*self
            .system
            .as_ref()
            .context("missing system controller")?
            .signals
            .get(&port)
            .context("missing system strobe")?)
        .into())
    }
    fn gsr_field(&self, id: ResourceId, suffix: &str) -> Result<Vec<String>> {
        let fields = self
            .device
            .resource_settings(id)
            .context("missing GSR configuration")?;
        let field = fields
            .fields
            .iter()
            .find(|f| f.name.ends_with(suffix))
            .context("missing GSR field")?;
        ensure!(
            ["resolved", "ambiguous", "unobserved"].contains(&field.status.as_str())
                && !field.candidates.is_empty(),
            "unresolved global user GSR field {}",
            field.name
        );
        Ok(field.candidates.clone())
    }
    fn tile_of_kind(&self, kind: &str) -> Result<TileId> {
        let tiles = self
            .device
            .model()
            .tiles()
            .filter(|(_, t)| t.kind == kind)
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        ensure!(tiles.len() == 1, "expected one system tile {kind}");
        Ok(tiles[0])
    }
    fn gsr_clock_routed(&self, wire: &Wire, seen: &mut HashSet<WireId>) -> Result<bool> {
        let wire = self.index.canonical(self.device.model(), wire)?;
        if !seen.insert(wire) {
            return Ok(false);
        }
        if self.producers.contains_key(&wire) {
            return Ok(true);
        }
        for incoming in self.index.incoming(self.device.model(), wire)? {
            for connection in
                self.device
                    .matching_connections_to(incoming.tile, &incoming.raw, None)
            {
                if let Endpoint::Wire(source) = &connection?.source {
                    if self.gsr_clock_routed(source, seen)? {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    pub(super) fn configure_system(&mut self, startup: Startup) -> Result<()> {
        let ids = self
            .device
            .model()
            .resources()
            .filter(|(_, r)| r.family == "GSR")
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        ensure!(ids.len() <= 1, "multiple global GSR resources");
        let mut user_gsr = Input::Constant(Logic::Zero);
        for id in ids {
            let modes = self.gsr_field(id, ".GSRMODE")?;
            ensure!(
                modes.len() == 1,
                "ambiguous global user GSR mode: {modes:?}"
            );
            let mode = &modes[0];
            self.report.system.user_gsr_resource =
                Some(self.device.model().resource(id).key.clone());
            self.report.system.user_gsr_mode = Some(mode.clone());
            let sync = self.gsr_field(id, ".SYNCMODE")?;
            let synchronous = sync.as_slice() == ["SYNC"];
            ensure!(
                synchronous || sync.iter().all(|s| ["ASYNC", "NONE"].contains(&s.as_str())),
                "unsupported or ambiguous global user GSR synchronization: {sync:?}"
            );
            self.report.system.sync_candidates = sync;
            ensure!(
                !synchronous || mode == "ACTIVE_LOW",
                "synchronous global user GSR requires HY01-measured ACTIVE_LOW polarity, got {mode}"
            );
            if mode != "NONE" {
                ensure!(
                    ["ACTIVE_LOW", "ACTIVE_HIGH"].contains(&mode.as_str()),
                    "unsupported user GSR mode {mode}"
                );
                let wire = self.resource_port(id, "GSR")?;
                user_gsr = self.input(&wire)?;
                if mode == "ACTIVE_LOW" {
                    user_gsr = Input::Inverted(self.signal(&wire)?);
                }
                self.report
                    .roots
                    .push(format!("{}:GSR", self.device.model().resource(id).key));
            }
            if synchronous {
                let key = self.device.model().resource(id).key.clone();
                let wire = self
                    .resource_port(id, "CLK")
                    .context("synchronous global user GSR has no CLK pin")?;
                ensure!(
                    self.gsr_clock_routed(&wire, &mut HashSet::new())?,
                    "synchronous global user GSR has no CLK route: {key}"
                );
                let clock = self.input(&wire)?;
                // HY01: plain stages, no asynchronous preset and no local or
                // global reset. hy01b/c establish their configuration state 11.
                let s1 = self.builder.internal_flip_flop(
                    &format!("sgsr:{key}:s1"),
                    user_gsr,
                    clock,
                    Logic::One,
                )?;
                let s2 = self.builder.internal_flip_flop(
                    &format!("sgsr:{key}:s2"),
                    s1.into(),
                    clock,
                    Logic::One,
                )?;
                let reset = self.builder.signal(Wire {
                    location: self.device.model().resource(id).location,
                    name: "$SGSR.reset".into(),
                })?;
                self.builder.mux(
                    &format!("sgsr:{key}:raw-or-s2"),
                    user_gsr,
                    s2.into(),
                    Input::Constant(Logic::One),
                    reset,
                )?;
                user_gsr = reset.into();
                self.report.roots.push(format!("{key}:CLK"));
                self.record_resource(id, "HY01 and hy01b/c: ACTIVE_LOW SYNC user GSR = raw active OR s2; two rising-CLK stages initially 11 with no resets or asynchronous preset; asynchronous distribution to GSR-enabled resources; propagation delay is zero, captures within 20 ns of reset transitions are timing hazards");
            } else {
                self.record_resource(id,"GSRMODE polarity; ASYNC/NONE share no-sync pattern; selected global input is an execution root");
            }
        }
        self.report.system.flip_flop_initial = (match startup {
            Startup::Configured => "REGSET at configuration regardless of SLICE GSR: observed and repeated on a ULX3S over JTAG SRAM loads (HS01, docs/inventory/LFE5U-85F/hs01_init_slice); inputs settling after wake-up are not modelled",
            Startup::ReadyUnknown | Startup::WakePhases => "X until a local reset, GSR or capture defines it",
        }).into();
        let wake = match startup {
            Startup::ReadyUnknown | Startup::Configured => None,
            Startup::WakePhases => {
                let primary = self.tile_of_kind("EFB0_PICB0")?;
                let secondary = self.tile_of_kind("EFB2_PICB0")?;
                let sequence = self.one(primary, "SYSCONFIG.WAKE_UP")?;
                let done = self.one(primary, "SYSCONFIG.DONE_EX")?;
                ensure!(
                    done == self.one(secondary, "SYSCONFIG.DONE_EX")?,
                    "inconsistent DONE_EX fragments"
                );
                let done_phase = self.one(primary, "SYSCONFIG.DONEPHASE")?;
                for (field, expected) in
                    [("GOEPHASE", "T1"), ("GSRPHASE", "T2"), ("GWEPHASE", "T2")]
                {
                    ensure!(
                        self.one(primary, &format!("SYSCONFIG.{field}"))? == expected,
                        "custom wake phase {field} is unsupported"
                    );
                }
                Some(match (sequence.as_str(),done.as_str(),done_phase.as_str()) {
                    ("4","ON","T0")=>WakeSequence::Sequence4,
                    ("21","OFF","T3")=>WakeSequence::Sequence21,
                    _=>bail!("unsupported wake sequence/DONE_EX/DONEPHASE combination: {sequence}/{done}/{done_phase}"),
                })
            }
        };
        self.report.system.wake_sequence = wake;
        self.report.system.rule="logical strobes settle before FF/RAM capture; GSR resets enabled slice FF and EBR read state, preserving arrays; GOE gates output driver; GWDIS inhibits RAM writes; wake sys:step is an explicit phase boundary, not physical START; sequence4 external DONE holds completion at T3; PUR unmodelled".into();
        self.system = Some(
            self.builder
                .system_controls(SystemConfig { wake, user_gsr })?,
        );
        Ok(())
    }
}
