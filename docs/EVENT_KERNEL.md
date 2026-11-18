# Event-driven execution: the first milestone

The first kernel milestone and its 52 checks are kept below. These were followed by [automatic execution from CRAM and the emulate command](AUTOMATIC_EXECUTION.md), [LUTRAM/RAMW and the shared FF/write capture](LUTRAM_EXECUTION.md), then [global controls and explicit wake phases](SYSTEM_EXECUTION.md) and [EBR execution](EBR_EXECUTION.md).

October 22, 2026. After [configuration loading and state allocation](CONFIGURATION_STATE.md), the Rust module [kernel](../src/kernel/mod.rs) was added. It executes an explicitly specified plan on top of `ConfiguredDevice` and `DeviceState`: LUT4 from the live storage, local FFs, directed digital transfers and muxes. The permanent architecture and all non-executable resources are kept.

A full automatic assembly of an executable circuit from CRAM is not implemented yet. The new module is available through the library API and integration checks; the commands `load` and the old `run` keep their former purpose. A successful local LUT→FF path does not mean support for all active resources of an arbitrary bitstream.

## Time and event order

`Time` is a reduced non-negative fraction of picoseconds with a numerator and a denominator of `u64`. Comparison and intermediate computations use `u128`. A period of 1 000 000/3 ps is not rounded at each edge; 10 000 additions were checked against the exact product. An unrepresentable result returns an error. The picosecond here is a unit of virtual time, not a claim about the accuracy of physical delays.

The scheduler merges equal fractional timestamps. At each time it:

1. Discards the events of stale timer generations and checks that there are no conflicting writes to one driver.
2. Sets all input drivers simultaneously, then resolves the values of the affected wires.
3. Stabilizes the combinational part in waves: all elements of a wave read one snapshot, then publish their outputs together.
4. Captures the sequential elements from one stabilized snapshot. All new Qs are published after the capture completes.
5. Repeats the combinational/sequential waves if clocks, reset or other dependencies changed.

Clocks from the Q of another FF are handled by the next delta wave of the same virtual time. Two independent input clocks with one timestamp capture each other's previous Qs. Permuting the registration order of elements, wire identifiers and input events does not change the checked results and traces.

When the kernel is created, the initial drivers and combinational connections stabilize, then the initial clock levels are set without an artificial edge. Asynchronous reset is applied in this initial state as well. `DeviceState` leaves FFs at X by default; the physical PUR/GSR are not replaced by the value of REGSET.

`build` closes time 0. Initial levels are given to the drivers before it; subsequent stimuli are allowed strictly later than the currently closed time. `run_until` closes the given timestamp, including a gap with no events. Moving backward, or adding a stimulus later into an already processed time, is not allowed.

## Explicit digital rules

The following decisions are the kernel's contract for uncertainty and coincidences, tied to open question S05. They do not close the question of physical races on the board.

| Situation | Result |
|---|---|
| D changes simultaneously with the clock | The FF sees D after the combinational phase of this timestamp stabilizes |
| Clock changes inside the combinational stabilization | Kept in the delta trace; the FF compares the final levels between capture phases, physical hazards are not reproduced |
| Known 0→1 / 1→0 | Ordinary capture on the selected edge, both directions are supported |
| A possible edge through X/Z | Hold and capture are merged: identical results are kept, different ones give X; the event is counted in the statistics |
| An unchanged X on the clock | By itself does not create an edge |
| CE=X/Z | Capture and hold are merged |
| Local sync/async reset | Reset has priority over CE; sync acts on the edge, async without waiting for the clock |
| Reset=X/Z | The asserted/deasserted variants are merged |
| Z on D or on a LUT address | Undefined logic input; the FF does not write Z into Q |
| Unknown LUT address | All compatible rows of the live table are checked; a known common answer is kept |
| Several strong drivers | Z releases the line, identical levels agree, 0/1 or known/X give X |
| Pull drivers | Counted only when all strong drivers give Z; conflicting pulls give X |
| Two different values of one driver at one timestamp | An event-specification error before the wave is published; the insertion order gives no priority |
| Repeated identical events of one driver | A single set, preserving all the direct causes of the change |

Driver strength has two classes so far, `Strong`/`Pull`. This is a digital model of conflicts and pull-ups; electrical IO standards, currents, voltages and analog thresholds are not included. A directed `buffer` carries a digital value, including a released Z; LUT/FF interpret such an input as undefined. There is no reverse connection and no wire merging.

