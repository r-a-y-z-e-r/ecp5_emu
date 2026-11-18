# HD09: DSP clock dividers disabled

Question: does `CLK0_DIV=DISABLED` change which edges capture each DSP register?
HD08 used nextpnr's default `ENABLED`: ALU C registers captured every other edge,
with GSR holding the divider at zero; the other observed registers captured every
edge. Three-load ULX3S board records and the six configurations that ran are now
in this directory. The board build disabled fabric FF GSR as HD07/08 did.

## Designs

| Files | Base and active registers | UART line |
|---|---|---|
| `hd09.v`, `hd09s.v` | Exactly HD08 **c**, including nets for constants, no IN_1, C/opcode/output registers | `M` + 5120 hex, 128 pre-edge 160-bit samples |
| `hd09m.v`, `hd09ms.v` | HD07: four ALU output probes and four standalone MULT18 output probes | `M` + 3840 hex, 256 pre-edge 60-bit samples |
| `hd09p.v`, `hd09ps.v` | HD09m with the four standalone MULT18 changed to the isolated stages below | Same layout as HD09m |

Every ALU54B and MULT18X18D explicitly receives **all four** `CLK0_DIV` through
`CLK3_DIV` parameters as the string `"DISABLED"`. This includes ALUs in HD09m/p:
ALU and MULT18 configuration fields can share hardware bits, so leaving any
participant at its default would confound the test. **CLK0..3 all connect to
clk_25mhz**, while every active register still selects CLK0. This aliasing works
around nextpnr's divider remapping bug described below. All DSP GSR settings remain
ENABLED. The unused physical clock pins are disconnected by remapping.
Apart from the clock pin aliases and divider settings, the fabric, UART, pins,
BELs, traffic, output sampling, and other parameters of
HD09 and HD09m remain those of their references. The `s` variants record the first
window from configuration and resend it; traffic continues during UART.

HD07 actually enables **only output** registers. HD09p supplies the missing input
and pipeline coverage without changing HD09m or its line format:

| MULT18 | Site in block X22, row 58 | Sole active register | CE / RST | RESETMODE |
|---|---|---|---|---|
| ma | MULT18_0 | INPUTA | CE1 / RST2 | ASYNC |
| mb | MULT18_1 | INPUTB | CE2 / RST3 | SYNC |
| mc | MULT18_4 | PIPELINE | CE3 / RST1 | SYNC |
| md | MULT18_5 | OUTPUT | CE0 / RST0 | ASYNC |

For ma, A is `{0,c[16:1],0}` and B is `0x20001`; the others swap these operands.
The combinational product copies c[16:1] into P[16:1] and P[33:18]. Exactly one
stage delays that data in each probe, so the HD07 bit analysis still applies.
MULT18 C registers are covered by HD09's paired ALU/MULT18 C probes.

## Generate and check

From this directory:

```sh
python3 -B hd09_gen.py > hd09.v
python3 -B hd09_gen.py s > hd09s.v
python3 -B hd09m_gen.py > hd09m.v
python3 -B hd09m_gen.py s > hd09ms.v
python3 -B hd09p_gen.py > hd09p.v
python3 -B hd09p_gen.py s > hd09ps.v
python3 -B check.py
```

The small generators run the existing HD08/HD07 generators read-only and add the
explicit divider parameters and clock pin aliases. HD09s uses the **c** design with the recording state
changed to repeat; it does not revert to HD08s's IN_1 or literal constants.
`check.py` compares each checked-in Verilog file with its generator, runs
`yosys -q -p 'read_verilog X.v; synth_ecp5 -top top -json .check.json'`, verifies
DSP cell counts, all four parameters and equal clock-pin connections on every
DSP cell, checks HD09p's isolated
stage parameters, and runs both analysis selftests. Temporary JSON stays here and
is removed. `validation.txt` records the checks performed for this delivery.

