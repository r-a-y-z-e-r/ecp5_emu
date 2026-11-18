//! Pure route work, prepared by tile; reports and builder calls stay in walk order.
use super::*;
use std::ops::Range;

type DecodedWire = (WireId, Result<Vec<Incoming>>);

type Candidate = (Option<Wire>, RouteUse, Vec<BitConstraint>);

pub(super) struct Connection {
    pub kind: ArcKind,
    pub sink: Wire,
    pub candidate: Candidate,
}

pub(super) struct Incoming {
    pub item: index::Incoming,
    pub constant: Option<Result<Vec<String>>>,
    pub carry: Option<Result<Vec<String>>>,
    pub connections: Option<Vec<Result<Option<Connection>>>>,
}
impl Incoming {
    pub fn lazy(item: index::Incoming) -> Self {
        Self {
            item,
            constant: None,
            carry: None,
            connections: None,
        }
    }
}

// Bound preparation, live memory and time to the next FIFO prefix. In
// particular, do not decode the rest of the chip for a small observed fan-in.
pub(super) const BATCH: usize = 16_384;
// Smaller batches keep lazy decoding; scoped-worker setup and tile grouping
// must be amortized over useful work. The override also prepares small batches
// so the same paths can be checked with ECP5_COMPILE_THREADS=1 and =2.
const MIN_BATCH: usize = 256;

pub(super) struct Decoded {
    wires: Vec<Option<Result<Vec<Incoming>>>>,
    threads: usize,
    forced: bool,
}
impl Default for Decoded {
    fn default() -> Self {
        let forced = std::env::var("ECP5_COMPILE_THREADS")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|&n| n > 0);
        Self {
            wires: Vec::new(),
            threads: forced.unwrap_or_else(crate::fast::performance_cores),
            forced: forced.is_some(),
        }
    }
}
impl Decoded {
    pub fn take(&mut self, wire: WireId) -> Option<Result<Vec<Incoming>>> {
        self.wires.get_mut(wire.0)?.take()
    }

    pub fn prepare(
        &mut self,
        device: &ConfiguredDevice,
        index: &index::Index,
        queue: &VecDeque<WireId>,
        seen: &[bool],
        producers: &HashMap<WireId, Vec<Producer>>,
    ) {
        if !self.forced && queue.len().min(BATCH) < MIN_BATCH {
            return;
        }
        let parts = index.with_lookup(|lookup| {
            let mut unique = HashSet::new();
            let mut wires = Vec::new();
            for &id in queue.iter().take(BATCH) {
                if !seen.get(id.0).copied().unwrap_or(false)
                    && !producers.contains_key(&id)
                    && unique.insert(id)
                {
                    wires.push(id);
                }
            }
            if unique.is_empty() || (!self.forced && unique.len() < MIN_BATCH) {
                return Vec::new();
            }
            let mut tiles = BTreeMap::<usize, Vec<WireId>>::new();
            for id in wires {
                tiles.entry(lookup.tile_order(id)).or_default().push(id);
            }
            let tiles: Vec<_> = tiles.into_values().collect();
            ranges(tiles.len(), self.threads, |range| {
                tiles[range]
                    .iter()
                    .flatten()
                    .map(|&id| (id, decode_wire(device, &lookup, id)))
                    .collect::<Vec<_>>()
            })
        });
        self.store(parts);
    }
    fn store(&mut self, parts: Vec<Vec<DecodedWire>>) {
        for (id, result) in parts.into_iter().flatten() {
            self.wires
                .resize_with(self.wires.len().max(id.0 + 1), || None);
            self.wires[id.0] = Some(result);
        }
    }
}

