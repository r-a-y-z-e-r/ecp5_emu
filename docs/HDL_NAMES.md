# HDL names (nextpnr `--write`)

Added on 2026-10-29. Probes, stimuli and waveforms can use the design's own signal names
instead of chip coordinates. The implementation is [names.rs](../src/names.rs).

## Use

Write the routed design next to the configuration, then pass it to `emulate`:

```sh
nextpnr-ecp5 --85k --package CABGA381 --json top.json --lpf top.lpf \
  --textcfg top.config --write top_routed.json
cargo run --release --locked -- emulate top.config --names top_routed.json \
  --startup configured --scope observed --clock net:clk_25mhz --cycles 16 \
  --probe net:count --probe net:led --vcd run.vcd
```

- `net:NAME` works wherever a reference is accepted: `--probe`, `--set`, `--clock` and stimulus
  files.
- A bus name without an index (`net:count`) expands to all its bits in a probe; `--set` and
  `--clock` need a single bit (`net:count[3]`).
- The printed samples and the VCD variables carry the HDL names.

## How a name resolves

nextpnr writes one entry per bit in `netnames`, with a `ROUTING` attribute made of triples
"wire; pip; strength":
- **A net** resolves to its source wire, the triple without a pip, for example
  `X2/Y24/Q0_SLICE` → `R24C2:Q0_SLICE`. The emulator traces that wire like any other probe, so the
  value is the one the configured chip has there.
- **A global clock net** also lists its branch taps (`G_HPBX…`) without pips. Its source is the
  DCC output.
- **A top-level port** resolves to the package pin of its `TRELLIS_IO` cell, for example
  `led[0]` → `pin:B2`.
- **A net without a routed source** cannot be probed by name: a constant, or a net inside a cell
  such as a carry between the two halves of a CCU2. The error says so.

The names come from the tools, so they are as good as the synthesis keeps them: Yosys renames or
removes nets it optimises away. The routed JSON must belong to the same nextpnr run as the
configuration; nothing checks this yet.

## Checks

- `names.rs` unit test: source selection, global nets, constants.
- `tests/names_native.rs` (`--ignored`) builds a counter with nextpnr `--write`. It checks that
  `net:count` and the port `led` follow the counter on every cycle, and that the ports resolve to
  the LPF balls.