Primitive/source inspection used local Yosys 0.70's `ecp5/cells_sim.v` and its
includes; this version declares DSP parameters in `ecp5/cells_bb.v` (MULT18 around
line 243, ALU54B around 546). Both expose all four string parameters with default
ENABLED. The local nextpnr `ecp5/bitstream.cc`, `write_mult18` around 1126 and
`write_alu54` around 1184, writes each enum from the cell parameter, defaulting
to ENABLED. Synthesis parameters alone are insufficient: placement remaps and
erases these fields before the writer runs. Routing and ecppack are now checked
in scratch space; see `validation.txt` and `check_routed.py`.

Synthesis counts (full carry and mux counts are in `validation.txt`):

| Design | ALU54B | MULT18X18D | TRELLIS_FF | LUT4 |
|---|---:|---:|---:|---:|
| hd09 | 11 | 22 | 20094 | 17599 |
| hd09s | 11 | 22 | 20094 | 18501 |
| hd09m / hd09p | 4 | 12 | 14270 | 14287 |
| hd09ms / hd09ps | 4 | 12 | 14270 | 15824 |

<a id="routing-and-recording-handoff"></a>

## Routing and recording

Use `hd09.lpf` for every design and the reference flow's 25 MHz clock constraint.
As in HD07/08, after synthesis keep fabric FFs out of the user GSR with:

```yosys
setparam -set GSR "DISABLED" t:TRELLIS_FF
```

Place that command **after** `synth_ecp5` and write the routing JSON afterward;
it changes fabric FFs only, leaving DSP GSR ENABLED. The required plain synthesis
checks use the exact synthesis flow before this routing step.
Do not apply an equivalent blanket GSR change to the DSP cells.

### Why the original source did not disable the active clock

The local source at `~/FPGA/nextpnr` explains the complete path:

- `ecp5/pack.cc`, `pack_dsps` around 1186, creates all CLK0..3 ports, even if
  disconnected. Constant handling around 988 preserves tied DSP control pins as
  connections to the packer constant net.
- After placement, `ecp5/arch.cc` around 690 calls `remap_dsp_blocks` in
  `ecp5/arch_place.cc` around 254. It canonicalizes clock nets within each DSP
  block and updates the `REG_*_CLK` selectors.
- `remap_dsp_cell`, around 401..425, copies `CLKn_DIV` only for pins that **move**.
  It erases the parameters for pins that stay in place. The original real clock
  remained CLK0, so its DISABLED parameter disappeared. The three tied-low pins
  collapsed onto CLK1; their moved parameters survived there instead.
- `ecp5/bitstream.cc` around 1126/1184 then writes ENABLED for each missing
  parameter. That produced CLK0 ENABLED, CLK1 DISABLED, CLK2/3 ENABLED.

The generators now connect all four DSP clock pins to clk_25mhz. CLK1..3 collapse
onto CLK0, copying DISABLED onto the active CLK0 field. Every active register
selector remains CLK0. The inactive physical CLK1..3 divider enums become ENABLED,
but no register selects those clocks. **No config patch or nextpnr modification is
required.** Retain these aliases even though only CLK0 is named by the registers.

Before ecppack, check the routed output, not only the synthesis JSON:

```sh
python3 -B check_routed.py hd09.routed.json hd09.config
rg '^enum: (ALU54_[0-9]+|MULT18_[0-9]+)\.CLK0_DIV ' hd09.config | sort | uniq -c
rg '^enum: (ALU54_[0-9]+|MULT18_[0-9]+)\.CLK0_DIV ENABLED$' hd09.config
```

Expect **33 CLK0_DIV DISABLED** enums for hd09/s and **16** for hd09m/ms/p/ps;
expect **zero CLK0_DIV ENABLED**. The inactive CLK1..3 fields each have 33 or 16
ENABLED enums. The total DISABLED count is the same as in the old broken builds,
so it is not evidence by itself. `check_routed.py` also verifies the routed clock
net, the DISABLED routed parameters, and every active `REG_*_CLK` selector.

To reproduce the earlier routing-only build, copy the generated Verilog and LPF to a
scratch directory outside the repository. From that directory, for each design:

