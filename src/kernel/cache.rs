//! Construction-time snapshots for the plain PLC/routing/system kernel.
//! Timed blocks and memory/DSP/IOLOGIC controllers deliberately fall back to
//! compilation until their complete construction state has a cache schema.
use super::*;
use crate::{
    compile::{Compiled, Report},
    model::{
        topology::{Location, Wire},
        DeviceModel,
    },
};
use std::{borrow::Cow, cell::RefCell};

#[derive(Default)]
struct Identity {
    scope: u64,
    nodes: Vec<NodeId>,
    node_ids: HashMap<NodeId, usize>,
    #[cfg(test)]
    resource_ids: HashMap<ResourceId, usize>,
}
thread_local! { static IDENTITY: RefCell<Identity> = RefCell::default(); }
struct IdentityGuard;
impl IdentityGuard {
    fn new(model: &DeviceModel, scope: u64) -> Self {
        let nodes: Vec<_> = model
            .resources()
            .filter(|(_, r)| r.family == "PLC2")
            .flat_map(|(id, _)| model.nodes(id).map(|n| n.id))
            .collect();
        let node_ids = nodes.iter().enumerate().map(|(i, &n)| (n, i)).collect();
        IDENTITY.with(|c| {
            *c.borrow_mut() = Identity {
                scope,
                nodes,
                node_ids,
                #[cfg(test)]
                resource_ids: model
                    .resources()
                    .enumerate()
                    .map(|(i, (id, _))| (id, i))
                    .collect(),
            }
        });
        Self
    }
}

/// Complete deterministic test snapshots, independent of cache eligibility.
/// Private block/storage types opt into serialization only in test builds.
#[cfg(test)]
pub(crate) mod snapshot {
    use super::*;
    use serde::Serializer;
    use std::hash::Hash;

    impl Serialize for ResourceId {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            IDENTITY
                .with(|c| c.borrow().resource_ids.get(self).copied())
                .ok_or_else(|| serde::ser::Error::custom("resource outside snapshot architecture"))?
                .serialize(s)
        }
    }
    impl Serialize for NodeId {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            node::serialize(self, s)
        }
    }
    impl Serialize for TimerId {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.index.serialize(s)
        }
    }
    // Sort by serialized identity: hash seeds, addresses and kernel scopes
    // must never affect a comparison. Vector order is preserved everywhere.
    pub(crate) fn map<K: Serialize + Eq + Hash, V: Serialize, S: Serializer>(
        values: &HashMap<K, V>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut entries = values
            .iter()
            .map(|(k, v)| {
                postcard::to_stdvec(k)
                    .map(|key| (key, v))
                    .map_err(serde::ser::Error::custom)
            })
            .collect::<Result<Vec<_>, S::Error>>()?;
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.serialize(s)
    }
    pub(crate) fn set<K: Serialize + Eq + Hash, S: Serializer>(
        values: &HashSet<K>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut entries = values
            .iter()
            .map(postcard::to_stdvec)
            .collect::<Result<Vec<_>, _>>()
            .map_err(serde::ser::Error::custom)?;
        entries.sort();
        entries.serialize(s)
    }
    pub(crate) fn array<T: Serialize, S: Serializer, const N: usize>(
        value: &[T; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_slice().serialize(s)
    }
    pub(crate) fn optional_array<T: Serialize, S: Serializer, const N: usize>(
        value: &Option<[T; N]>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        value.as_ref().map(|v| v.as_slice()).serialize(s)
    }
    pub(crate) fn encode(compiled: &Compiled) -> Result<Vec<u8>> {
        let k = &compiled.kernel;
        let _identity = IdentityGuard::new(k.device.model(), k.scope);
        // The production cache deliberately skips these temporal hazard fields.
        let register_times: Vec<_> = k
            .registers
            .iter()
            .map(|r| (r.preload_changed_at, r.released_at))
            .collect();
        Ok(postcard::to_stdvec(&(
            "ecp5-full-kernel-snapshot-v1",
            k.device.fingerprint(),
            k,
            register_times,
            &compiled.drivers,
            &compiled.probes,
            &compiled.report,
        ))?)
    }
}
impl Drop for IdentityGuard {
    fn drop(&mut self) {
        IDENTITY.with(|c| *c.borrow_mut() = Identity::default());
    }
}