## State, timers and observation

The kernel owns `DeviceState` and provides it for reading. A LUT is read from the same array that is allocated from INIT; an FF writes to its own physical storage. Each LUT/FF keeps a single owner of the stored bits; drivers and wires hold derived output values. Wire changes are reflected in the state. The summary of such a state notes `event_kernel;explicit_execution_plan`.

`Builder::with_state` accepts an initial snapshot with the matching configuration fingerprint and without cached wires. The initial LUT/FF can be set explicitly for an experiment. This is not save/resume: restoring drivers, timers and the queue from an arbitrary snapshot is not available yet. Raw write access to the storage is not exposed externally during execution; the future RAMW/EBR must publish their writes through the kernel phases.

Each timer has a generation. `cancel_timer` changes it, old events are dropped when extracted from the queue; new events of the same timer use the new generation. Ordinary stimuli are not cancelled by this. Automatic periodic OSC/PLL generators and cancellation by their hardware resets are not connected yet.

`SignalId`, `DriverId`, `TimerId` check that they belong to the specific kernel. Physical `NodeId` still belong to their own loaded architecture. A second executor for one LUT or FF is forbidden.

Observation is selected by the same coordinate-based `Wire`s. A change contains the exact time, the delta number, the phase, the old/new value and the direct sources/causes. The trace is ordered by the readable wire identity and bounded by a ring buffer; the number of evictions is available separately. A disabled trace does not change execution. The initial value of an undriven wire can be read as Z; if it did not change, there is no change record.

The delta-wave limit applies to the whole virtual time, including repeated captures and generated clocks. For a loop that does not stabilize, the kernel reports the last 16 source changes with wires and levels. After an execution error the kernel is stopped; the partial state/history stay readable, continuation is forbidden. An incorrect API request rejected before execution does not stop the kernel.

## Link with ECP5 and the verified path

`Builder::lut4(node)` uses the native A/B/C/D/F and the table from `DeviceState`. `Builder::flip_flop` binds Q to a native port and accepts an explicit D/clock/CE/local reset profile. The DI/M selection, the shared CLK/LSR/CE, MODE, PRLD and the global controls are not derived by this API automatically. The local priority of reset over CE corresponds to the collected LS05; this is not a transfer of all assumptions of the open `TRELLIS_FF` to the physical startup.

The native check creates a LOGIC configuration `R11C10:PLC2`, INIT `0xa15c` and an explicit selection `F0←F0_SLICE`, packs a compressed `.bit`, then loads the text and the bitstream independently. In both cases all 14 231 tiles, 83 640 LUT/FF and 208 EBR remain. One explicitly selected path is executed:

`A/B/C/D → SLICEA.K0 → F0_SLICE → F0 mux → DI0 → DI0_SLICE → SLICEA.FF0 → Q0_SLICE`.

Three directed connections are taken from the pinned database with the source line preserved. For the mux, the single raw candidate was checked and the LOGIC context was set. The FF profile in the experiment is specified explicitly: DI, external clock on the native CLK, CE=1, no local reset/PUR/GSR, initial Q=X. All 16 addresses gave the expected F and Q for both formats. The source image did not change after execution. This is a software check of the conversion/execution, without sending to a board.

The [native result with a trace](inventory/LFE5U-85F/kernel_native_execution.json) and the [milestone check summary](inventory/LFE5U-85F/event_kernel_validation.json) are saved.

## Re-running the checks

```sh
cargo test --release --locked
cargo test --release --locked --test event_kernel -- --ignored
cargo test --release --locked --test configuration_state -- --ignored
cargo test --release --locked --test device_model -- --ignored
cargo test --release --locked --test resource_decode -- --ignored
cargo test --release --locked --test whole_chip routed_counter_counts_through_tile_boundaries -- --ignored
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
```

The native checks require a local Trellis, `ecppack` and the built `trellis_image_reader`; the new integration test saves a fresh trace in `target/event-kernel/native_execution.json`. The ordinary set is portable and does not depend on an installed FPGA toolchain.

At this milestone, **52 tests** passed: 18 unit, five portable architectural, eight configuration/state, 16 event kernel, and five explicitly run native/full integration checks. Among them are the earlier counter with 65 538 observations and the full cross-check of 197 408 clock relations. A separate external blink test with `ECP5_TEST_BIT` was not run. Clippy and formatting passed. There are still no hardware results; 78 research questions and 37 future series are not closed by these tests.

