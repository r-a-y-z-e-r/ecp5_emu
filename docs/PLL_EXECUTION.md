# PLL execution (EHXPLLL)

Added on 2026-10-29. The emulator executes EHXPLLL with static settings and a periodic
reference clock. The model is
[kernel/pll.rs](../src/kernel/pll.rs); the planner part is `execute_pll` and `finish_plls` in
[compile/mod.rs](../src/compile/mod.rs).

## Architecture data

The structural model had the four PLL sites only as anchors, without ports, so a design using a
PLL left its outputs undriven. The emulator ran it silently with a stuck counter and `z` on LOCK.

[tools/bind_trellis_sites.py](../tools/bind_trellis_sites.py) now emits the EHXPLLL ports from
`Bels.cpp:add_pll`: 22 per PLL, 88 in total.
[trellis_resource_probe.cpp](../tools/trellis_resource_probe.cpp) verifies them against the native
constructor (506 constructors, exact port match). `resource_ports.csv`, `model_manifest.json` and
`configuration_manifest.json` were regenerated. The other inventory tables are byte-identical.

## Behaviour

| Aspect | Model | Basis |
|---|---|---|
| Frequency | Output feedback: `Fout = Fin × CLKFB_DIV × Dfb / (CLKI_DIV × Dout)`; HC08 raw-VCO feedback: `Fout = Fin × CLKFB_DIV / (CLKI_DIV × Dout)`; dividers are stored minus one | HC05/HC07 output feedback; HC08 A–H board captures |
| Feedback | Exact canonical `INT_*` uses `CLKINTFB`; qualified external feedback traces `CLKFB`; the two measured INT_OP/INT_OS plus fabric unions select the raw VCO | complete selector bits and CLKFB mux interpreted together, not an ambiguous/subset enum fallback |
| Phase | `CLKOS*` offset from the feedback output by `(8·CPHASE + FPHASE)` differences in eighths of a VCO period; the feedback output is aligned to reference rising edges | ecppll's phase convention (tools only) |
| Duty cycle | 50% | assumption |
| LOCK, INTLOCK | high from the start; an enabled RST/STDBY holds them low, then LOCK rises after 98304 phase-detector periods, feedback latency and acquisition delay | HC05 startup; HC07 LOCK after release (INTLOCK remains unmeasured) |
| RST, STDBY | DISABLED ignores its input; ENABLED holds outputs and LOCK low 1 ps after assertion; either live control holds the PLL | HC07, HC07b: all four sites, RST and STDBY |
| Restart | old output phase grid; short-hold delay: one reference period at CLKI_DIV 1; two with CLKOP feedback or 4.5 with CLKOS3 at CLKI_DIV 5; holds >= 1 µs use 2.2 µs; full rate immediately | HC07: short restart, long first edge and phase; the board's ramp is not modelled |
| Feedback latency | six reference periods at CLKI_DIV 1 / CLKOS3, eight at CLKI_DIV 1 / CLKOP, 25 at CLKI_DIV 5 with either feedback | HC07b short LOCK: the draft was one/three cycles early at CLKI_DIV 1; CLKI_DIV 5 matched |
| Acquisition delay | linear between hold/delay pairs in µs: 0.32/0, 0.64/4, 1.28/43, 4/134, 10.24/300, 40/424; constant above 40 µs | HC07, HC07b: extra LOCK delay, with about ±15% spread between sites |
| Output enables | a static `*_ENABLE=ENABLED` runs the output; `ENCLK*` is not used | HC05 with `ENCLKOP=0` |
| Reference period | learnt from the first two rising edges of `CLKI`; outputs stay low until then | on the board the reference already ran during configuration |
| Reference change | a rising edge off the learnt period faults the kernel | relock is not measured (C06) |

Rejected with an explicit error:
- `DPHASE_SOURCE`, `SYNC_ENABLE`, `REFIN_RESET` or `INTFB_WAKE` enabled;
- REFCLK bypass in `OUTDIVIDER_MUX*`;
- output trims other than `RISING`/`0`;
- a disabled output that the plan reaches;
- `CLKFB` fed by anything other than one of the PLL's own outputs (user feedback);
- unmeasured FEEDBK_PATH/CLKFB overlap patterns, a raw-VCO fabric return from
  anything other than the same PLL's CLKOP, or live enabled RST/STDBY with
  raw-VCO feedback (HC08 did not measure recovery).

Each generator schedules its output edges one reference period ahead, through a kernel timer
(`timer:pll:<site>`). Edge times are exact: they lie on a grid of `T_ref / (16 · CLKFB_DIV · Dfb)`,
on which VCO periods, phase eighths and half output periods are whole steps.
For HC08 raw-VCO feedback the frequency factor `Dfb` is 1; no configured
output divider is changed to obtain that factor.

Fast mode runs PLLs with no live enabled control. It explicitly refuses a PLL whose
`controls` list is nonempty, with "RST/STDBY enabled (HC07); the event kernel does".
Only controls enabled by PLLRST_ENA/STDBY_ENABLE enter this list; disabled inputs
are not planned.

Internal feedback through `CLKINTFB` is supported, including LiteX's
`FEEDBK_PATH=INT_OS3` with CLKFB and CLKINTFB on the same net. FEEDBK_PATH bits
overlap the CLKFB routing mux bits. Canonical INT_OS3 sets both mux bits, which
the raw routing decoder reads as JCLKFB3, but remains internal feedback (HC07).
The planner compares the complete physical selector against Trellis patterns
for each site before choosing the feedback mode; unrelated bits are excluded.
Input planning is queued; the suspected re-entry/overwritten-entry bug did not
occur.