pub(super) mod node {
    use super::*;
    pub fn serialize<S: serde::Serializer>(id: &NodeId, s: S) -> Result<S::Ok, S::Error> {
        IDENTITY
            .with(|c| c.borrow().node_ids.get(id).copied())
            .ok_or_else(|| serde::ser::Error::custom("node outside cache architecture"))?
            .serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<NodeId, D::Error> {
        let index = usize::deserialize(d)?;
        IDENTITY
            .with(|c| c.borrow().nodes.get(index).copied())
            .ok_or_else(|| serde::de::Error::custom("invalid cached node"))
    }
}

// Internal resource FFs have no physical PLC node. Their values live in the
// ordinary register drivers; they never claim physical DeviceState storage.
pub(super) mod optional_node {
    use super::*;
    pub fn serialize<S: serde::Serializer>(id: &Option<NodeId>, s: S) -> Result<S::Ok, S::Error> {
        let index = id
            .map(|id| {
                IDENTITY
                    .with(|c| c.borrow().node_ids.get(&id).copied())
                    .ok_or_else(|| serde::ser::Error::custom("node outside cache architecture"))
            })
            .transpose()?;
        index.serialize(s)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<NodeId>, D::Error> {
        Option::<usize>::deserialize(d)?
            .map(|index| {
                IDENTITY
                    .with(|c| c.borrow().nodes.get(index).copied())
                    .ok_or_else(|| serde::de::Error::custom("invalid cached node"))
            })
            .transpose()
    }
}

// Dense handles in a file contain only their index. Every read gets a fresh
// kernel scope, including the handles exported in Compiled::drivers/probes.
macro_rules! cached_handle {
    ($kind:ident) => {
        impl Serialize for $kind {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.index.serialize(s)
            }
        }
        impl<'de> Deserialize<'de> for $kind {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Ok(Self {
                    index: usize::deserialize(d)?,
                    scope: IDENTITY.with(|c| c.borrow().scope),
                })
            }
        }
    };
}
cached_handle!(SignalId);
cached_handle!(DriverId);