```sh
series_dir="~/IDE Workspaces/RustroverProjects/ecp5_emu/docs/inventory/LFE5U-85F/hd09_dsp_clkdiv"
yosys -q -l yosys_hd09.log -p 'read_verilog hd09.v; synth_ecp5 -top top -json hd09.json'
nextpnr-ecp5 --85k --package CABGA381 --seed 1 --json hd09.json --lpf hd09.lpf --textcfg hd09.config --write hd09.routed.json -l nextpnr_hd09.log --quiet
python3 -B "$series_dir/check_routed.py" hd09.routed.json hd09.config
ecppack hd09.config hd09.bit
```

Here `series_dir` is the absolute path to this series directory. Substitute all six
design names. The earlier validation run used `scratchpad/hd09_hs02/build.sh`, adding `--write`
for verification. That routing-only script omitted the fabric GSR override and
used nextpnr's default 12 MHz timing target. The later board-record build in
`scratchpad/hd09_hs02b/build.sh` includes the fabric GSR override. Before a board
run retain the fabric GSR override above and request the actual 25 MHz timing
constraint (`--freq 25`); neither change is needed to demonstrate the two routing
fixes. Do not load an unadjusted routing-test bitstream as a board measurement.

Check DSP GSR ENABLED, the C/opcode clocks from HD08c, and output clocks from HD07.
For hd09p, grep `REG_(INPUTA|INPUTB|PIPELINE|OUTPUT)_(CLK|CE|RST)` and verify the table
above in the X22 DSP tilegroup; identically named primitives in other tilegroups
belong to the ALUs. Keep the `.tile_group` context.

`run.py` and `hd09.lpf` are unchanged copies of HD08's. Board runner examples
(not executed during preparation):

```sh
python3 run.py hd09.bit M 3 25 > observations.log
python3 run.py hd09s.bit M 3 8 > observations_s.log
python3 run.py hd09m.bit M 3 20 > observations_m.log
python3 run.py hd09ms.bit M 3 8 > observations_ms.log
python3 run.py hd09p.bit M 3 20 > observations_p.log
python3 run.py hd09ps.bit M 3 8 > observations_ps.log
python3 -B hd09m_analyse.py observations_m.log
python3 -B hd09m_analyse.py observations_p.log --probe
python3 -B hd09_analyse.py observations.log
```

Both analyses also accept raw complete M lines. Duplicate windows are removed,
invalid/truncated M lines are counted, and empty inputs fail. Use the matching
analysis for each line length; m and p have the same layout but different stages.

## Reading

### Records and units

All three loads agree on every complete window. The free-running files contain
55 complete HD09 windows and 59 each for HD09m/p; their `s` files contain one
configuration window. Each load also contains one incomplete line, which the
parsers reject. Thus the runner's 56 HD09 lines are 55 complete windows plus a
fragment. Repeated loads are deduplicated, not counted as independent samples.
`analysis_inputs.sha256` fingerprints the untouched Verilog, configs and records.

The board configs have 33 active CLK0_DIV DISABLED enums for HD09/s and 16 for
m/ms/p/ps, with no active CLK0 ENABLED enums. The board build used
`setparam -set GSR "DISABLED" t:TRELLIS_FF` for the fabric. The build-time reports
in `validation.txt` are the earlier routing checks; the results below concern the
new board records and their configs.

### Disabled dividers require two input snapshots per cycle

The original models updated an output register only at the rising edge. The new
records instead fit this **sample-level** model:

1. Update from the inputs in sample a, before its rising edge.
2. Let fabric inputs change to those recorded in sample b.
3. Update again from b's inputs before sample b is recorded.

Call this **rise+fall**. CE and RST are evaluated at each update. The measured GSR
hold is retained: GSR active in a or b clears/holds the register for that
transition. RESETMODE=ASYNC clears an internal stage before another stage reads
it; synchronous resets act at that stage's update. Cascaded registers update
simultaneously from the preceding stage values at each update.

For a simple one-bit output with no reset or GSR, the rule is:

```text
q_mid = CE(a) ? D(a) : Q(a)
Q(b)  = CE(b) ? D(b) : q_mid
```

This is an observable model, not proof of a physical dual-edge flip-flop. The
inputs change only after rising edges and Q is sampled only before rising edges.
A suitable transparent latch during the intervening phase can produce the same
record. Falling-edge-only capture misses the rising contribution at startup and
after GSR; it comes close but is not an exact model across the records.

| Registers with CLK0_DIV DISABLED | Exact sample model |
|---|---|
| ALU R output registers | rise+fall |
| MULT18 INPUTA, INPUTB, PIPELINE, OUTPUT | rise+fall |
| ALU opcode IN_0, OP0_0/1, OP1_0/1 | rise+fall, including the configured cascades |
| ALU/MULT18 C0/C1 | rising only, **every** cycle; ENABLED used every other cycle |
| C bypass controls c4 and upper c3 | bypass ahead of the rise+fall output register |

The field masks, CE/RST selectors and reset kinds are those measured in HD07/08;
they still give exact results. The clock/update model was what changed.

### Counts

Counts below are mismatching predicted transitions / checked transitions. Direct
R/P checks count individual bits; opcode checks count R[15:0] words.

| Record | Model | Mismatches |
|---|---|---:|
| hd09ms | rise+fall, all 32 recorded R/P bits | 0 / 8,160 |
| hd09ps | rise+fall, all 32 recorded R/P bits | 0 / 8,160 |
| hd09m | rise+fall, all 32 recorded R/P bits | 0 / 481,440 |
| hd09p | rise+fall, all 32 recorded R/P bits | 0 / 481,440 |
| hd09 | rising C registers, composed with rise+fall R | 0 / 69,700 |
| hd09 | C bypass controls, composed with rise+fall R | 0 / 41,910 |
| hd09 | rise+fall opcode stages and R | 0 / 48,751 words |

For HD09's free-running windows, hidden stages start unknown. The C check excludes
150 bit predictions and the opcode check 144 word predictions while hidden state
is unknown. These are not silently counted as matches. All subsequent predicted
samples fit. The output and standalone MULT18 checks have no unknown-state gaps.

In the configuration window all observed initial R/P bits are zero. The HD09s
hypothesis that its hidden C/opcode stages also start at zero gives **0/2,032**
C/bypass bit mismatches and **0/889** opcode word mismatches, covering all 127
transitions with no unknown predictions. This is consistent with zero initial
state; R's initial zero alone would not establish each hidden stage's value.

Every direct output/stage record was also tested against:

- Rising only and falling only; both edges; every second and fourth cycle, all
  phases, with and without GSR phase reset.
- D latencies 0 through 8, while controls retain their recorded timing.
- Frozen Q, constant zero/one, combinational bypass, a CE level latch, GSR-only
  capture, and GSR clear followed by a frozen register.

The full mismatch counts are in the reports. Non-GSR changes rule out the frozen
and GSR-only hypotheses. For example, hd09m ea's four recorded bits give 19,734
rising-only mismatches and 55 falling-only mismatches out of 60,180 transitions;
rise+fall gives zero. The s windows already reject rising-only and falling-only output models when
the recorded R/P bits are considered together. Individual bits and opcode stages
can leave ties in a short window; the full records distinguish more cases.

### Reproduce the analysis

The extended analyses use Python 3 and NumPy (2.5.2 in this run). They never build
or modify Verilog/configuration files. The old ENABLED-era searches remain under
`--legacy`; the new default reports numerical mismatches for timing hypotheses.

```sh
python3 -B hd09_analyse.py observations_s.log > analysis_s.txt
python3 -B hd09_analyse.py observations.log > analysis.txt
python3 -B hd09m_analyse.py observations_ms.log > analysis_ms.txt
python3 -B hd09m_analyse.py observations_m.log > analysis_m.txt
python3 -B hd09m_analyse.py observations_ps.log --probe > analysis_ps.txt
python3 -B hd09m_analyse.py observations_p.log --probe > analysis_p.txt
python3 -B hd09_analyse.py --selftest
python3 -B hd09m_analyse.py --selftest
```