### HC08: INT_OP/INT_OS with a fabric CLKOP return

[HC08](inventory/LFE5U-85F/hc08_pll_analog/README.md#final-reading-run_1-and-run_2)
measured 24 SRAM loads: A/B/E run at 100 MHz; C/D/F/G/H run at 16.666667 MHz,
all with synchronized LOCK high by 10 ms. C/D/F/G combine INT_OP with fabric
JCLKFB3; H combines INT_OS with the same fabric return. In the upper-left PLL,
the complete asserted selector bits are respectively F1B1–F4B1 (0x0F) and
F1B1–F5B1 (0x1F), with all other bits of the six-bit mask zero. Neither is a
canonical internal selector. The planner derives equivalent patterns from
each site's Trellis data, rather than using UL bit coordinates elsewhere.

Only these exact unions, with JCLKFB3 traced back to the same PLL's CLKOP,
qualify the raw-VCO rule: `Fvco = Fin × CLKFB_DIV / CLKI_DIV`. Each output then
divides that VCO by its own divider. Changing CLKOS3_DIV 8 -> 3 -> 2 leaves
CLKOP unchanged; the rejected INT_OS3-divider hypothesis does not explain the
measurements. Zero versus the working 12/8/1/2 analog words likewise does not
change the board outcomes. A/B external output feedback and E's canonical
internal feedback retain the output-divider factor and 100 MHz CLKOP.

Other distinguishable overlap patterns (including INT_OS2 plus fabric
JCLKFB1/2/3, UL masks 0x2D/0x2E/0x2F) are refused with the FEEDBK_PATH, CLKFB
source and HC08 qualification in the message. An identical final bit pattern
cannot reveal which source-level enum/arc combination produced it. Canonical
internal patterns retain precedence, including INT_OS3's overlapping mux bits.

LOCK is not a test of achieving the intended frequency. Its existing time-zero
high convention remains a configured-startup approximation; HC08 supplies no
exact raw lock-onset delay. Raw-VCO phase uses a synthetic zero feedback phase
origin (unmeasured), and live RST/STDBY is refused rather than borrowing HC07's
output-feedback recovery fits. The kernel and fast mode consume the same VCO
period ratio and output dividers; no separate fast-mode formula is needed.

## Related planner change

nextpnr can start a carry chain in a PLC whose `FCI` comes from the `FCO` of a LOGIC-mode slice
next to it. No carry function is established for that output (S04). The planner now drives it as
an explicit `x` and records it in `report.constants`, instead of rejecting the design. The first
CCU2 half of such a chain selects its generate term, so `CIN` does not affect it; any real use
would show as `x`.

## Checks

- `tests/pll_feedback_native.rs` (`--include-ignored`) builds and packs HC08
  A–H in temporary scratch, checks 10 ns CLKOP for A/B/E and 60 ns for
  C/D/F/G/H, and compares complete 1 ns event/fast traces including LOCK.
  Raw-VCO cases also check all three other output dividers. Packed negative
  cases cover the three INT_OS2 overlaps, an unknown selector, wrong fabric
  return, and live RST/STDBY; canonical INT_OS3 remains output feedback.

- `tests/event_kernel.rs`, `pll_outputs_follow_dividers_and_phase_and_lock_is_high_from_the_start`:
  exact edges of two outputs with different dividers and phases, LOCK from the start, and the
  fault on a changed reference period.
- `tests/pll_native.rs` (`--ignored`) runs the exact HC05 configuration that ran on the board,
  with a 40 ns reference, and decodes its UART report. The result equals the board's
  `L=000002 P=000002 F=040000 D=00` (6 of 6 loads). That is about 349k reference cycles and
  1.4M PLL cycles, and it takes ~33 s.
- `tests/event_kernel.rs`, `pll_rst_holds_outputs_and_lock_and_restarts_on_its_grid`:
  short and long enabled holds for CLKOP/CLKOS3 feedback at CLKI_DIV 1/5,
  output/LOCK cancellation, restart on the old phase grid, acquisition delay
  and an unknown control.
- `tests/pll_reset_native.rs` (`--ignored`) replays at least three HC07 events and
  two HC07b events through the event kernel. HC07 first primes its previous-event
  ph1 field with one report; that initial phase sample has no match in the board record.
  Each UART field of the subsequent events is compared with the
  board records at the same W. Disabled controls match exactly. Enabled short holds
  check first/lock against the board range, lost within four and bmin within three.
  Longer holds check first within three and lock within the same-CLKI_DIV envelope
  across both records, extended by 15% of the extra delay over short holds. The
  fields affected by the absent ramp are printed without assertions. It also checks
  fast mode's live-control refusal. Evidence: [HC07](inventory/LFE5U-85F/hc07_pll_reset/README.md).
- `tests/fast_mode.rs`, `fast_mode_matches_the_event_kernel_through_a_pll`
  (`--ignored`): the static PLL still agrees with the event kernel.
- `tests/device_model.rs` checks the port total, now 3 246 331.

## Open

- the output frequency ramp after a long RST/STDBY hold;
- recovery latency for other feedback/divider profiles (the old one-reference-period
  restart and five-phase-detector-period latency assumption remains there);
- relock after a change of the reference;
- `INTLOCK` and `INT_LOCK_STICKY`;
- `PLLWAKESYNC` and `INTFB_WAKE`;
- dynamic phase stepping;
- REFCLK bypass;
- the real duty cycle of odd dividers;
- phase between outputs, which is ecppll's convention and not measured;
- user feedback through logic.