// Keep the model and logic modules independent of the compiled-plan format.
impl<'de> Deserialize<'de> for Logic {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "lowercase")]
        enum Value {
            Zero,
            One,
            X,
            Z,
        }
        Ok(match Value::deserialize(d)? {
            Value::Zero => Self::Zero,
            Value::One => Self::One,
            Value::X => Self::X,
            Value::Z => Self::Z,
        })
    }
}
impl<'de> Deserialize<'de> for Location {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Value {
            row: i16,
            col: i16,
        }
        let v = Value::deserialize(d)?;
        Ok(Self {
            row: v.row,
            col: v.col,
        })
    }
}
impl<'de> Deserialize<'de> for Wire {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Value {
            location: Location,
            name: String,
        }
        let v = Value::deserialize(d)?;
        Ok(Self {
            location: v.location,
            name: v.name,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct RegisterValue {
    #[serde(with = "node")]
    node: NodeId,
    value: Logic,
}
#[derive(Clone, Serialize, Deserialize)]
struct SystemValue {
    phase: Option<u8>,
    values: [Logic; 7],
    wake: bool,
}
#[derive(Serialize, Deserialize)]
struct Core<'a> {
    signals: Cow<'a, [Signal]>,
    drivers: Cow<'a, [Driver]>,
    combinational: Cow<'a, [Combinational]>,
    registers: Cow<'a, [Register]>,
    aliases: Cow<'a, [Vec<usize>]>,
    clock_groups: Cow<'a, [ClockGroup]>,
    system: Cow<'a, Option<system::Controller>>,
    signal_rank: Cow<'a, [usize]>,
    driver_rank: Cow<'a, [usize]>,
    stats: Cow<'a, Statistics>,
    recent_activity: VecDeque<(usize, Logic)>,
    delta: u32,
    register_values: Vec<RegisterValue>,
    system_value: Option<SystemValue>,
    drivers_by_name: Cow<'a, BTreeMap<String, DriverId>>,
    probes_by_name: Cow<'a, BTreeMap<String, SignalId>>,
    report: Cow<'a, Report>,
}

pub(crate) fn encode(compiled: &Compiled) -> Option<Vec<u8>> {
    let k = &compiled.kernel;
    if !k.lut_writes.is_empty()
        || !k.ebr.is_empty()
        || !k.multipliers.is_empty()
        || !k.alus.is_empty()
        || !k.plls.is_empty()
        || !k.iologic.is_empty()
        || !k.dcs.is_empty()
        || !k.dtrs.is_empty()
        || !k.oscillators.is_empty()
        || !k.queue.is_empty()
        || !k.timers.is_empty()
        || !k.trace.is_empty()
        || k.now != Time::ZERO
        || k.fault.is_some()
    {
        return None;
    }
    let _identity = IdentityGuard::new(k.device.model(), k.scope);
    let core = Core {
        signals: Cow::Borrowed(&k.signals),
        drivers: Cow::Borrowed(&k.drivers),
        combinational: Cow::Borrowed(&k.combinational),
        registers: Cow::Borrowed(&k.registers),
        aliases: Cow::Borrowed(&k.aliases),
        clock_groups: Cow::Borrowed(&k.clock_groups),
        system: Cow::Borrowed(&k.system),
        signal_rank: Cow::Borrowed(&k.signal_rank),
        driver_rank: Cow::Borrowed(&k.driver_rank),
        stats: Cow::Borrowed(&k.stats),
        recent_activity: k.recent_activity.clone(),
        delta: k.delta,
        register_values: k
            .registers
            .iter()
            .filter(|r| r.config.node.is_some())
            .map(|r| {
                let node = r.config.node?;
                Some(RegisterValue {
                    node,
                    value: k.state.register(node).ok()?,
                })
            })
            .collect::<Option<_>>()?,
        system_value: k.state.system().map(|s| SystemValue {
            phase: s.phase,
            wake: s.profile == "wake-phases",
            values: [
                s.goe,
                s.gwdis,
                s.startup_gsr,
                s.user_gsr,
                s.gsr,
                s.done,
                s.ready,
            ],
        }),
        drivers_by_name: Cow::Borrowed(&compiled.drivers),
        probes_by_name: Cow::Borrowed(&compiled.probes),
        report: Cow::Borrowed(&compiled.report),
    };
    postcard::to_stdvec(&core).ok()
}

pub(crate) fn decode(
    device: Arc<ConfiguredDevice>,
    options: Options,
    bytes: &[u8],
) -> Result<Compiled> {
    #[cfg(feature = "compile-timing")]
    let state_timer = crate::prepared::load_timing::enter("kernel.fresh_state", 0);
    let mut k = Builder::new(device)?.build()?;
    #[cfg(feature = "compile-timing")]
    drop(state_timer);
    k.options = options;
    #[cfg(feature = "compile-timing")]
    let identity_timer = crate::prepared::load_timing::enter("kernel.identity", 0);
    let _identity = IdentityGuard::new(k.device.model(), k.scope);
    #[cfg(feature = "compile-timing")]
    drop(identity_timer);
    #[cfg(feature = "compile-timing")]
    let deserialize_timer =
        crate::prepared::load_timing::enter("kernel.deserialize", bytes.len() as u64);
    let (core, rest): (Core<'_>, _) = postcard::take_from_bytes(bytes)?;
    #[cfg(feature = "compile-timing")]
    drop(deserialize_timer);
    #[cfg(feature = "compile-timing")]
    let _restore_timer = crate::prepared::load_timing::enter("kernel.restore", 0);
    ensure!(rest.is_empty(), "trailing plan payload");
    k.signals = core.signals.into_owned();
    k.drivers = core.drivers.into_owned();
    k.combinational = core.combinational.into_owned();
    k.registers = core.registers.into_owned();
    k.aliases = core.aliases.into_owned();
    k.clock_groups = core.clock_groups.into_owned();
    k.system = core.system.into_owned();
    k.signal_rank = core.signal_rank.into_owned();
    k.driver_rank = core.driver_rank.into_owned();
    k.stats = core.stats.into_owned();
    k.recent_activity = core.recent_activity;
    k.delta = core.delta;
    k.register_seen = vec![false; k.registers.len()];
    k.signal_ids.reserve(k.signals.len());
    for (index, s) in k.signals.iter().enumerate() {
        ensure!(
            k.state.register_wire(s.wire.clone(), s.value)? == index,
            "duplicate cached wire"
        );
        k.signal_ids.insert(
            s.wire.clone(),
            SignalId {
                scope: k.scope,
                index,
            },
        );
    }
    for r in core.register_values {
        k.state.write_register(r.node, r.value)?;
    }
    if let Some(s) = core.system_value {
        let [goe, gwdis, startup_gsr, user_gsr, gsr, done, ready] = s.values;
        k.state.set_system(crate::state::SystemState {
            profile: if s.wake {
                "wake-phases"
            } else {
                "ready-unknown"
            },
            phase: s.phase,
            goe,
            gwdis,
            startup_gsr,
            user_gsr,
            gsr,
            done,
            ready,
            power_on_reset: "unmodelled; image already loaded",
        });
    }
    Ok(Compiled {
        kernel: k,
        drivers: core.drivers_by_name.into_owned(),
        probes: core.probes_by_name.into_owned(),
        report: core.report.into_owned(),
    })
}
