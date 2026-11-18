# HC07 slice: EHXPLLL RST and STDBY (C06, C07)

This series was first labelled HC06, a label the OSCG series already had. Its files
keep the hc06 names they ran under.

What RST and STDBY do while a PLL runs, with their enables on and off.
[HC05](../hc05_pll_lock/README.md) measured lock at configuration. This series measures
stopping and restarting all four PLL sites.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381.
  Loaded into SRAM with `openFPGALoader -b ulx3s` under `out/board.lock`. The flash was
  not written. Tool versions and commands are in [TOOLS.txt](TOOLS.txt); checksums are
  in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hc06_gen.py](hc06_gen.py) writes [hc06.v](hc06.v) and [hc06b.v](hc06b.v).
  The routed configurations are [hc06.config](hc06.config) and [hc06b.config](hc06b.config),
  both with [hc06.lpf](hc06.lpf). Their `.tile_group` lines confirm these sites:

  | Build | PLL | Site | Control | Enable | Feedback | CLKI_DIV |
  |---|---|---|---|---|---|---:|
  | hc06 | pa | LR | RST | DISABLED | CLKOS3 through CLKINTFB | 1 |
  | hc06 | pb | UR | RST | ENABLED | CLKOS3 through CLKINTFB | 1 |
  | hc06 | pc | LL | STDBY | DISABLED | CLKOP | 1 |
  | hc06 | pd | UL | STDBY | ENABLED | CLKOP | 1 |
  | hc06b | pa | LR | RST | ENABLED | CLKOP | 1 |
  | hc06b | pb | UR | STDBY | ENABLED | CLKOS3 through CLKINTFB | 1 |
  | hc06b | pc | LL | RST | ENABLED | CLKOS3 through CLKINTFB | 5 |
  | hc06b | pd | UL | RST | ENABLED | CLKOP | 5 |

  Every PLL has 25 MHz in, a 600 MHz VCO and 50 MHz CLKOP. CLKOS3 feedback uses
  `FEEDBK_PATH=INT_OS3`, `.CLKFB(fb), .CLKINTFB(fb)`, as LiteX wires it, with
  ICP 6 / LPF 16. CLKOP feedback uses ecppll's ICP 12 / LPF 8.
- **Pulse:** one signal drives all four PLLs. W is drawn from 1, 2, 3, 4, 5, 8, 16, 32,
  100, 256, 1000, 4096, 10000, 25000, 50000 and 100000 reference cycles of 40 ns.
