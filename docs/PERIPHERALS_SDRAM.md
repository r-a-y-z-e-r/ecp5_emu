# ULX3S SDRAM

The SDRAM model is in [src/peripherals/sdram.rs](../src/peripherals/sdram.rs). It models the
board's Winbond/Micron 256 Mbit SDR SDRAM (MT48LC16M16A2: four banks of 8192 rows of 512 sixteen-bit
words, 32 MB) from its package balls, with no dependency on either executor.

The ULX3S balls from [board.rs](../src/board.rs) are:

- `SDRAM_CLK` (`F19`), `SDRAM_CKE` (`F20`), `SDRAM_CSN` (`P20`), `SDRAM_RASN` (`R20`),
  `SDRAM_CASN` (`T19`), `SDRAM_WEN` (`T20`);
- `SDRAM_BA[1:0]` (`P19`, `N20`), `SDRAM_A[12:0]` (`M20`..`G19`), `SDRAM_DQM[1:0]` (`U19`, `E20`);
- `SDRAM_D[15:0]` (`J16`..`J17`, bidirectional).

## Sampling and storage

Create `Sdram::new()`, then call `feed(time_ps, &SdramPins)` in time order after every executor
instant, including instants without a `clk` transition.

Zero-delay sampling rule:

- Commands, `cke`, `ba`, `a`, `dqm`, and write `dq` use the sample recorded *before* a rising
  `clk` edge (`previous`). That preserves the setup values when a zero-delay controller changes its
  outputs at the same timestamp as the rising clock edge.
- Read data (`Drive = Option<u16>`) is published one rising edge *before* the controller captures
  it, so for CAS latency 2 (`CL = 2`) a `READ` sampled at rising edge $k$ puts data on `Drive`
  after edge $k + 1$, ready to be captured at edge $k + 2$ (`CL = 3` is one edge later).
- `drive_mask()` returns a 2-bit byte mask (`!dqm & 3` sampled two edges before capture): bit 0 is
  the low byte (`dq[7:0]`), bit 1 the high byte (`dq[15:8]`). When a byte is masked, only the
  unmasked byte is driven and the masked byte stays `Z`.
- Clock suspend (`cke != Logic::One`) delays all queued read beats and auto-precharge deadlines by
  one clock edge.
- Unknown (`X`/`Z`) command or address bits on a non-idle command are counted in `unknown_samples`
  and ignored. Unknown write bytes leave the existing memory byte unchanged.

Memory is allocated sparsely in 512-word rows. Each row sits behind an `Arc` (`Arc::make_mut` on
write) so live-window checkpoints share unchanged rows instead of copying memory.

## Timing checks and assumptions

`feed` measures `clock_period_ps` between consecutive rising edges of `clk` and converts datasheet
nanosecond minimums to whole clock cycles using $\max(\lceil t / T \rceil, \text{min\_clocks})$:

| Check | Requirement |
|---|---|
| `init` | $\ge 100\text{ }\mu\text{s}$ (`100_000_000 ps`) of `NOP`/`DESELECT` after the first sample, then `PRECHARGE ALL` (`A10 = 1`), two `AUTO REFRESH`, and `LOAD MODE` |
| `tRP` | `PRECHARGE` to `ACTIVE` / `AUTO REFRESH` / `LOAD MODE`: $\max(\lceil 20\text{ ns} / T \rceil, 2)$ clocks |
| `tRCD` | `ACTIVE` to `READ` / `WRITE`: $\max(\lceil 20\text{ ns} / T \rceil, 2)$ clocks |
| `tRC` | `ACTIVE` to `ACTIVE` (same bank): $\max(\lceil 66\text{ ns} / T \rceil, 1)$ clocks |
| `tRAS` | `ACTIVE` to `PRECHARGE` (same bank): $\max(\lceil 44\text{ ns} / T \rceil, 1)$ clocks |
| `tWR` | Last `WRITE` data to `PRECHARGE` (same bank): $\max(\lceil (T + 7.5\text{ ns}) / T \rceil, 2)$ clocks |
| `tRFC` | `AUTO REFRESH` to next non-idle command: $\max(\lceil 66\text{ ns} / T \rceil, 1)$ clocks |
| `tMRD` | `LOAD MODE` to next non-idle command: 2 clocks |
| `refresh` | At least 8192 `AUTO REFRESH` commands in every rolling 64 ms window once `LOAD MODE` completes initialisation (checked every $64\text{ ms} / 8192$ thereafter, even if `clk` stops) |
| `bank` / `mode` | `ACTIVE` on an open bank, `READ`/`WRITE` on a closed bank, `REFRESH`/`LOAD MODE` with any bank open, or reserved mode bits |