`hd09_timing.py` scores visible stages and latency hypotheses; `hd09_joint.py`
composes hidden C/opcode stages with R. Original field/control search functions
remain available for the HD07/08 regression. The extended selftests use complete
synthetic serial lines, reject the wrong edge/stage model, and preserve the older
selftests. Results are recorded in `analysis_checks.txt`.

The remaining ambiguity is physical timing between sampled edges: these records
cannot distinguish edge-triggered updates from equivalent phase transparency.
Flags are not recorded, IN_1 is absent, and CLK1..3 are not selected by active
registers. Nothing about those unmeasured cases is inferred. No board access,
synthesis, routing, cargo, rserv or commit was performed during this analysis.

## Emulator check

The event kernel and fast mode implement the measured HD09 profile in `src/compile/dsp.rs`,
`src/kernel/dsp.rs`, `src/kernel/alu.rs` and `src/kernel/fast_dsp.rs`: disabled selected dividers update
MULT18 INPUTA/B, PIPELINE, OUTPUT, ALU R and opcode stages on both edges. C0/C1
(and MULT18 INPUTC in that ALU view) update on every rising edge. ENABLED keeps
the previous behavior, including C's every-other-rise divider. CE/reset kind and
priority are unchanged. GSR active at a rising edge holds through the following
fall; its release permits the next rising-edge update. HD09ps's configured input
and pipeline data stages start at zero with GSR enabled; startup sign storage was
not recorded and remains unknown. Standalone registered INPUTC remains unsupported.

`tests/hd09_hs02_native.rs` runs the actual routed configurations from startup
at the DSP pins, before each clock edge: hd09s compares 16,384/16,384 bits,
hd09ms 8,192/8,192 and hd09ps 8,192/8,192, all equal without exclusions.
The probes include every recorded ALU R and MULT18 P bit, including C/opcode
stage effects. `tests/alu54.rs` and `tests/dsp.rs` add random both-edge capture
checks against independent models and equal fast/kernel traces, with CE/RST/GSR
traffic, default settings and `FAST_THREADS=4 FAST_THREAD_MIN=1`. The multiplier
check asserts that forced settings split a DSP clock-group settle across workers
and includes a slice FF sharing the DSP clock. Fast mode also checks the native
pin samples: hd09s and hd09ms from startup; hd09ps after its first kernel edge
loads the unmeasured sign storage (sample zero is checked by the kernel).
The hd09m/p free UART test passes four complete windows per design, including window zero
against the corresponding startup record and later windows against the free
board lines. No board bits are excluded from the kernel replay.

Mixed ENABLED/DISABLED active register clocks/cells and ALU/MULT divider
disagreement in a slice are refused as unmeasured. HD09 records physical CLK0;
selected CLKn uses the same rule. Fast captures request both polarities for the
DISABLED non-C stages; C captures request only rising edges and omit their divider.
The clock group ORs edge requests from all members, so slice FFs sharing that
clock also settle on falling edges. The extra settle is idempotent and is paid
only by DISABLED designs. GSR-at-rise storage is initialized from the kernel on
the switch to fast mode. ENABLED programs keep the previous slot allocation
and generated C:
37 existing ALU/MULT18 programs, totaling 7,384,356 bytes, compare byte-for-byte
before and after. `DSP_SOURCE_SNAPSHOT` on the synthetic tests reproduces those
C snapshots; hashes and sizes are in [enabled_c_identity.txt](enabled_c_identity.txt).
Validation commands and details are in `emulator_checks.txt`.

The optional old HD08c full UART regression was stopped after more than 20
minutes without a result; it is not claimed as passed. Existing ENABLED unit/fast
comparisons and the HD08 startup replay passed.

Validation commands and results are in `emulator_checks.txt`; the board build
commands are preserved exactly in `TOOLS.txt`. Verilog and configs were not
regenerated. `analysis_inputs.sha256` still verifies their original bytes.
