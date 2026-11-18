# Internal oscillator execution (OSCG)

Added on 2026-10-29. The emulator executes the internal oscillator `OSCG` as a free-running clock.
The model is [kernel/osc.rs](../src/kernel/osc.rs), the planner part
[compile/oscillator.rs](../src/compile/oscillator.rs). The behaviour comes from the board
measurement [HC06](inventory/LFE5U-85F/hc06_osc/README.md).

## Behaviour

| Aspect | Model | Basis |
|---|---|---|
| Frequency | base frequency / divider | HC06: the dividers are exact |
| Base frequency | given by the board profile (`Board::oscillator_hz`, CLI `--osc-hz`); default the nominal 310 MHz | the base belongs to the chip: HC06 measured 311.8–311.97 MHz (+0.6%) on one ULX3S |
| Divider | measured for each `OSC.DIV` bit pattern ([table](#divider-table)) | HC06 sweep of every pattern the Trellis database can write |
| Start | running from time 0: low, rising after half a period, 50% duty | HC06: running before user logic starts (`E` = 2–13 reference cycles), at an arbitrary phase; duty not measured |
| Stability | constant | HC06: ±0.1% wander between 2.6 ms windows (σ ≈ 0.03%), ~0.05% between loads; not modelled |

The generator keeps one future edge in the event queue and schedules the next one when it fires.
A long `run_until` does not fill the queue.

## Divider encoding

nextpnr writes `OSC.DIV` as an enum in the `EFB0_PICB0` and `EFB1_PICB1` tiles. The Trellis
database encodes the 126 values (2…127) in only six bits:
- in `EFB0`, two bits that follow (DIV−1) mod 4;
- in `EFB1`, four more bits.

So above 31 several values share one bit pattern: 63 patterns in all. Values 2–31 are unique;
from 32 they come in groups (32–33, 34–35, …, 64–66, 67–70, …, 123–126), and 127 stands alone.
nextpnr also writes `DIV=128`, the primitive's default, as `127`.

Consequences on the board:
- `DIV=33` gives ÷32;
- `DIV=127` and `DIV=128` give ÷128;
- the chip applies one fixed divider per pattern, which the database does not tell.

The planner therefore takes the candidates of the decoded `OSC.DIV` field, which name the bit
pattern, and looks up the divider measured for that pattern. A pattern without a measured divider
is rejected.

## Divider table

`OSCILLATOR_DIVIDERS` in [compile/oscillator.rs](../src/compile/oscillator.rs) has one entry per
bit pattern, measured in the HC06 sweep. `tests/oscillator_dividers.rs` derives the table from
[sweep.json](inventory/LFE5U-85F/hc06_osc/sweep.json) and checks that it covers 2…127 exactly
once.

| `DIV` values sharing a pattern | Divider |
|---|---|
| 2, 3, …, 31 (one each) | the value |
| 32–33, 34–35, …, 62–63 | 32, 34, …, 62 |
| 64–66 | 64 |
| 67–70, 71–74, …, 123–126 | 68, 72, …, 124 |
| 127 (also nextpnr's 128) | 128 |

The chip's divider is exact up to 31, in steps of 2 up to 64 and in steps of 4 up to 128.

## Checks

- `tests/event_kernel.rs`, `oscillator_runs_free_with_exact_edges_and_one_queued_event`: exact edge
  times on a rational period, and 300 000 edges in a long run.
- `tests/oscillator_native.rs` (`--ignored`) runs two HC06 configurations that ran on the board,
  `DIV=16` and `DIV=127`, with the board's base frequency. The reported `F` must be within 0.1% of
  every load's mean on the board. `S` and the start-up `E` must lie within what the board showed.
  The nominal profile must give 310 MHz over the divider.

## Open

- the duty cycle, in particular for odd dividers;
- `SEDSTDBY` and the oscillator's other uses (SED, configuration clock);
- the frequency spread over chips and temperature (one board, room temperature).