Assumptions:

- Mode register supports burst lengths 1, 2, 4, 8, and full page (512), sequential or interleaved
  bursts (sequential only for full page), CAS latency 2 or 3, and single-location write (`A9 = 1`).
  Reserved mode bits log a `mode` violation and fall back to CL2/sequential.
- `timing_violations` counts every violation; `messages` keeps the first diagnostic of each kind
  (up to 32 entries).

## Fast mode and the live window

In fast mode ([src/fast.rs](../src/fast.rs)), the 16 `sdram_d` balls are compiled as external
tristate drivers starting at `Logic::Z` (`None`):

- Each pad follows the kernel: the design's output while its output enable is active, else the
  external level when not `Z`, else the pad's `PULLMODE`. If both the design and the external
  driver drive different levels at once, fast mode stops with an error naming the pad.
- Because read data is due one SDRAM clock after `READ` at CL 2, [tests/sdram.rs](../tests/sdram.rs)
  uses `FastSim::run_edges_with` to feed `Sdram::feed` after each board-clock edge
  and deliver changed DQ bits (respecting `drive_mask()`) 1 ps later. The new entry point
  resolves probe and input slots once, reuses its buffers, and refines time once per call.
  The existing sampled methods and generated C are unchanged.

In the live runner ([src/peripherals/mod.rs](../src/peripherals/mod.rs),
[src/live.rs](../src/live.rs)):

- `Attached` attaches an `Sdram` when the design drives all 23 control, bank, address, and DQM
  balls and all 16 `SDRAM_D` balls are `Direction::Bidirectional`.
- `input_balls()` registers all 16 `SDRAM_D` balls starting at `None` (`Logic::Z`).
- `horizon()` still returns `Some(1)` while an SDRAM is attached. While running, the runner uses
  `run_edges_with` in batches of at most 1024 edges, feeding all 39 traced pads after every edge
  and after every answer instant. `respond_into` reuses an answer buffer and avoids scanning
  all 16 DQ bits when the driven word and mask are unchanged. Commands, input deadlines, pacing,
  checkpoints and publishing are checked between batches. Paused steps keep their wire readings.
- `View::sdram` exposes `sdram::Status`, and checkpoints keep the `Sdram` (sharing untouched rows
  via `Arc`) plus the DQ levels last given so going back replays deterministically. Replay uses
  the same edge callback and answer timing; restoring invalidates the cached answer comparison.

## On the board

[HR06](inventory/LFE5U-85F/hr06_sdram/README.md), 2026-11-16:
- `sdram_test` printed `SDRAM W8000 E0000`, as fast mode does.
- `picorv32_sdram` counted the emulator's cycles for every pass it reported: 854997 for pass 1
  and 855452 for passes 2–6, with 0 memtest errors. That count depends on every SDRAM access and
  refresh, so the controller, the pads and the model agree to the cycle.

## Limits

- **PLL-clocked SDRAM:** the live runner steps on board-clock edges, so an SDRAM clocked by a PLL
  faster than the 25 MHz board clock cannot be answered between PLL edges in time.
- **Pad `PULLMODE`:** fast mode stays two-valued for logic. If an `sdram_d` pad has
  `PULLMODE=NONE` and is read while neither the design nor the model drives it, fast mode refuses
  the design (the event kernel puts `X` on a floating input). The reference designs set
  `PULLMODE=UP` on `sdram_d[15:0]`.
- **Not modelled:** data loss on missed refreshes (only `refresh_shortfalls` is counted),
  power-down / self-refresh modes, and sub-clock analogue setup/hold/output-enable timing (`tAC`,
  `tOH`).

## Checks and measured speeds

Unit tests (`cargo test --release -p ecp5_emu --lib`): `peripherals::sdram::tests`
and `peripherals::tests::attached_sdram_horizon_respond_and_snapshot` pass.

Reference designs (25 MHz board clock, controller `sdram_clk = ~clk`, CL 2, BL 1, auto precharge):

