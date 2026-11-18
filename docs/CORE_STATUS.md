# First milestone of the main code

The continuation is already implemented: [configuration image and separate state](CONFIGURATION_STATE.md), [event kernel](EVENT_KERNEL.md), [automatic execution](AUTOMATIC_EXECUTION.md), [LUTRAM/RAMW](LUTRAM_EXECUTION.md), then [global controls and explicit wake phases](SYSTEM_EXECUTION.md). The limits and the 25 checks described below belong to the first milestone; the current information is given in the continuations.

Implementation began on October 22, 2026, after the initial collection of materials. A separate Rust layer `DeviceModel` was added. It loads the permanent architecture of the LFE5U-85F independently of the user configuration. Event execution and the merging of the new family decoders remain the next steps.

## What works

| Model content | Verified volume |
|---|---:|
| Tiles / types | 14 231 / 148 |
| Resources with permanent readable keys | 53 690 |
| Source site references | 54 721 |
| Bound to an instance / group | 47 372 / 5 733 |
| Candidate / unbound | 120 / 1 496 |
| Internal PLC COMB/FF/RAMW representations | 177 735 |
| Interface records of resources and PLCs, including alternative representations | 3 246 331 (3 246 243 + 88 EHXPLLL ports added 2026-10-29) |
| Route descriptions by type | 69 287 |
| The same descriptions over all tile placements | 28 013 216 |
| All balls of the four packages / GPIO links | 1 976 / 947 |

The 28 million is the number of source descriptions repeated per tile, including inapplicable density prefixes and boundary endpoints. It is not the number of proven physical conductors or active connections. A template is stored once per tile type, the instances of the connections are resolved on request; disabled variants stay in the model.

Preserved are the CRAM windows, the original vendor site positions separately from the fabric coordinates, the PLC/SLICE and DSP hierarchy, the port directions, the kind and the source line of a route, and the polarities/local bits of its selection. Every port is bound to a coordinate wire name. The internal PLC templates give the COMB/FF/RAMW interfaces, each with a SLICE owner. These are representations of the structure; a separate executable storage has not been allocated yet, and the coarse/split interfaces are not counted as additional physical FFs.

All available resources are present regardless of whether the circuit uses them. CIBTEST and other unparsed references keep their source data and status; candidates do not become confirmed. Dedicated, power/ground and NC pins stay next to the GPIO; the absence of a PIO does not remove the ball.

## API and data

The main types are in [src/model](../src/model/mod.rs): `DeviceModel`, `TileId`, `ResourceId`, `NodeId`, `SiteReference`, `Port`, `PackagePin`, `Connection` and the coordinate `Wire`. After loading, access to the model is immutable. Readable keys are used for external references; dense handles belong to the specific loaded model and are not meant to be saved between data versions.

The ordinary globalization of names repeats Trellis. For G_HPBX/VPTX/HPRX, the relation to the physical segment is returned separately. The original name itself is preserved; computing the segment does not merge nets and does not enable a TAP/SPINE buffer. `connections(tile)` returns the directed descriptions of all fixed/mux, including the variants with `OtherDensity`/`OutsideDevice`. `cram_constraints()` carries the selection bits into the shared CRAM. An empty bit group does not declare a path active. The conditions of the mode bypasses, including ECLKSYNC, are not executed yet.

The loader checks [model_manifest.json](inventory/LFE5U-85F/model_manifest.json): eight researched input tables/catalogs and all 152 pinned database files. Diverging hashes, unknown/repeated references, missing sites, hierarchy cycles and bits/ports out of bounds cause an error. The match of the source site tuples and of all package GPIO with the current database is checked. The Trellis/database versions and the SHA-256 of the manifest are part of the summary.

```sh
# After a deliberate change of the source architecture tables:
python3 tools/build_model_manifest.py

# Load the permanent model without .bit/.config:
cargo run --release --locked -- device --output target/device-model-summary.json
```

The path of the tables can be changed with `--architecture PATH`, that of the database with `--db PATH`/`ECP5_DB`. Loading the new layer itself needs neither Python nor libtrellis: Rust reads the pinned tables and the database. Generating the manifest is a separate data preparation step. The recorded [model summary](inventory/LFE5U-85F/core_model_summary.json) does not replace the full structure in memory.

## Checks

```sh
cargo test --release --locked
cargo test --release --locked --test device_model -- --ignored
cargo test --release --locked --test whole_chip routed_counter_counts_through_tile_boundaries -- --ignored
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

18 earlier unit tests, five new portable checks and two explicitly run integration checks passed. The new checks distinguish the direction and alternatives of the mux, tiles with equal row/col, dedicated pads, raw/segment clocks, corruption of the sources, lost sites, wrong vendor coordinates, cycles and an escape from the manifest root. The full integration checked the capacities/interfaces and **all 197 408 clock-segment relations** against the saved native study. Separately, the old counter passed 65 538 observations with transitions between tiles. Clippy passed without warnings.

The remaining conditional integration tests of reading `.bit` were not rerun at this milestone; their earlier check is described in [TOOLCHAIN.md](TOOLCHAIN.md). There was no new hardware run.

## Milestone boundary and next step

`device` uses the new structural layer. `run` still uses the earlier prototype. The new model does not yet replace its simulation, does not execute DSP/EBR/PLL and does not fix the loss of service registers in the earlier `.bit`→`.config` bridge. All 78 research topics remain open. The internals of DSP/IO/DQS have not yet got a full Rust graph; banks are so far represented by the source BANKREF tiles/fields, not by a separate behavioral API.

The next step per the [plan](IMPLEMENTATION_PLAN.md): bind the full configuration snapshot to this architecture, keeping the ambiguous fields and the service data, then a separate `DeviceState` and the event kernel. After that, execute LUT/FF/carry/PIO through the directed paths of the same model. The merging of the LUTRAM/FF/DSP states, the mode activity and the unsupported active functions must stay explicit, without turning a fixed-description into an unconditional electrical wire.