fn ranges<T: Send>(len: usize, threads: usize, work: impl Fn(Range<usize>) -> T + Sync) -> Vec<T> {
    let workers = threads.min(len.div_ceil(64)).max(1);
    if workers == 1 {
        return vec![work(0..len)];
    }
    std::thread::scope(|scope| {
        let work = &work;
        let chunk = len.div_ceil(workers);
        let handles: Vec<_> = (0..len)
            .step_by(chunk)
            .map(|start| scope.spawn(move || work(start..(start + chunk).min(len))))
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

fn decode_wire(
    device: &ConfiguredDevice,
    lookup: &index::Lookup<'_>,
    id: WireId,
) -> Result<Vec<Incoming>> {
    Ok(lookup
        .incoming(device.model(), id)?
        .into_iter()
        .map(|item| {
            let constant = item
                .constant
                .as_ref()
                .map(|field| device.enum_candidates(item.tile, field));
            let carry = carry_field(device, item.tile, &item.raw)
                .map(|field| device.enum_candidates(item.tile, &field));
            let override_carry = carry
                .as_ref()
                .is_some_and(|v| v.as_ref().is_ok_and(|v| v.as_slice() == ["CCU2"]));
            let connections = item
                .constant
                .is_none()
                .then(|| connections(device, item.tile, &item.raw, override_carry));
            Incoming {
                item,
                constant,
                carry,
                connections,
            }
        })
        .collect())
}

pub(super) fn carry_field(device: &ConfiguredDevice, tile: TileId, raw: &str) -> Option<String> {
    (device.model().tile(tile).kind == "PLC2"
        && raw.len() == 2
        && raw.starts_with('F')
        && (b'0'..=b'7').contains(&raw.as_bytes()[1]))
    .then(|| {
        let slice = (b'A' + (raw.as_bytes()[1] - b'0') / 2) as char;
        format!("SLICE{slice}.MODE")
    })
}

pub(super) fn connections(
    device: &ConfiguredDevice,
    tile: TileId,
    raw: &str,
    override_carry: bool,
) -> Vec<Result<Option<Connection>>> {
    let source = override_carry.then(|| format!("{raw}_SLICE"));
    device
        .matching_connections_to(tile, raw, source.as_deref())
        .map(|connection| {
            let c = connection?;
            let source = match c.source {
                Endpoint::OtherDensity => return Ok(None),
                Endpoint::Wire(ref wire) => Some(wire.clone()),
                Endpoint::OutsideDevice => None,
            };
            if c.definition.kind == ArcKind::Fixed && source.is_none() {
                return Ok(None);
            }
            let Endpoint::Wire(ref sink) = c.sink else {
                return Ok(None);
            };
            Ok(Some(Connection {
                kind: c.definition.kind,
                sink: sink.clone(),
                candidate: (
                    source,
                    RouteUse {
                        tile: c.tile.name.clone(),
                        source: c.definition.source.clone(),
                        sink: c.definition.sink.clone(),
                        source_line: c.definition.source_line,
                        rule: if override_carry {
                            "CCU2 sum; overlapping INJECT/F bits"
                        } else if c.definition.kind == ArcKind::Fixed {
                            "fixed; qualified reachable path"
                        } else {
                            "maximal matching mux pattern"
                        }
                        .into(),
                    },
                    c.cram_constraints().collect(),
                ),
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_ranges_join_in_input_order_and_have_a_single_thread_fallback() {
        let main = std::thread::current().id();
        let one = ranges(130, 1, |range| (range, std::thread::current().id()));
        assert_eq!(one, vec![(0..130, main)]);
        let two = ranges(130, 2, |range| (range, std::thread::current().id()));
        assert_eq!(
            two.iter()
                .map(|(range, _)| range.clone())
                .collect::<Vec<_>>(),
            vec![0..65, 65..130]
        );
        assert_ne!(two[0].1, two[1].1);
        assert!(two.iter().all(|(_, thread)| *thread != main));
    }

    #[test]
    fn speculative_errors_are_owned_and_reported_only_in_walk_order() {
        for threads in [1, 2] {
            let parts = ranges(128, threads, |range| {
                range
                    .map(|i| {
                        (
                            WireId(i),
                            Err(anyhow::anyhow!("decode {i}").context(format!("source {i}"))),
                        )
                    })
                    .collect()
            });
            let mut decoded = Decoded {
                wires: Vec::new(),
                threads,
                forced: true,
            };
            // Preparing/joining succeeds even with errors in unvisited wires.
            decoded.store(parts);
            // Tile order puts 0 first; the FIFO walk reaches 97 first.
            let error = decoded.take(WireId(97)).unwrap().err().unwrap();
            assert_eq!(format!("{error:#}"), "source 97: decode 97");
            assert!(decoded.take(WireId(97)).is_none());
            let error = decoded.take(WireId(0)).unwrap().err().unwrap();
            assert_eq!(format!("{error:#}"), "source 0: decode 0");
        }
    }
}