- [sdram_test](../examples/ulx3s/sdram_test/top.v) (375 LUT4, 193 FF, 49 PIO; nextpnr
  137.84 / 152.11 MHz): writes 64 KB of an LFSR pattern, reads it back, and prints
  `SDRAM W8000 E0000`.
- [picorv32_sdram](../examples/ulx3s/picorv32_sdram/top.v) (1680 LUT4, 812 FF, 4 EBR, 128 LUTRAM
  columns, 49 PIO; nextpnr 86.06 / 89.65 MHz): PicoRV32 running entirely from SDRAM, repeating
  passes of a 2000-limit sieve and a 256 KB LFSR memtest with a pass-dependent seed.

Measured on `rserv` (`--release`, 2026-11-16), against **8ac90ff**. Both sides use that
base's default widening cone folding with up to 12 inputs. Earlier measurements from before
that default changed are not used in this comparison. The baseline was a clean detached
checkout of 8ac90ff; the sandbox prevented creating a linked worktree, so the checkout lived
under `/private/tmp` and was removed afterwards. Builds and all tests ran on rserv, without
a GUI. Timed work excludes synthesis, loading and C compilation. Live speeds are the runner's
reported 500 ms samples.

| Runner | Work | Clean 8ac90ff (cycles/s) | With these changes (cycles/s) | Gain |
|---|---|---:|---:|---:|
| `tests/sdram.rs`, fast | `sdram_test`, 706,290 cycles | 577,617 | 2,822,764 | 4.89× |
| `tests/sdram.rs`, fast | `picorv32_sdram`, passes 0–1, 60,225,215 cycles | 278,923 | 654,853 | 2.35× |
| Live runner | `picorv32_sdram`, pass 0 | 371,565 | 624,491 | 1.68× |
| Live runner after going back | `picorv32_sdram`, pass 1 | 371,397 | 624,015 | 1.68× |

Fast wall times were 1.223 → 0.250 s for `sdram_test` and 215.920 → 91.968 s
for PicoRV32. The four-pass correctness run completed 120,255,639 cycles in 189.064 s.
All SDRAM runs had zero timing violations, unknown samples, framing errors and memtest errors.

The host changes remove the repeated sampled setup: probe and input slots are resolved once
per callback run, and probe rows, traces and answer batches retain their capacity. Answers
still execute at a separate instant 1 ps after the edge, and that instant's trace is fed too.
The existing sampled entry points and generated C are unchanged. With one clock and no PLLs,
the callback entry point uses the already exported `fast_step_input`. The live runner checks
commands, input deadlines, pacing, history and publishing between batches of at most 1024
edges; replay uses the same per-edge delivery and restores the peripheral state.

A further trial replaced the SDRAM pending-read tree with a reusable ordered queue, including
clock-suspend and snapshot checks. PicoRV32 stayed effectively unchanged (654,853 → 655,244
cycles/s), `sdram_test` measured 2,822,764 → 2,710,153, and live pass 0 measured 624,491 →
619,171. The trial did not pay and was reverted, including its additional test. No peripheral
model timing or memory representation was changed in the retained implementation.

The horizon remains one for these designs: their measured SDRAM clock period is 40,000 ps,
the board clock or its inverse, and CL2 gives `2 * (CL - 1) - 1 = 1` safe board edge. No larger
horizon was enabled on a clock-period guess. Read data for a READ at SDRAM edge n remains due
after edge n + CL - 1.

The requested 3× speedup is met for `sdram_test`, but remains unmet for PicoRV32 and the live
runner. The additional allocation trial did not improve those workloads, so no further
speculative batching or clock assumptions were added.

UART counts remain exact: pass 0 reports `855498 cycles`; with `SDRAM_PASSES=4`, passes 1–3
report **854997, 855452, 855452** cycles. Each pass reports 303 primes and zero memtest errors,
matching HR06 for passes 1–3. The live test goes back 100,000 cycles and replays to the identical
paused cycle, comparing the full SDRAM status, UART console and LEDs before continuing to pass 1.

Validation on rserv: all four SDRAM tests, all four live peripheral tests, all nine live board
tests, SD card, instant trace, both external-tristate tests, all three HDMI tests and the library
tests pass. `cargo clippy -p ecp5_emu --all-targets -- -D warnings` is clean.