- **Observer:** reference-clocked logic reads gray edge counters through two flip-flops.
  It sends `H`, W in five hex digits, then the fields of pa, pb, pc and pd over the FTDI
  UART (L4, 115200 8N1). Each PLL has 35 hex digits in hc06 and 72 in hc06b.
  [The generator's docstring](hc06_gen.py) defines every field.
  hc06 takes ph1 from the previous event, reads lock as 0 for W=1 and wraps during at
  65536. hc06b fixes these, adds the count of LOCK rises and twelve 1024-cycle edge
  windows, and reports lock from the fourth cycle after the fall. Counts include
  two to three cycles of observer latency.
- **Runner:** [run.py](run.py), `run.py hc06.bit H 3 25` and `run.py hc06b.bit H 3 40`.
- **Analysis:** [hc06_analyse.py](hc06_analyse.py), with [analysis.txt](analysis.txt) and
  [analysis_b.txt](analysis_b.txt). It parses every distinct report line per load.

## Observations

On 2026-11-16, three loads of hc06 ran for 25 s each (13:59–14:01Z). They recorded
3552 events: 1186, 1183 and 1183. Three loads of hc06b ran for 40 s each
(14:06–14:09Z). They recorded 2410 events. Each load's distinct lines are in
[observations.log](observations.log) and [observations_b.log](observations_b.log).

## Reading

1. **DISABLED ignores the input.** In all 3552 hc06 events, at every width, pa and pc
   never lost LOCK. They counted 2W rising edges during the hold (with the field's
   wrap), 128 edges in every 64-cycle block and no phase change.
2. **ENABLED holds the PLL.** LOCK fell within the first reference cycle; drop=2 is
   the synchronizer latency. No CLKOP edge occurred during the hold (during=0).
   The output stopped low. In hc06b load 3, pa read level=1 in 177 of 444 long
   holds. Its edges lay on the observer's sampling edge in that load, and ph0
   parity was mixed too. This is an observer artifact.
3. **RST and STDBY act alike.** The same UR site and loop with RST (hc06 pb) and
   STDBY (hc06b pb) gave first=60 and 60, and lock medians 108784 and 108768 for
   W >= 1000.
4. **Short holds, W <= 8 (up to 320 ns):** CLKOP restarted at once at its rate;
   first=0..3 is the observer latency. With CLKI_DIV 1 it lost W+1 reference
   periods of edges (W+2 for one PLL at W=8). With CLKI_DIV 5 it lost W+2 with
   CLKOP feedback and W+4.5 with CLKOS3 feedback. LOCK rose at a fixed delay in
   every event: 98311 cycles for CLKI_DIV 1 with CLKOS3 feedback, 98313 with
   CLKOP feedback, and 491546 for CLKI_DIV 5. This is 98304 = 3·2^15
   phase-detector periods plus latency.
5. **Long holds, W >= 32 (1.28 µs):** the VCO stopped. The first edge came
   51..61 reference cycles after release (2.0..2.4 µs). The output then ran at
   58..61% of its rate: 1172..1248 of 2048 edges in the first 1024-cycle window
   after W >= 1000. It reached its rate 9400..12100 cycles later (0.38..0.48 ms).
   With CLKI_DIV 5 it overshot slightly, up to 133 of 128 edges in a 64-cycle
   block. LOCK rose 98304 phase-detector periods later still.

   Extra LOCK delay over a short hold, across the six enabled PLLs:

   | W | Extra reference cycles |
   |---:|---:|
   | 16 | 41..172 |
   | 32 | 646..1458 |
   | 100 | 2578..3949 |
   | 256 | 6128..8357 |
   | >= 1000 | 9090..11436 |

   From W=1000 to 100000 the median was about 10600, independent of W.
   This delay is in absolute time. CLKI_DIV 5 gave +10920 and +11260,
   against +9090..11436 with CLKI_DIV 1.
6. **Phase returned.** After every hold, short or long, CLKOP returned to the
   same phase against the reference. ph1−ph0 was a whole number of CLKOP
   periods, with the same value modulo 4 per PLL. LOCK rose once per event
   (lrise=1 in hc06b).
7. **The planner misread internal feedback.** FEEDBK_PATH shares bits with the
   CLKFB routing mux. INT_OS3 sets both mux bits, so tracing the mux alone
   selected JCLKFB3 and ended at a fabric JCLK0 instead of CLKINTFB. The planner
   now uses CLKINTFB for INT_* settings and traces CLKFB for external feedback.
   The proposed re-entry explanation did not apply: input planning is queued.

**Not covered:**

- INTLOCK and INT_LOCK_STICKY;
- PLLWAKESYNC and INTFB_WAKE;
- both enables at once (the model ORs them);
- holds shorter than one reference cycle;
- a hold at configuration;
- other loop filters and VCO frequencies;
- the spread between sites (about ±15%).

## Emulator check

[PLL_EXECUTION.md](../../../PLL_EXECUTION.md) describes the event kernel model in
[src/kernel/pll.rs](../../../../src/kernel/pll.rs):

- Outputs and LOCK fall 1 ps after an enabled control rises.
- Outputs restart on the old grid. For holds under 1 µs, the delay is one
  reference period with CLKI_DIV 1. With CLKI_DIV 5 it is two periods for
  CLKOP feedback and 4.5 for CLKOS3 feedback. Longer holds use 2.2 µs.
- LOCK rises after 98304·CLKI_DIV reference periods, plus a feedback latency
  and `acquisition(hold)`. The feedback latency is six reference periods for
  CLKI_DIV 1 / CLKOS3, eight for CLKI_DIV 1 / CLKOP, and 25 for CLKI_DIV 5
  with either feedback. Acquisition is linear between (hold, extra delay), in µs:
  (0.32, 0), (0.64, 4), (1.28, 43), (4, 134), (10.24, 300), (40, 424).
  It stays at 424 µs above 40 µs.
- The output runs at its full rate from restart. The ramp is not modelled.

[tests/pll_reset_native.rs](../../../../tests/pll_reset_native.rs) plans all four
PLLs in each configuration and replays their UART events against the records at
that W. hc06 first runs one report to prime its previous-event ph1 field;
that initial phase sample has no match in the board record. It then compares three
complete events. hc06b compares two. The replay stops at those counts, without
an extra event for margin. Disabled inputs must match exactly. Enabled inputs
must give drop=2 and during=0. Short holds check first and lock against each PLL's board range,
lost within four counts and bmin within three. Longer holds check first within
three counts and lock against all enabled PLLs of the same CLKI_DIV, with 15%
of their extra acquisition delay allowed. Other long-hold fields are printed
beside the board ranges; they are not asserted because the ramp is absent.
The first replay found that the draft model's short LOCK was one cycle early
with CLKOS3 feedback and three early with CLKOP at CLKI_DIV 1. CLKI_DIV 5
already matched. Its short-hold lost field was 14 counts high with CLKOS3 and
six high with CLKOP at CLKI_DIV 5. The feedback latencies and short delays
above correct these constants; the comparison bounds were not widened. Other
feedback/divider recovery profiles keep the old one-period restart and
five-phase-detector-period latency assumption.

Fast mode refuses a live enabled RST/STDBY control. The native tests stay ignored
by default because they need the image reader and Trellis database.

[tests/event_kernel.rs](../../../../tests/event_kernel.rs) checks a short and a
long hold, cancellation of pending edges, the old phase grid, LOCK timing and
an unknown control. [tests/pll_native.rs](../../../../tests/pll_native.rs) keeps
HC05's configured-startup line as a regression check.
