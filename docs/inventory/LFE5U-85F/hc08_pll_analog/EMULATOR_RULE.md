<a id="hc08-pll-feedback-rule-original-proposal-and-implementation-handoff"></a>

# HC08 PLL feedback rule: original proposal

Implementation update: the planner now applies this rule, with native A–H
regressions and kernel/fast equality checks. See README.md and
implementation_validation.txt. The original proposal below is retained as
design history; its statements about no implementation refer to that stage.

The proposed rule targets `src/compile/mod.rs`. HC08 run_1/run_2 provide
24 ULX3S SRAM loads with repeatable frequency and LOCK observations.
The saved emulator reports are pre-fix evidence; they must not be rewritten
as though the following rule had already executed.

## Decode complete bit patterns before choosing a feedback path

In `execute_pll` (currently around the FEEDBK_PATH match and feedback-port
selection at lines 1525–1590), classify the physical selector bits jointly
with the CLKFB mux. Do not decide from a single textual enum candidate,
and do not turn a `pll_setting` ambiguity/error into external feedback.
The six-bit mask for the measured `MIB_R4C0:PLL0_UL` site is F1B1 through
F6B1. Define `selector6 = sum(bit(FiB1) << (i-1), i=1..6)`; bits outside the
mask must not affect this classification. Missing/unresolved physical bits
are an error. For other PLL sites use their Trellis mapping and existing
qualification, not these literal UL tile coordinates.

| selector6 | Asserted bits (all other mask bits zero) | Classification |
|---|---|---|
| 0x0F | F1B1 F2B1 F3B1 F4B1 | RawVco: measured INT_OP/fabric union, C/D/F/G |
| 0x1F | F1B1 F2B1 F3B1 F4B1 F5B1 | RawVco: measured INT_OS/fabric union, H |
| 0x0E | F2B1 F3B1 F4B1 | Canonical internal CLKOP, E |
| 0x1D | F1B1 F3B1 F4B1 F5B1 | Canonical internal CLKOS, existing qualification only |
| 0x2C | F3B1 F4B1 F6B1 | Canonical internal CLKOS2, existing qualification only |
| 0x3B | F1B1 F2B1 F4B1 F5B1 F6B1 | Canonical internal CLKOS3, retain HC07 qualification |
| 0x03 | F1B1 F2B1 | External JCLKFB3, A/B when return traces to this PLL's CLKOP |

The two RawVco masks must be matched **exactly**. H is 0x1F, not 0x0F;
its current ambiguous enum candidate set `[INT_OP, INT_OS]` is expected from
overlap and does not authorize a generic ambiguity fallback. Validate that
the actual decoded fabric entry is JCLKFB3 and that its routed buffer root
is the same PLL's CLKOP, as measured in HC08. A different return source or
unresolvable route has no HC08 qualification and must be refused. Store the
classification in `PendingPll`, for example as a feedback kind with distinct
Internal(output), External, and RawVco cases. Preserve existing qualified
canonical modes; HC08 does not newly qualify internal OS/OS2 behavior.

Canonical internal patterns take precedence over interpreting the overlapping
low bits as a fabric mux. In particular, canonical INT_OS3 already includes
both mux bits; it must not be reclassified as RawVco just because a mux decoder
reports JCLKFB3. Use CLKINTFB as the route root only for a qualified canonical
internal pattern. For RawVco and External, trace CLKFB for route qualification.
Matching source-level wiring or names alone is insufficient: the input is a
packed bitstream, and nextpnr's symbolic enum/arc lines can overlap.

## Refuse distinguishable unmeasured combinations

Do not generalize "any INT_* plus any fabric arc" to VCO feedback. In the UL
encoding, the distinguishable unmeasured INT_OS2 unions with JCLKFB1/2/3 are
0x2D, 0x2E and 0x2F respectively. Refuse them. Refuse other unknown, ambiguous
or unresolved combined patterns, unsupported fabric entries, and feedback
routes not covered by an existing board qualification. The HC08 external
control only adds evidence for 0x03 with a same-PLL CLKOP return; it adds no
qualification for other fabric arcs or arbitrary user feedback. Preserve any
independent prior board qualification rather than treating HC08 as its removal.

The refusal must include the PLL resource, full mask, decoded candidates/entry,
and the missing qualification, for example:

```text
PLL <resource>: unqualified feedback selector6=0x2F
(INT_OS2/fabric JCLKFB3); HC08 VCO feedback covers only 0x0F and 0x1F
with the measured JCLKFB3 <- same-PLL CLKOP return
```