## Performance

Measured on 2026-10-29: Apple M3 Max, release build, single run. The fixture is the 8-bit counter
`target/automatic-compile/top.bit` in the observed scope, run with `--clock pin:G2 --cycles 1000000`,
`--set pin:J4=1`, two probes and `--stimulus cli-events.json`. The wall time includes about 5.5 s
of loading the architecture and the image.

| Kernel | Wall time | Excluding load | Clock cycles/s |
|---|---:|---:|---:|
| Before the hot-path rework | 53.3 s | ~47.8 s | ~21k |
| After the hot-path rework | 14.2 s | ~8.7 s | ~115k |

The rework keeps observable behaviour unchanged:
- An update carries its driver and a compact reason. `Cause` strings are built only for traced
  transitions of watched signals.
- Each wave is ordered by ranks precomputed from the wire order and the driver-label order, instead
  of sorting strings.
- Dirty sets and update buffers are reused between waves.
- Wire state is stored densely by slot.

With the old and the new binary, the `emulate` outputs (JSON, stdout, stderr) were byte-identical
on ten stored fixtures: counter observed and all-plc, LUTRAM, MULT18, DSP registers, DSP output, DSP
ALU operands, two EBR fixtures and wake-phases.

### Second pass (2026-10-29, evening)

| Kernel | 1M-cycle counter, wall time | Peak memory |
|---|---:|---:|
| After the hot-path rework | 14.2 s | ~1.4 GB |
| After the second pass | 7.7 s | 465 MB |

Throughput on configurations that ran on the board (`tests/kernel_speed.rs`, reference cycles per
second, loading excluded):

| Configuration | Before | After | Factor |
|---|---:|---:|---:|
| HI02 (418 FF, 439 LUT4) | 10.9k | 90k | ×8 |
| HC05 (PLL at ×4) | 13.4k | 91k | ×7 |
| HI02b (2304-bit recorder) | 1.58k | 18.3k | ×12 |

The HI02b and HI02c native tests went from about 14 minutes each to about a minute.

Changes:
1. **Clock groups.** Flip-flops with the same clock input share one group and one previous clock
   level. A clock change marks the group, not every flip-flop. A change that is no edge for a
   flip-flop (the falling edge for a rising-edge one) costs nothing for it.
2. **Collapsed routing buffers.** A routing segment whose only driver is a buffer of another signal,
   and that no probe watches, follows the root of its buffer chain once the initial settle has made
   them equal. It takes the root's value in the same delta, without a gate evaluation or a wave of
   its own; whatever reads it is marked from the root. A loop of buffers is left as it is.
3. **Stored values from drivers.** A flip-flop's old value is its output driver's value, not a hash
   map lookup.
4. **Loading.** `tilegrid.json` and the other JSON inputs were parsed from unbuffered files, one
   system call per byte. Reading them whole cut `DeviceModel::load` from about 5 s to 0.3 s.
5. **CLI.** `emulate --clock` generates the clock while it runs instead of building the whole
   timeline first, and buffers the sample lines.

**What changed in the outputs.** On the ten stored fixtures, stdout and the observations, state,
plan and image in `--output` are byte-identical. Statistics count less work. Trace transitions are
the same set (time, wire, previous and new value), but they arrive in fewer delta cycles: delta
numbers in traces and EBR notices are smaller, and transitions within one time step can come in
another order. All native tests, including the board replays, pass unchanged.

Known remaining costs:
- The CLI formats a line for every sample; for long runs, use fast mode or a VCD file.
- A design evaluates every flip-flop of a clock group on each active edge, whether or not its
  inputs changed.
- LUT storage is still looked up through a SipHash map.
- Loading a `.bit` runs the native image reader as a separate process (about 0.6 s).

For real-time speed, see [FAST_MODE.md](FAST_MODE.md).

## Next milestone

Assemble a mode-conditioned executable plan from the configuration: the scalar PLC/FF fields, the joint INJECT/F and the F/OFX/carry paths, CLK/LSR/CE, then CCU2 and a basic PIO. Introduce an explicit system profile of a ready configuration with GSR/PUR/GOE/GWDIS instead of guesses about startup. After that, the first automatic run of a real bitstream with two external clocks, a carry counter, reset and GPIO on the new kernel. PRLD/LUTRAM, the other families and the full loading procedure remain later tasks under the [plan](IMPLEMENTATION_PLAN.md).