A request for a different source arc is not always distinguishable after
packing. INT_OP OR JCLKFB1 has the same 0x0F bits as INT_OP OR JCLKFB3;
INT_OS OR JCLKFB2 has the same 0x1F bits as INT_OS OR JCLKFB3. INT_OP OR
JCLKFB2 remains canonical 0x0E, and INT_OS OR JCLKFB1 remains 0x1D.
INT_OS3 OR any of these mux entries remains 0x3B. Identical final bits and
routes cannot be classified differently based on how source text produced
them. Refuse distinguishable unmeasured configurations; do not invent an
intent test that requires unavailable Verilog provenance.

## Frequency construction in finish_plls

At the current feedback-root resolution and divider selection in `finish_plls`
(roughly lines 1620–1680), retain the existing internal/external output-feedback
formula. For RawVco, validate the saved fabric route but **do not multiply by
CLKOP_DIV or any CLKOS*_DIV**. Set the feedback divider factor to 1, and hand
the kernel the reduced VCO period/reference period ratio:

```text
a = CLKI_DIV
b = CLKFB_DIV                 # RawVco
g = gcd(a, b)
vco_per_reference = (a/g, b/g)
Fvco = Fin * b/a
Fout = Fvco / Dout
```

These are actual divider values, after adding one to the routed word values.
For C/D/F/G/H, `(a,b)=(1,4)` gives a 10 ns VCO period and 60 ns CLKOP period.
For canonical internal or external output feedback, keep
`b = CLKFB_DIV * Dfeedback`; A/B/E therefore retain a 10 ns CLKOP period.
Every enabled output uses its own divider in either case. Do not reject zero
analog words or use them to suppress LOCK/clock generation: HC08's zero and
12/8/1/2 analog profiles have the same measured outcomes. This does not newly
qualify arbitrary analog settings.

Use a distinct RawVco feedback identity rather than pretending the feedback
output is CLKOS3 with divider 1, or mutating a configured output's divider.
For a synthetic raw-VCO phase origin, use feedback phase offset 0; keep each
output's own configured phase offset. HC08 measured frequencies and sampled
LOCK, not absolute phase, jitter, or relative phases of the unused outputs.
The planner report must label that phase origin as a modeling convention and
reference HC08 for frequency/feedback qualification.

<a id="kernel-handoff-and-lock-limits"></a>

## Kernel interface and LOCK limits

`src/kernel/pll.rs` already schedules outputs from `vco_per_reference` and
each output divider; it needs no separate formula or bit-pattern decoder.
Add explicit RawVco feedback metadata only as needed for the shared planner /
kernel interface, updating both sides together.
Do not reuse a fake CLKOS3 identity: `feedback_output` also selects HC07 reset
recovery delays around `pll_arm` (currently near line 271).

Retain the existing configured-startup convention of LOCK high initially and
reference-period learning before output edges. HC08 observed synchronized LOCK
high by 10 ms, not its exact raw time of assertion; it supplies no measured
10 ms acquisition delay to add to the kernel. The wrong-frequency cases are
locked, so LOCK must not be gated on achieving the nominal 100 MHz. Mark the
time-zero high convention as the existing startup approximation.

HC08 does not qualify raw-VCO RST/STDBY recovery, dynamic phase changes, or
reference-loss/relock. Keep existing static-control restrictions. For RawVco,
refuse enabled RST/STDBY controls that can be asserted (anything other than a
proven constant-low input) with a message such as `HC08 raw-VCO reset/standby
recovery is unmeasured`; do not silently apply HC07 CLKOP/CLKOS3 recovery fits.
Canonical feedback keeps its separately qualified HC07 behavior.

<a id="review-expectations-for-the-implementation-job"></a>

## Implementation checks

The packed C/D/F/G/H CLI runs should produce 60 ns CLKOP periods and the UART's
166666/166667 edges per 10 ms. A/B/E must remain at 10 ns and 1000000 edges.
Changing CLKOS3_DIV between 8/3/2 must not affect RawVco's CLKOP frequency.
Check H's exact 0x1F case and its old ambiguity separately; check canonical
0x3B remains internal CLKOS3, despite the mux overlap. Include explicit refusal
cases for 0x2D/0x2E/0x2F, unknown bits, different fabric return sources and
raw-VCO reset/standby control. These were proposed checks; at that stage, no
emulator change or post-fix emulator execution had been performed.
