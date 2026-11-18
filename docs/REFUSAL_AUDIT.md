# Refusal audit: student designs on the ECP5 emulator

Audited `dev`, final frozen source snapshot **2026-11-17T12:11:35.148010+00:00**, HEAD `20e5d19065739c99e5dddd06da3812269ffef9a3` plus the concurrent uncommitted DSP edits present at capture. Initial experiments built HEAD `1218dc603d9dbcec0877e83b2c3a48b18df8c3ae`; **all 39 successfully routed cases were re-run in both executors against the final frozen source**. The build snapshot's sorted source-hash manifest has SHA-256 `d957af7f5cfb2f31374fea3b7deb24a4923440d9b8306a2feb6f0f3b4aa6d3e2`. Scope: every `bail!`, `ensure!`, `Err(...)`, error-producing `anyhow!`, and unsupported/unmeasured path in `src/compile/`, `src/decode.rs`, `src/decode/`, `src/prepared.rs`, `src/kernel/` (including both fast files), and `src/fast.rs`. The audit changed only this document and created no commit. HEAD advanced and DSP source/docs/tests changed during the audit; those changes were preserved. No internet was used. Experiments ran over SSH on rserv.

Follow-up at `bf47e64` (including `1c79418`): DTR now runs in fast mode; R16 is DCSC only. The appended R07/R11 comparison distinguishes plain CLI stimulus from the live ULX3S defaults and supersedes their initial likelihood estimates.

Follow-up at `b663063` (including `be71ccb`): R02's measured PDP rule now runs in fast mode. Hunk A fixes X03; selected-demand hunk B remains held. R02 retains its original rank for traceability, not as a current X03 blocker.

The biggest ordinary-RTL findings are **fast-mode failures**, not absence of the underlying resource from the event kernel. A 16×1 distributed RAM refuses an unused address wire; the earlier 512×32 inferred-RAM failure during writes is fixed by `be71ccb`. R03 native differential digital IO is now fixed; DDR memory-interface primitives remain whole-plan blockers. Ordinary async-reset FFs, 32×32 multiplication, ODDRX1F, LiteX-style SDR IO, ecppll feedback and SERV's narrow register file are supported in the tested profiles.

The historical R03 native LVDS finding was classified A **with an explicit LPF IO_TYPE**, not as a claim that Verilog alone infers an electrical standard. R03 is now fixed for input/output digital IO; differential bidir and open-drain output remain explicitly refused.

This is an audit of the **LFE5U-85F / CABGA381** model represented by the requested inventory. A different die/package is a separate target-support question. A refusal requires the resource to be in the selected plan; unused resources are not all executed just because they appear in the chip catalogue. `--scope observed` follows the probes and their dependencies; `all-plc` additionally includes the PLC FFs. Global GSR/configuration-port checks can reject even outside an observed output cone.

R15 follow-up (HY01): public ACTIVE_LOW SGSR with a routed CLK now plans and
executes in the event kernel and fast mode. X20's HDL is rebuilt in both native
image forms; the initial-state and timing qualifications are documented below.
The old rank is retained only for traceability; unmeasured polarity/clock profiles
remain explicit guards.

## How to read the ranking

**A** = inferred by behavioural RTL; **B** = an ordinary public primitive/profile, with its local source named; **C** = an explicit, unusual primitive parameter, port, combination, or raw architecture-cell profile; **D** = unavailable through the standard public-primitive pipeline, rejected by the tools first, or a command/environment error rather than a design feature; **E** = internal invariant. C rows are source audits unless an experiment is named. They are not claimed to have been reproduced. D claims are scoped to the pinned tools and their normal public primitive mappings, not arbitrary hand-authored `.config` files or fabricated blackbox declarations for internal nextpnr cells.

Priority is estimated likelihood × impact, not measured incidence in a student population. Likelihood weights: common inferred pattern 4; frequent boundary-stimulus omission 3; interface/core-dependent 2; occasional explicit primitive 1; uncommon 0.5; unusual setting 0.2 (very exotic 0.1). Impact: whole-plan block 3, fast-only block 2, particular stimulus/startup corner 1. Ties favour demonstrated, useful designs. “Fast-only” still matters for a CPU: the event kernel can be much slower. Fix sizes: **S** local qualification/diagnostic/liveness change; **M** changes across planner/executor plus regression cases; **L** a new stateful model or board measurement series. Sizes are scope estimates, not promises of elapsed time.

In the table, `Rnn` identifies a grouped refusal. The exhaustive site ledger below gives every message and source location, including guards omitted from the feature ranking. “X” below means an unknown digital value; fast mode is two-valued. An error asserting that the event kernel would make something X is a description of the fast executor's policy, not independent proof that its event implementation does so at the same intermediate instant.

## Ranked feature refusals

| Rank / ID | Refusal message (variable parts abbreviated) | File:line | Class | Minimal trigger / impact | Evidence | Likely sources; likelihood × impact | Suggested fix size |
|---|---|---|---|---|---|---|---|
| — / HC08 (**fixed**, wrong frequency) | Historical INT_OP/INT_OS fabric-return misqualification: emulator ran C/D at 100 MHz where the board runs 16.667 MHz | `src/compile/mod.rs` PLL selector/feedback planning; `src/kernel/pll.rs` | **B**, fixed | EHXPLLL INT_OP or INT_OS combined with fabric JCLKFB3 returned from the same PLL CLKOP | [HC08 A–H](inventory/LFE5U-85F/hc08_pll_analog/README.md#final-reading-run_1-and-run_2), 24 SRAM loads; `tests/pll_feedback_native.rs` checks packed A–H, kernel/fast periods and equal traces | Known masks now use undivided VCO feedback; A/B/E stay at 100 MHz | Fixed: exact joint bit-pattern qualification; other unmeasured overlaps/returns and raw-VCO live reset/standby remain refused with HC08 context |
| 1 / R01 | `fast mode reads R…:A5, which is z; fast mode reads a constant …` | `src/kernel/fast.rs:441,454` | **A** | 16×1 inferred LUTRAM; fast-only, even with all real inputs driven | X01 `lutram`: event OK, fast refuses; X02 width-4 control passes | small FIFOs, flags, register files; 4×2=8 | M: omit dead physical LUTRAM columns/inputs from fast terms |
| 2 / R02 (X03 fixed) | `fast mode stopped … logic reads an output of EBR … unknown here` (historical X03 failure) | `src/fast.rs:1259` | **A** | 512×32 synchronous read-before-write RAM, writing the read address; formerly fast-only | X03 originally event OK, fast code 2; HB09P event rule plus fast hunk A (`be71ccb`) now match over 2,048 q rows across 512 addresses; X04 WE=0 passes | student RAM/ROM exercises, caches; original priority 4×2=8, no longer an X03 blocker | Fixed for X03 by A alone; B held because the remaining masked-X case is rare after A and selected-demand checks cost every EBR design; retain unmeasured-X refusals |
| — / R03 (**fixed**, former rank 3) | Historical `differential or ambiguous-pair BASE_TYPE is not implemented` removed | `src/compile/mod.rs` (`pio`, `pio_differential`, `pio_complement`) | **A**, fixed | Ordinary FF + side-bank `IO_TYPE=LVDS`; input/output execute | X05 and built LVDS input variant: equal event/fast traces; differential aliases retain pair ownership | No longer ranked as a blocker; electrical qualification is outside the digital model | Proposed commit subject: `Execute differential PIO inputs and outputs (R03)`; uncommitted by request |
| 4 / R04 | `IOLOGIC MODDRX.MODE is not implemented; IOLOGIC MODE=… output … is not implemented` | `src/compile/iologic.rs:268,463` | **B** | default OSHX2A (also memory DDR family); whole plan | X06 `oshx2`: both refuse MOSHX2 under MIDDRX_MODDRX | nextpnr public OSHX2A / DDR memory IO list; DDR2/3 PHY use, not an inferred ordinary FF; 2×3=6 | L: memory serializers, DQS phases/pointers and tristate relationships |
| 5 / R05 | `unsupported reachable resource … (DDRDLL / DQSBUFM) output …` | `src/compile/mod.rs:1421` | **B** | DDRDLLA.LOCK, or DQSBUFM.DATAVALID with a native DDRDLL; whole plan | X07 `ddrdll`, X08 `dqs_with_dll`: both tools/pack pass and both planners refuse | nextpnr public DDRDLLA/DQSBUFM; docs/DQS_CONTROL.md and RESOURCE_COVERAGE.md; DDR PHYs; 2×3=6 | L: DLL/DQS control, FIFO pointers and validity model; no ordinary SDRAM implication |
| — / R08 (**fixed**, former rank 6) | Historical IOLDOD/OUTDEL refusal removed; WAIT_FOR_EDGE remains named | `src/compile/iologic.rs:320,333,526`; `src/kernel/iologic.rs` | **B**, fixed | OFS1P3BX → DELAYG/DELAYF → pad now executes | X12 and fabric-FF variant have equal event/fast traces; [HI11](inventory/LFE5U-85F/hi11_output_delay/README.md) proves output data and DELAYF startup on left/bottom banks | No longer ranked as a blocker; right/top startup assumed by IOLOGIC kind | Fixed: zero-time IOLDOD, output CFLAG, IOLOGIC startup DEL_VALUE & 0x3E / SIOLOGIC 0; WAIT_FOR_EDGE still refused |
| — / R09 (**fixed**, former rank 7) | Historical ECLKSYNC MODE NONE / bypass blanket refusals removed | `src/compile/eclk.rs:12,63`; `src/compile/mod.rs:1944` | **B**, fixed | Default CLKDIVF now executes through qualified MODE NONE with unrouted STOP | X13 and pin/PLL variants have equal kernel/fast traces; [HI12](inventory/LFE5U-85F/hi12_eclksync_none/README.md#reading) confirms the pin-clock NONE buffer at /2 and /3.5 | No longer ranked as a blocker; connected STOP remains refused | HI13 closes enabled pin-clock startup: stages 0 for primary pin/constant-enabled DCC paths, 1 for delayed/fabric paths; PLL/OSCG history unmeasured |
| 8 / R10 | `unsupported reachable resource … (JTAGG) output JTDI` | `src/compile/mod.rs:1421` | **B** | default JTAGG with a used fabric-side output; whole plan | X14 `jtag`: both planners refuse JTDI | nextpnr public JTAGG; local docs/PACKAGE_JTAG.md; fabric JTAG/debug bridges (no such core vendored here); 1×3=3 | L: TAP, user scan chains and external JTAG stimulus |
| 9 / R11 | `fast mode is two-valued; input external:… starts at z; this input cannot be set to Z` | `src/kernel/fast.rs:881,3016` (bf47e64; initial-Z path is :3016) | **A** | ordinary input FF with an undriven PULLMODE=NONE input; fast-only environmental stop | X15 refuses; F05 repeats on a real GPIO with explicit PULLMODE=NONE; F04 omitted PULLMODE and stock/live buttons pass | occasional new CLI GPIO with explicit NONE and no stimulus; default live buttons/UART/switches already driven; 2×1=2 | S: supply the known external/board level or an electrically justified LPF pull; no arbitrary zero for a genuinely floating input |
| 10 / R06 | `fast mode does not run pll:… with RST/STDBY enabled (HC07)` | `src/kernel/fast.rs:2935` | **B** | EHXPLLL explicitly setting PLLRST_ENA="ENABLED" or STDBY_ENABLE="ENABLED", with its control connected to logic/input; X09 holds RST at 0; fast-only | X09 hand-instantiated explicit-reset profile: event OK, fast refuses before any reset pulse; not the default LiteX profile | Lattice IP/Clarity-generated PLL profiles or hand instantiations that explicitly enable RST/STDBY; default LiteX ECP5PLL runs in fast mode; 1×2=2 | M: schedule measured reset/standby/recovery in fast host |
| 11 / R12 | `fast mode runs pll:… only from a clock input; its reference comes from logic` | `src/kernel/fast.rs:2944` | **B** | ecppll fed by a behavioural divide-by-two FF; fast-only | X17 `pll_logic_reference`: event OK, fast refuses | clock experiments, fabric-derived PLL references; 1×2=2 | M: feed host PLL scheduler from generated/reference signal edges |
| 12 / R13 | `fast mode stopped … logic reads an output of EBR … unknown here` | `src/fast.rs:1259` | **A** | two independently clocked inferred RAM ports write one row simultaneously with different data; stimulus corner | X18 `ram_collision`: event ends X, fast stops at 20000 ps; stock SERV control passes | dual-port RAM/CDC experiments; **not** the fixed SERV same-clock READBEFOREWRITE case; 2×1=2 | M/L if hardware-defined cases can be added; otherwise this is intentional X handling |
| 13 / R14 | `pad … is driven by the design and from outside with different levels` | `src/fast.rs:1242` | **A** | inferred inout with pull; external 0, design switches from Z to 1 at 90 ns; stimulus corner | X19 `pad_conflict_dynamic`: event X, fast stops at 90000 ps | bus turnaround, I²C/testbench mistakes; 2×1=2 | S diagnostics; do not invent a binary result for contention |
| — / R15 (**fixed**, former rank 14) | Historical blanket SYNC user-GSR refusal removed | `src/compile/system.rs:99-163` | **B**, fixed | Default ACTIVE_LOW SGSR with routed CLK now plans and executes | X20 HDL rebuilt as config/bit; equal event/fast traces; [HY01 and hy01b/c](inventory/LFE5U-85F/hy01_sgsr/README.md) qualify the rule and startup | No longer ranked as a blocker; non-ACTIVE_LOW SYNC and missing CLK routes remain refused | Fixed: raw active OR plain two-stage pipeline initially 11; no resets/preset; propagation delay zero, captures within 20 ns are timing hazards |
| 15 / R07 | `fast mode refuses a pad that can float with no pull while the logic reads it` | `src/kernel/fast.rs:2084` (bf47e64) | **A** | `assign pad=oe?d:1'bz; assign q=pad;` + PULLMODE=NONE; fast-only | X10 refuses; X11 with pull-up passes; follow-up F12 bus reproduces in CLI/live, including with binary external --set levels | uncommon added bidirectional bus with no real pull/driver; stock button LPF does not trigger it; 0.5×2=1 | M: check actual unknown reads rather than float capability; use a real LPF pull/device model where justified; keep genuine X reads refused |
| 16 / R16 | `fast mode does not run DCSC (HC02); the event kernel does` | `src/kernel/fast.rs:339` (bf47e64) | **B** | HC02-style DCSC clock mux; fast-only | X22 `dcsc_hc02`: full local HC02 fixture freshly rebuilt; DTR supported since 1c79418, no longer part of R16 | nextpnr public DCSC; local HC02 benches; 0.5×2=1 | M: DCS switch history and host scheduling |
| 17 / R17 | `reference period changed … relock is not modelled; reference period is not a whole number of fast-mode steps; reference/RST/STDBY is not binary` | `src/kernel/pll.rs:214,304,331; src/fast.rs:857,871` | **B** | ecppll with independent reference stimulus; vary its period, or introduce an unknown control/clock transition; clock/stimulus corner | X23 `pll_changed_ref`: event refuses period change at 110000 ps; fast first refuses the independent reference tick grid | clock switching/relock labs; independent external PLL references in fast mode; 1×1=1 | M/L: refine tick grid for secondary references; model relock; binary-input diagnostics are intentional |
| 18 / R18 | `CLKDIVF … GSR=ENABLED is not measured` | `src/compile/eclk.rs:111` | **C** | explicit CLKDIVF.GSR="ENABLED"; whole plan | `clkdiv_gsr` reproduced; normal nextpnr default is DISABLED | unusual enabled global reset on an edge-clock divider; 0.2×3=0.6 | M + board qualification |
| 19 / R19 | `IOLOGIC GSR=… / SRMODE=… / clock enable CE… is not measured for MODE=…` | `src/compile/iologic.rs:280,284,321` | **C** | explicit enabled GSR on X2/7:1 gearbox; raw DDR CE or synchronous reset profile; whole plan | packer defaults DDR GSR to DISABLED, X1 ENABLED is supported; CE is forced to 1 for public DDR cells | custom gearbox/reset/architecture-cell experiments; 0.2×3=0.6 | M/L per measured profile; ordinary LiteX SDR CE/reset already works |
| 20 / R20 | `MULT18 C/dynamic/highspeed source mode is not implemented; uses unknown/nonparallel SOURCEA/B; unsupported MULT18 output …` | `src/compile/dsp.rs:110,228; src/kernel/dsp.rs:574; src/kernel/fast_dsp.rs:452` | **C** | explicit SOURCEB_MODE other than B_SHIFT, nonzero SOURCEA/B, or SROA/SROB/ROC observation; whole plan/runtime | Yosys dsp_map_18x18 hardwires SOURCEA/B=0 and uses ordinary P outputs | custom DSP chains; no inferred multiply/MAC maps to these source modes; 0.2×3=0.6 | L: shift/C/high-speed forwarding, cascade contracts |
| 21 / R21 | `MULT18 unsupported/unresolved INPUTC stage; registered MULT18 SIGNEDP is unresolved D01/D05` | `src/compile/dsp.rs:166,222` | **C** | standalone registered C without ALU, or observing SIGNEDP after product OUTPUT register; whole plan | normal inference creates no internal DSP register stage; external FFs remain fabric FFs | explicit MULT18X18D register/sign experiments; 0.2×3=0.6 | M/L: qualify C view and measured SIGNEDP output-stage behavior |
| 22 / R22 | `ALU54B REG_OPCODEIN_1_CLK=… is not understood … 6% of the clocks` | `src/compile/dsp.rs:592` | **C** | explicit second IN opcode stage on CLK0..3; whole plan | HD08 o4/o7; default NONE passes | custom ALU54B pipelines; Yosys synth_ecp5 does not infer ALU54B; 0.2×3=0.6 | L: resolve board behavior before implementing |
| 23 / R23 | `ALU54B output registers on two clocks / REG_OUTPUT0_RST and REG_OUTPUT1_RST differ / C RESETMODE differs` | `src/compile/dsp.rs:620,739,753` | **C** | ALU output groups on distinct clocks/reset lines or ALU/MULT C reset mismatch; whole plan | explicit ALU/MULT shared-slice profiles only | custom ALU54B; local fir_scope uses matched measured settings; 0.2×3=0.6 | M/L per clock/reset measurement |
| 24 / R24 | `ALU54B … is not measured (only …); unsupported ALU54 output … (CO is not measured); registered flags beside CIBOUT_BYP …` | `src/compile/dsp.rs:638,803,845` | **C** | dynamic MASKPAT/MCPAT, LEGACY/force-zero barrel enabled, CO output, or flags beside bypassed second MULT; whole plan | four nondefault settings listed in settings_coverage.csv; normal measured ALU profile passes | explicit arithmetic/pattern/flag IP; 0.2×3=0.6 | L: mode/pattern/flag/carry measurements |
| 25 / R25 | `mixed ENABLED/DISABLED DSP … not measured (HD09); ALU/MULT18 CLK_DIV disagreement …` | `src/compile/dsp.rs:411,452,478,483,501` | **C** | different enabled/disabled divider profiles across active stages/cells/shared slice; whole plan | default all ENABLED; Yosys inference has no such registered stages | explicit DSP register configurations; 0.2×3=0.6 | M/L: per-stage scheduling and shared divider evidence |
| 26 / R26 | `36-bit PDP read with REGMODE_A != REGMODE_B is not measured; EBR … active/unknown local reset with unsupported synchronous release; EBR output … outside validated data view` | `src/compile/ebr.rs:115,162; src/kernel/ebr.rs:416; src/fast.rs:1265` | **C** | explicit DP/PDP mismatched 36-bit reg modes; RESETMODE=ASYNC with ASYNC_RESET_RELEASE=SYNC and asserted reset; high output bit outside configured width | mapper uses matched NOREG and ASYNC_RESET_RELEASE=OPTION_RESETMODE; inferred async RAM control passes | hand-instantiated block RAM; 0.2×3=0.6 | M + board qualification; keep runtime rather than unconditional reset refusal |
| 27 / R27 | `PLL … setting=value is not implemented (only expected); output … disabled; user feedback is not implemented; output REFCLK … not implemented` | `src/compile/mod.rs:1478,1557,1572,1598` | **C** | dynamic phase (LiteX: explicit expose_dpa())/sync/ref-loss reset/internal wake; crossed/REFCLK divider mux; trim; ENCLK-gated disabled output; user feedback; REFCLK observation | normal ecppll and default LiteX INT_O* feedback are accepted; full field list below | unusual PLL options; 0.2×3=0.6 | M/L: independent features, not one PLL rewrite |
| 28 / R28 | `IOLOGIC latch is not implemented / latch measured only with …; IOLTOMUX TDDR …; MODE/output … not implemented` | `src/compile/iologic.rs:348,361,378,435,463` | **C** | raw IOLOGIC TSREG latch/TDDR; IO latch with wrong reset/GSR/REGSET; unsupported combined paths | public nextpnr FF packing writes FF, not LATCH; behavioural latch rejected by default synth_ecp5 | hand-authored internal architecture-cell profiles; 0.1×3=0.3 | M/L; low priority for ordinary student RTL |
| 29 / R29 | `… ENABLE retains package … user IO is not supported / ownership conflicts with configured USRMCLK; custom wake phase … unsupported; unsupported wake sequence …` | `src/compile/config_ports.rs:146,187; src/compile/system.rs:119,127` | **C** | explicit SYSCONFIG port retention while using its balls/USRMCLK; custom startup phases with --startup wake-phases | nextpnr writer accepts sysconfig enums; checks are global; configured startup ignores custom wake phases | custom board startup/configuration settings; 0.1×3=0.3 | M/L for ownership/startup transport; conflicts are normally correct to reject |
| 30 / R31 | `fast mode does not run ALU54B … OP can read R, and some bits of R are unregistered` | `src/kernel/fast_dsp.rs:1024` | **C** | explicit bypassed ALU result with OP selecting feedback R; fast-only / real combinational feedback | event kernel makes the problematic bits X; HD03/04 | custom ALU54B; 0.1×2=0.2 | M/L; retain refusal for a real unresolved combinational loop |
| — / R32 | `fast mode is two-valued; … is unknown (StartsUnknown)` | `src/kernel/fast.rs:414,1182,1596,2924,3193; src/kernel/fast_dsp.rs:252,306` | **A** | inferred FF/RAM not yet known under ready-unknown; usually a transient retry, not final CLI failure | X16 warms one FF cycle and runs; DSP-only instances of this error are C profiles | CPU reset/warmup, uninitialized storage; 2×1=2 but normally self-recovers, so excluded from blocking priorities | S diagnostics/warm start; no fabricated initialization |
| — / D | Invalid enum/mode/file/reference/API/environment refusals (individual messages below) | exhaustive ledger | **D** | Public tool mapping rejects first, or invalid command/state rather than HDL | source audit and negative synthesis/route controls | no normal student feature impact; pathological depth caveat below | no feature fix; S diagnostics where useful |
| — / E | Internal invariants (individual messages below) | exhaustive ledger | **E** | Catalogue/owner/handle/cache/timer/word invariants | fixed metadata and planner contracts | skipped from student priorities because a bug is required | fix only if the invariant actually fails |

**Concurrent change:** R30, the former fast `CLK_DIV=DISABLED` refusal, was removed while this audit ran. The final source schedules both-edge DSP captures. It is excluded from current priorities; mixed profiles still refuse at R25. This audit did not independently repeat the HD09 hardware/equivalence series for that new implementation. Several rows share one generic error site; the ledger groups those duplicates without treating them as additional independent gaps.

## A. Inferred RTL

With ordinary LVCMOS IO, the demonstrated behavioural-RTL blockers affect fast mode or a particular input/collision condition. Native LVDS requires the LPF IO_TYPE constraint and now executes under the R03 digital pair model. An async reset to a constant is supported. The default toolchain rejects an inferred latch and a nonconstant asynchronous load before producing a bitstream; the corresponding emulator latch/PRLD checks must not be presented as ordinary inferred-RTL gaps. `*` maps to parallel unregistered MULT18X18D with SOURCEA/B tied low, plus fabric logic/FFs. A large multiply does not automatically request the unimplemented DSP preadder, shift/C source modes, or ALU54B opcode registers.

The 16×1 LUTRAM refusal is shape-dependent: the otherwise equivalent width-4 RAM passes. It is the dead physical columns/terms that need attention, not a blanket absence of LUTRAM. The original PDP36 failure depended on writing; `be71ccb` now fixes that measured profile in fast mode as well as the event kernel. The independent two-port collision case still really leaves X in the event trace; treat it separately from the fixed PDP36 fast/event discrepancy.

An input left floating is an environment problem unless a modelled device supplies its level. A tri-state pad with a pull runs. A pad that can float without one is refused when read by logic, even when the particular initial OE happens to drive it. Initial contention alone did not stop the tested fast run; the dynamic 90 ns contention did. That last observation is a limitation of this refusal audit, not a claim of general fast/event equivalence.

## B. Ordinary primitives and local core evidence

“B” does not mean Yosys infers the primitive. It means a public primitive used in its ordinary profile, rather than an exotic parameter. Local primary sources are the installed Yosys lattice cell library and the pinned nextpnr `ecp5/docs/primitives.md`, `pack.cc`, and `bitstream.cc`, together with repository examples and board reports. DDR/JTAG applicability is to the named primitive; no full LiteX/LiteDRAM/JTAG core was fetched or claimed to have been tested.

- **ecppll:** `examples/ulx3s/blinky_pll/pll.v`, `hdmi_bars/pll.v`, and HC05 show ordinary static feedback; that profile passes. Internal `FEEDBK_PATH=INT_OS3`, `.CLKFB(fb), .CLKINTFB(fb)` is explicitly supported after HC07. R06 requires PLLRST_ENA or STDBY_ENABLE explicitly set to ENABLED; X09 is a hand instantiation with PLLRST_ENA="ENABLED" and RST held at zero, not output from ecppll or the default LiteX wrapper.
- **LiteX:** the [upstream ECP5PLL source](https://github.com/enjoy-digital/litex/blob/master/litex/soc/cores/clock/lattice_ecp5.py) (master, read 2026-11-17) connects `i_RST=self.reset` and `i_STDBY=self.stdby`; `PLLRST_ENA` and `STDBY_ENABLE` are **not set**. It sets `FEEDBK_PATH=INT_O*`, dividers, output enables and phases. The pinned nextpnr writer defaults both enables to DISABLED (`ecp5/bitstream.cc:1268,1273`); [HC07](inventory/LFE5U-85F/hc07_pll_reset/README.md) measured that disabled RST/STDBY inputs are ignored, and the planner omits them from the fast PLL control list. **Default LiteX PLLs therefore run in fast mode and do not hit R06.** X09 explicitly enables reset and does not represent that default profile. LiteX sets DPHASE_SOURCE="ENABLED" only after an explicit `expose_dpa()` call; that opt-in feature hits R27. Local HI05 documents `IFS1P3*`/`OFS1P3*` SDR IO with CE, synchronous/asynchronous reset and enabled GSR; the small OFS1P3BX test passes. These sources do not establish every LiteX core. DDR2/3 PHYs using the named DQS/DLL/memory-DDR primitives face R04/R05; ordinary SDR SDRAM does not require them.
- **SERV:** the vendored `examples/gallery/serv_shell/serv/serv_rf_ram.v` is SERV 1.4.0, width 2/depth 576. It maps to narrow READBEFOREWRITE DP16KD. The fresh stock-source wrapper passes in both executors. The old narrow-RBW gap and same-clock cross-port collision were fixed (gallery README records `847a9ef`, `04268bb`, `8f3673e`). Some prose in `docs/EBR_EXECUTION.md` still describes the earlier fast failure; it is not current evidence of a gap.
- **PicoRV32:** the vendored core and `picorv32_console`, `riscv_shell`, flash/SDRAM examples use inferred LUT/FF/RAM and optional multiplication. Large ordinary multiplication passes this audit's 32×32 control. Its presence does not imply ALU54B or DSP shift modes. This audit did not rebuild and boot every whole CPU example.
- **VexRiscv:** no vendored VexRiscv RTL or specific local implementation was found in examples/docs. No network was allowed. There is no experimental basis here to assert that a particular VexRiscv configuration runs or fails; inferred memories/multipliers inherit their own demonstrated classifications. A LiteX CRG using the default ECP5PLL profile does not introduce R06.
- **ODDRX1F:** local `ddr_pattern` / HDMI examples use it with LVCMOS33D, which is supported. Native LVDS and other differential input/output BASE_TYPE values now execute (R03). Explicit X1 GSR=ENABLED is supported; the remaining gearbox GSR restriction applies to X2/7:1, not X1.

## C. Rare settings

These are real coverage limits, but plain `synth_ecp5` does not infer the unusual modes. They require explicit primitive parameters/ports or raw IOLOGIC profiles. Source-level reachability is stated separately from experiments: C is not a claim that every unusual combination routes on this package. Some checks are shadowed by an earlier guard, and the first refusal can change with which output is probed.

The PLL loop at `src/compile/mod.rs:1463–1483` checks exactly DPHASE_SOURCE=DISABLED, SYNC_ENABLE=DISABLED, REFIN_RESET=DISABLED, INTFB_WAKE=DISABLED; OUTDIVIDER_MUXA/B/C/D=DIVA/B/C/D; CLKOP/CLKOS_TRIM_POL=RISING; CLKOP/CLKOS_TRIM_DELAY=0. It is not a refusal of noninteger PLL ratios or of all phase offsets. Static CPHASE/FPHASE, all four output clocks, LOCK, INTLOCK, and CLKINTFB are modelled. Disabled static outputs are refused only when used; their dynamic ENCLK behavior is not implemented. In LiteX, DPHASE_SOURCE remains at nextpnr's DISABLED default unless the user explicitly calls `expose_dpa()`, which sets it to ENABLED; this R27 refusal is not part of the default ECP5PLL profile.

For DSPs, `synth_ecp5`'s `dsp_map_18x18.v` does not infer ALU54B or internal DSP pipeline stages; the 32×32 control generated four MULT18X18D and 53 CCU2C. `examples/gallery/fir_scope/dsp_mac.v` is an explicit ALU/MULT design in a measured profile. Its comments deliberately use constant OP=11'h25d to avoid reading floating cascade inputs. A runtime OP may read more sources and hit the same generic unknown-net guard as R01; that is an explicit DSP profile, not evidence that ordinary inferred multiplication fails.

## D. Toolchain, command, and environment boundaries

Normal public primitive mappings never emit arbitrary enum strings. Invalid widths, clock/reset/CE names, inactive live modes, or malformed configuration/catalogue data must not be counted as missing executable features. This includes the complete implemented two-value enums (FF reset modes, CE, REGSET, GSR, EBR write modes, DCS modes). The ledger retains each error and explains its boundary.

Default `synth_ecp5` rejects the behavioural latch at PROC_DLATCH and rejects a nonconstant asynchronous load at FF legalization. Default nextpnr rejects the tested combinational loop at timing analysis. Its `--ignore-loops`, manual raw-cell blackboxes, edited CRAM, or changing the kernel's delta budget can change those boundaries; they are not evidence that a normal student circuit needs a latch/loop model. Very deep acyclic circuits can also exhaust a finite delta budget: that pathological size limit is not proven impossible by a loop check. No exhaustive maximum-depth place-and-route experiment was attempted, and the ledger flags that caveat rather than calling it an invariant.

The fixed LFE5U model cannot execute UM/UM5G SERDES merely because DCUA/EXTREFB/PCSCLKDIV exist in a generic nextpnr primitive list. PRADD, MULT9 and ALU24 are architectural inventory entries without ordinary `synth_ecp5`/nextpnr mappings. EFB/SED and DLLDELD inventory presence likewise is not proof of a usable public mapping in this pinned writer. A manual internal-wire probe can reach an unsupported producer without a student design ever using it.

The “one clock” error in `src/fast.rs:697` is one host `clock()` call, not one clock domain in the design. Fabric-divided FF clocks, PLL outputs and OSCG can coexist. Missing clang, clang/dlopen/symbol failures, cancellation, past timestamps, duplicate drivers, and bad references are not HDL feature gaps. Fast time-grid restrictions for a *secondary externally driven PLL reference* are real and are demonstrated in R17; they must not be dismissed with the command-only checks.

## E. Internal invariants skipped from feature priority

The ledger marks these **E** and gives the exact message/location and why they were skipped: catalogue structure and owner identities, fixed topology cardinalities, cache payload consistency, kernel-handle scope and ownership, duplicate execution owners, impossible register widths/values, generated-event ordering, timer/controller state, and test-only benchmark comparisons. A shipped database/catalogue bug or a Rust API caller bug could reach them; ordinary valid RTL and the pinned writer cannot request those states. They are not evidence of unsupported student features. Malformed user files/commands remain D rather than being hidden in this skip group.

`Err(e)` propagation and match arms are listed separately at the end. They are not fresh refusal sites. The Err values used to represent a dynamic C expression in `fast_dsp.rs` are successful generator control flow, not failures. Tuner “not measured” text means that a performance candidate has not been timed; it is not a design refusal.

## Experiment protocol and results

Scratch sources, configurations, bitstreams and complete logs were kept at `/tmp/ecp5-refusal-audit` on this Mac and rserv, outside the repository. The rserv checkout was synchronized; it built the current emulator with `cargo build --release --offline -p ecp5_emu --bin ecp5_emu`. Subsequent batches used the same PATH/reader environment over SSH, avoiding repeated fixture transfers. No code/test/source changes were made to the checkout. This is CLI/planner execution, not a new hardware measurement.

Verified tools: Yosys **0.70**, SHA `28ba3cb92`; nextpnr-ecp5 **0.11.1-77-g6b438a1c**, full SHA `6b438a1c9ab83cfb872fea21d9545d7d0bb8573f`; ecppack `~/FPGA/trellis-install/bin/ecppack`; native image reader `~/work/bin/trellis_image_reader`. Installed local nextpnr source matches that SHA. Source inspection used `/opt/homebrew/share/yosys/lattice/` and `~/FPGA/nextpnr/ecp5/`; no external site was opened.

Each experiment ran:

```sh
yosys -Q -T -p 'read_verilog top.v; synth_ecp5 -top top -json top.json'
nextpnr-ecp5 --85k --package CABGA381 --json top.json --lpf top.lpf \
  --lpf-allow-unconstrained --textcfg top.config --write routed.json \
  --seed 1 --freq 25 --timing-allow-fail
ecppack --compress top.config top.bit
target/release/ecp5_emu emulate /tmp/ecp5-refusal-audit/CASE/top.config \
  --names /tmp/ecp5-refusal-audit/CASE/routed.json \
  --startup configured --scope observed --probe net:q \
  --clock net:clk --period-ns 40 --cycles 10 [--set net:INPUT=0|1] [--stimulus FILE]
# Repeat the same command with --fast, FAST_CHOOSE=0 FAST_TUNE=0.
```

All final routed experiments explicitly LOCATE every IO to a real CABGA381 ball, despite the harmless retained `--lpf-allow-unconstrained` switch. `clk` is G2. One extra `hb` output FF keeps `clk` present in otherwise combinational tests; it is not probed. Except the cases explicitly noted below, IO is LVCMOS33/PULLMODE=NONE. Buses are initialized with separate `--set net:a[i]=0|1` options, never decimal values. The planner was run on `.config`; ecppack success proves that a `.bit` was also produced, not that every case was re-run from `.bit`. Timing was allowed to fail so a pin-placement timing miss did not masquerade as an emulator refusal; no `--ignore-loops` was used.

The first setup attempts used invalid `IOBUF ALLPORTS` syntax, then unconstrained IOs (which nextpnr can place on unbonded sites); those failures were corrected and are not used as feature evidence. LVDS on top-bank A16 failed ecppack because OUTPUT_LVDS is unavailable there; the final LVDS test uses side-bank A2 and packs successfully. Direct tiny DCSC/IO topologies hit a nextpnr routing assertion; a reduced HC02-style wrapper also failed. The final successful DCSC experiment rebuilds the **full** local HC02 fixture; it is identified explicitly rather than pretending the reduced wrapper routed. A DQS input with DDRDEL tied to fabric GND failed routing; the final test connects a native DDRDLLA.DDRDEL.

The final source was built independently under `/tmp/ecp5-refusal-audit/current_build` with `CARGO_TARGET_DIR=/tmp/ecp5-refusal-audit/current_target`; docs/catalogues point at the synchronized evidence tree. Source and required `include_str!` assets were copied, not edited. Initial isolated-build attempts lacked two included assets; after copying them the release build passed. All results below refer to the final-source rerun unless they are explicitly tool-stage negatives. No complete CPU boot or silicon equivalence is inferred from ten cycles.

### Final-source execution matrix

All X01–X35 passed Yosys, nextpnr and ecppack. Hashes are SHA-256 of the routed `.config` (shown as a 16-hex identifier here; the full hash for each principal A/B trigger appears with its RTL below). “OK” means the ten-cycle command completed, not that arbitrary future stimulus is supported.

| ID / scratch case | Conditions | Event kernel | Fast executor | Config hash prefix |
|---|---|---|---|---|
| X01 / `lutram` | narrow distributed RAM, WE=1 A=0 D=1 | OK | refused: Error: fast mode reads R2C103:A5, which is z | `fceea405fb3cfddd` |
| X02 / `lutram4` | same RAM, four data bits | OK | OK | `d44073decb2863c7` |
| X03 / `ram32` | 512×32 RAM, WE=1 A=0 D=1 | OK | Original snapshot: EBR-unknown refusal at 0 ps. Fixed by `be71ccb` hunk A; the fast X03 twin matches the event kernel over 2,048 q rows across 512 addresses | `d4c4d132184401c8` |
| X04 / `ram32_idle` | same RTL/image, WE=0 | OK | OK | `d4c4d132184401c8` |
| X05 / `lvds_side` | Q=A2; native LVDS, other pins LVCMOS25 | **Fixed R03:** P=data, N=complement | **Fixed R03:** equal trace with event kernel | Original `383e325821207321`; rebuilt in `tests/r03_differential.rs` |
| X06 / `oshx2` | Q=A2; D=0 E=1 | refused: IOLOGIC MODDRX.MODE is not implemented: LFE5U-85F/R35C0/IOLOGICA (MODE MIDDRX_MODDRX, candidates Some(["MOSHX2"])) | refused: IOLOGIC MODDRX.MODE is not implemented: LFE5U-85F/R35C0/IOLOGICA (MODE MIDDRX_MODDRX, candidates Some(["MOSHX2"])) | `feb09e350c202248` |
| X07 / `ddrdll` | default DDRDLLA LOCK | refused: unsupported reachable resource LFE5U-85F/R0C0/DDRDLL (DDRDLL) output LOCK | refused: unsupported reachable resource LFE5U-85F/R0C0/DDRDLL (DDRDLL) output LOCK | `c754eb9bcadda289` |
| X08 / `dqs_with_dll` | DQSI=E4; DDRDEL from DDRDLLA | refused: unsupported reachable resource LFE5U-85F/R17C0/DQSBUF (DQSBUFM) output DATAVALID | refused: unsupported reachable resource LFE5U-85F/R17C0/DQSBUF (DQSBUFM) output DATAVALID | `aefbbb22d1d1a5d9` |
| X09 / `litex_pll_reset` (historical scratch name; hand instantiation) | RST=0, explicit PLLRST_ENA="ENABLED"; INT_OS3 wiring; not the default LiteX profile | OK | refused: Error: fast mode does not run pll:LFE5U-85F/R4C1/EHXPLL_UL with RST/STDBY enabled (HC07); the event kernel does | `a3d4dc6bc0317d1c` |
| X10 / `tristate_float` | pad=A16, no pull; OE=0 D=1 | OK | refused: Error: fast mode refuses a pad that can float with no pull while the logic reads it (R0C110:PAD_A); the event kernel makes that read X | `281fafbabd8e3172` |
| X11 / `tristate_pull` | same RTL, pad pull-up | OK | OK | `2534f22322b7f09f` |
| X12 / `output_delay_sdr` | Q=A2, OFS1P3BX→DELAYG; R08 fixed, HI11 | OK (equal output-delay traces) | OK (equal output-delay traces) | `bdb028b6c3506d94` |
| X13 / `clkdiv_default` | default CLKDIVF; R09 fixed, HI12 | OK (qualified NONE buffer; equal traces) | OK (qualified NONE buffer; equal traces) | `bf8f700c2f0162ca` |
| X14 / `jtag` | used JTDI | refused: unsupported reachable resource LFE5U-85F/R94C4/JTAGG (JTAGG) output JTDI | refused: unsupported reachable resource LFE5U-85F/R94C4/JTAGG (JTAGG) output JTDI | `1a6aa70f90d0c45e` |
| X15 / `unknown_input` | D unspecified, no pull | OK | refused: Error: fast mode is two-valued; input external:LFE5U-85F/R0C63/PIOB starts at z | `b1cd69647291f5bb` |
| X16 / `warm_ff` | D=1 RST=0; ready-unknown | OK | OK; warm-start used | `2cc47a280d499626` |
| X17 / `pll_logic_reference` | ecppll reference from toggle FF | OK | refused: Error: fast mode runs pll:LFE5U-85F/R4C1/EHXPLL_UL only from a clock input; its reference comes from logic | `74bad5d1c3525b72` |
| X18 / `ram_collision` | both clocks rise at 20 ns; addresses 0, data 1/2 | OK | refused: Error: fast mode stopped at 20000 ps: the logic reads an output of EBR LFE5U-85F/R22C64/EBR3 that the event kernel makes unknown here (a cross-port collision on one row, a control combination the board has not shown, or a read of unknown memory); fast mode is two-valued, so run this with the event kernel | `8e80d1696db79055` |
| X19 / `pad_conflict_dynamic` | pull-up; PAD=0, D=1; OE 0→1 at 90 ns | OK | refused: Error: fast mode stopped at 90000 ps: pad R0C110:PAD_A is driven by the design and from outside with different levels; the event kernel makes it X | `2534f22322b7f09f` |
| X20 / `sgsr` | RST=1, D=1; HY01-qualified SGSR | OK: holds first two rising edges, captures at edge 3 | OK: equal settled trace; config and bitstream forms | Original `2a5b58eb2b1a9eec`; HDL rebuilt in `tests/sgsr_native.rs` |
| X21 / `dtr` | STARTPULSE=0 | OK | Supported since 1c79418; the original snapshot failure is obsolete. Follow-up validation: 7 DTR kernel tests and 2 native tests pass | `8fd08ca535ecf7d6` |
| X22 / `dcsc_hc02` | full HC02 dynamic fixture, only ports renamed clk/q | OK | refused: Error: fast mode does not run DCSC (HC02); the event kernel does | `see fixture` |
| X23 / `pll_changed_ref` | secondary CLKI rises 20,60,110 ns | refused: Error: event kernel at 110000 ps: pll:LFE5U-85F/R4C1/EHXPLL_UL reference period changed at 110000 ps (expected 100000 ps); relock is not modelled | refused: Error: pll:LFE5U-85F/R4C1/EHXPLL_UL reference period is not a whole number of fast-mode steps | `1935c9c7a5240a2f` |
| X24 / `async_ff` | constant asynchronous reset deasserted; D=1 | OK | OK | `2cc47a280d499626` |
| X25 / `ram_async` | inferred EBR async reset asserted; WE=0 | OK | OK | `22e9b4e29b158e95` |
| X26 / `multiply32` | 32×32 A=3 B=7, all product bits used | OK | OK | `9a350abd68a410b0` |
| X27 / `oddrx1` | Q=A16; D=0 E=1 RST=0 | OK | OK | `96ae7177c13b008a` |
| X28 / `sdr_ioreg` | Q=A16; EN=1 RST=0 D=1 | OK | OK | `b363032df3adfbba` |
| X29 / `ecppll` | unmodified local ecppll module | OK | OK | `4f0ca316cf97a1bc` |
| X30 / `gsr` | asynchronous GSR input deasserted | OK | OK | `06d2df8a04e9b920` |
| X31 / `serv_rf` | stock width-2/depth-576 RF; read/write address 0 | OK | OK | `89c456deee171c80` |
| X32 / `clkdiv_gsr` | explicit GSR=ENABLED | refused: CLKDIVF LFE5U-85F/R46C0/CLKDIV1 GSR=ENABLED is not measured | refused: CLKDIVF LFE5U-85F/R46C0/CLKDIV1 GSR=ENABLED is not measured | `149d4dc0592c6141` |
| X33 / `osc3` | OSCG DIV=3 | OK | OK | `17b35b73d78a8f0d` |
| X34 / `sstl` | Q=A2 SSTL18_I with LVCMOS18 other pins | OK | OK | `f7e8d919a90d8502` |
| X35 / `output_delay_ff` | fabric FF→DELAYG; already passed through fabric routing, now explicitly plans/checks IOLDOD (R08) | OK | OK | `638d67e38ccfba06` |

Additional PLL variants reused X09/X23's already routed configuration; X09 has explicitly enabled reset and is not a default LiteX PLL. A periodic 48 ns secondary reference is on the fast tick grid; changing its next period fires the **period-change** refusal in both engines. A 40 ns or 160 ns independently stimulated secondary reference instead first fires the fast **tick-grid** refusal. These are separate paths, not a blanket rejection of all rational PLL ratios. An initial unknown PLL control/reference can remain unknown without an immediate event error; a **0→X transition** at 90 ns hits the binary-control/reference checks. Fast CLI rejects the X stimulus before executing it.

| Variant | Event result | Fast result |
|---|---|---|
| `pll_grid_period_change` | refused: Error: event kernel at 124000 ps: pll:LFE5U-85F/R4C1/EHXPLL_UL reference period changed at 124000 ps (expected 116000 ps); relock is not modelled | refused: Error: pll:LFE5U-85F/R4C1/EHXPLL_UL reference period changed; relock is not modelled |
| `pll_reset_transition_x` | refused: Error: event kernel at 90000 ps: pll:LFE5U-85F/R4C1/EHXPLL_UL RST/STDBY is not binary: 0 -> x | refused: Error: fast mode is two-valued; pin:A11 is x at 90 ns |
| `pll_reference_transition_x` | refused: Error: event kernel at 90000 ps: pll:LFE5U-85F/R4C1/EHXPLL_UL reference is not a binary clock: 0 -> x | refused: Error: fast mode is two-valued; pin:A11 is x at 90 ns |

Tool-stage negative controls:

| Scratch design | Tool result | Audit consequence |
|---|---|---|
| `latch`: `always @* if(en) q=d;` | Yosys PROC_DLATCH: `ERROR: Latch inferred for signal …` | No default synth_ecp5 image; D for ordinary behavioural latch. |
| `prld`: `always @(posedge clk or posedge load) if(load) q<=m; else q<=d;` | Yosys FF legalization: `$_ALDFF_PP_ cannot be legalized: dffs with async set and reset are not supported` | Native PRLD errors require explicit primitive/raw profiles in this pipeline. |
| `comb_loop`: `assign q=en?~q:d;` | nextpnr: `Timing analysis failed due to combinational loops.` | No default-route image; do not call fast's acyclic-logic guard an inferred-latch blocker. |
| `lvds` on A16 | ecppack: `cannot set OUTPUT_LVDS` for the top-bank tile | Invalid placement for this native standard, not the emulator's LVDS finding; X05 uses A2. |
| tiny `dcsc`, `dcsc_oneclk`, `dcsc_pll`, `dcsc_sample` | nextpnr router assertion at `router1.cc:445` | Failed repro reductions, not proof of DCSC being unreachable; full HC02 X22 routes and hits fast's refusal. |
| DQS with constant DDRDEL / inappropriate side clock source | nextpnr cannot route GND/dedicated edge source | Use the working native DDRDLL→DQS chain in X08. |

### Minimal A/B trigger RTL

These are the actual small sources, with the otherwise unused `hb` harness FF omitted where it adds nothing to the trigger. For a standalone combinational primitive test, retain `output reg hb=0; always @(posedge clk) hb<=~hb;` and its clock constraint as in the experiment protocol. Shared module `pll` below is reused in the reference variants. All normal input levels are listed explicitly so a missing stimulus cannot masquerade as a primitive refusal.

#### X01 / R01: narrow LUTRAM

WE=1, A=0, D=1. X02 changes the data/memory/output width to 4 and passes.

```verilog
module top(input clk,we,input [3:0] a,input d,output q); reg m[0:15]; integer i; initial for(i=0;i<16;i=i+1)m[i]=0;
  always @(posedge clk) if(we)m[a]<=d;
  assign q=m[a];
endmodule
```

Routed config SHA-256: `fceea405fb3cfddd398c5b68dd8fce434be8d076383f1d842439028c2a8083b9`.

#### X03 / R02: inferred 32-bit RAM

WE=1, A=0, D=1. X04 changes only WE=0 and passes.

```verilog
module top(input clk,we,input [8:0] a,input [31:0] d,output reg [31:0] q); reg [31:0] m[0:511]; integer i; initial for(i=0;i<512;i=i+1)m[i]=0;
  always @(posedge clk) begin if(we)m[a]<=d; q<=m[a]; end
endmodule
```

Routed config SHA-256: `d4c4d132184401c83a9789748febb9488db2db204b7450bc693511d736a904be`.

##### R02 follow-up: HB09P measured NEW; event and fast rules implemented (2026-11-17)

Exact X03 maps through `$__PDPW16KD_` to one `DP16KD`, A/B widths 36,
physical MODE=PDPW16KD/read width 36. NORMAL defaults, NOREG, ASYNC reset/release,
GSR disabled; A writes, B reads, one positive clock; CE=1/1, OCE=0/1, masks are
registered WE. Yosys adds registered write address/data/WE and delayed-data
forwarding (75 fabric FFs). Previously both executors made INIT9 rows 0–3,
all 36 read bits, X at the 60 ns collision. The unselected forwarding input
hid it from event q=1; fast's static check rejected it.

[HB09P](inventory/LFE5U-85F/hb09p_pdp_collision/README.md) now supplies hardware
records: **explicit primitive DOA/DOB**, with no inferred-RAM forwarding before
the recorder, return **NEW in 1,299/1,299 known collisions per case** for
widths 32/36 × depths 64/128/256/512 (physical read width 36 throughout).
Three loads have the same 78 complete windows; the configuration window agrees
(23/23 NEW), and clean/post-collision reads match. This is raw PDP behavior,
not an inferred-q trace or a demonstrated Yosys/RTL mismatch: the delayed
physical write is the previous logical write. The changing-D X03 test keeps
logical read-old semantics and known q at every address.

`src/kernel/ebr.rs` implements NEW only in the measured PDP36/NOREG/NORMAL,
ASYNC reset/release, GSR-disabled profile with definite shared edges, selected
CE/CS, OCE=0/1, inactive reset/GWDIS, known addresses and full masks. Other PDP
collisions retain X. `tests/ebr_r02.rs` passes the 17,408-bit board replay,
all 512 physical words, exact X03's 512 rows, and conservative fallbacks.
Fast now applies
[A forwarding](inventory/LFE5U-85F/hb09p_pdp_collision/fast_a_hb09p.patch)
in `be71ccb`, in the same measured profile. The pending-A ignore was removed
from `fast_pdp36_new_word_equal_trace`; only its clang requirement remains.
The new `fast_x03_inferred_q_is_known_new_word_in_every_row` starts `FastSim`
from the same compiled design as the event comparison and drives inputs with
`set_at`. Both executors match expected logical q on **2,048 sampled rows**
(four cycles at each of 512 addresses, 32 bits per row), including changing D
on the third edge. Hunk A alone fixes the student X03 case.

[B runtime selected-demand](inventory/LFE5U-85F/hb09p_pdp_collision/fast_b_selected_demand.patch)
is **held, not applied**: its remaining masked-X case is rare after A, while
selected-demand checks would cost every EBR design. The
`fast_unselected_mux_collision_x_equal_trace_then_selected_x_refuses` test
keeps its pending-B ignore. B would defer unused-X checks while preserving
errors for a selected X, including selector changes without a new EBR edge.
It is not required for the measured PDP/X03 fix. Unmeasured collisions retain
conservative X handling; the host's `src/fast.rs` error-2 handler needs no change.

Commit `be71ccb` records rserv verification: `ebr_r02` **7 passed** (the B test
remains ignored), `ebr` 19, `ebr_start_native` 2, `ebr_collision_native` 3,
`fast_mode` 21 and `dsp` 19; Life and picorv32_console generated programs were
byte-identical, and fmt/clippy checks passed. These are the recorded checks from that commit.

#### X05 / R03: native LVDS from a constraint — fixed

`LOCATE COMP "q" SITE "A2"; IOBUF PORT "q" IO_TYPE=LVDS PULLMODE=NONE;` All other ports LVCMOS25. D=1.

```verilog
module top(input clk,d,output reg q);
  always @(posedge clk)q<=d;
endmodule
```

Original routed config SHA-256: `383e325821207321ea614301f6665981eab0dac37f1d85cc99c8c5875a31b0e5`.

**R03 fixed (2026-11-17), removed from the ranking (former rank 3).** Proposed
commit subject: `Execute differential PIO inputs and outputs (R03)`. No commit
was created. `tests/r03_differential.rs` builds this exact FF/constraint and an
LVDS input variant with Yosys/nextpnr/ecppack from PATH on rserv. Kernel/fast
traces agree at stimulus changes and both clock edges. Output P=data,
N=complement; both share tristate and GOE. Input reads P only: N and invalid-pair
behaviour are not modelled, whether N is complementary, equal to P or undriven.
The tests cover those three input stimuli and the other packable differential
standards. HP02's existing LVCMOS33D tristate regression is retained.

The 46 catalogue differential values comprise 17 OUTPUT and 16 INPUT values
executed and 13 BIDIR values refused. The old blanket guard covered the other
45 values besides OUTPUT_LVCMOS33D. Bidirectional differential IO (including
pseudo-differential bidir), open-drain differential output and ambiguous pair
ownership/direction still fail with IO_TYPE and reason. The
[PIO mapping table](PIO_BANKS.md#differential-digital-io-r03-added-2026-11-17) and
[test fixture](../tests/fixtures/r03_differential/README.md) distinguish these
emulator boundaries from nextpnr/ecppack availability; the sweep checked 80
constraints, 51 routing and packing.

This is a digital model, with no board check. ECP5 true-LVDS output needs a
2.5 V bank; ULX3S side banks run at 3.3 V, making that physical use out of
specification. The digital semantics are not in doubt. Termination, VCCIO,
common mode and swing are ignored like DRIVE/SLEWRATE. Both fast source files,
live/board code and GUI remain unchanged.

**Validation on rserv:** PIO decoder unit tests 7 passed; new R03 native tests
4 passed (32 packable differential spellings, including X05 and LVDS input,
with equal kernel/fast traces); targeted `--include-ignored` runs passed
`automatic_compile` 16, `configured_startup_native` 2, `differential_native`
(HP02) 1, `fast_mode` 21, `io_delay` 2, `io_delay_native` 2 and
`resource_decode` 1. The two HI08 native tests initially hit the shared /tmp
quota during clang compilation; rerunning them serially with
`TMPDIR=$PWD/target/r03-tmp` passed. Full default suite, once:
`cargo test --release -p ecp5_emu` — 324 passed, 205 ignored, no failures.
`cargo fmt --all -- --check` and
`cargo clippy --release -p ecp5_emu --all-targets -- -D warnings` passed.

#### X06 / R04: ordinary memory DDR output

Q=A2, D=0, E=1. The first refusal is MODDRX.MODE=MOSHX2.

```verilog
module top(input clk,d,e,output q); OSHX2A u(.SCLK(clk),.ECLK(clk),.RST(1'b0),.D0(d),.D1(e),.Q(q));  endmodule
```

Routed config SHA-256: `feb09e350c202248e4d3711484cac89c97053630ce4e92286b737a1aac84bae9`.

#### X07 / R05: native DLL

No unusual primitive parameter. The first refused output is LOCK.

```verilog
module top(input clk,output q); DDRDLLA u(.CLK(clk),.RST(1'b0),.UDDCNTLN(1'b0),.FREEZE(1'b0),.LOCK(q));
endmodule
```

Routed config SHA-256: `c754eb9bcadda28974827d6b1bbc68aa422ea4e24cdca8d82f8877f1bbde7bdd`.

#### X08 / R05: native DQS chain

DQSI=dqs at E4; initial dqs=0. DDRDEL must come from the DLL, not fabric GND.

```verilog
module top(input clk,dqs,output q);
  wire dd; DDRDLLA dll(.CLK(clk),.RST(1'b0),.UDDCNTLN(1'b0),.FREEZE(1'b0),.DDRDEL(dd)); DQSBUFM u(.DQSI(dqs),.SCLK(clk),.ECLK(clk),.RST(1'b0),.READ0(1'b1),.READ1(1'b1),.DDRDEL(dd),.PAUSE(1'b0),.READCLKSEL0(1'b0),.READCLKSEL1(1'b0),.READCLKSEL2(1'b0),.DATAVALID(q));  endmodule
```

Routed config SHA-256: `aefbbb22d1d1a5d9bc4d9a0d5e8753842d4918e4908ba05a02609781face7469`.

#### X09 / R06: hand-instantiated PLL with reset explicitly enabled

RST=0 throughout. The explicit `.PLLRST_ENA("ENABLED")` below causes the fast refusal even before any pulse; event execution succeeds. This is a hand-instantiated enabled-reset profile, applicable to Lattice IP/Clarity-generated PLLs or hand instances that explicitly enable RST/STDBY. It is **not** the default LiteX ECP5PLL profile. The scratch directory retains its historical name `litex_pll_reset` for traceability; that name does not establish LiteX provenance. R06 has likelihood weight 1 and priority 1×2=2 because ordinary LiteX PLL usage is excluded.

```verilog
module top(input clk,rst,output q);
  wire c,fb,l;
(* ICP_CURRENT="6", LPF_RESISTOR="16" *)
EHXPLLL #(.CLKI_DIV(1),.CLKFB_DIV(4),.CLKOP_DIV(6),.CLKOP_CPHASE(2),.CLKOP_ENABLE("ENABLED"),.CLKOS3_ENABLE("ENABLED"),.CLKOS3_DIV(6),.CLKOS3_CPHASE(2),.FEEDBK_PATH("INT_OS3"),.PLLRST_ENA("ENABLED")) u(.CLKI(clk),.CLKFB(fb),.CLKINTFB(fb),.CLKOP(c),.LOCK(l),.RST(rst),.STDBY(1'b0));
reg [3:0] n=0;
  always @(posedge c)n<=n+1;
  assign q=n[3]^l;
endmodule
```

Routed config SHA-256: `a3d4dc6bc0317d1cf139ba1f96521f7153c9691663fff99446bd88404b47213d`.

#### X10 / R07 and X19 / R14: inferred tri-state/readback

Pad=A16. X10: PULLMODE=NONE, OE=0, D=1, no outside driver. X19: PULLMODE=UP, PAD=0, D=1, OE initially 0 then 1 at 90 ns. X11: pull-up and no outside driver passes.

```verilog
module top(input clk,oe,d, inout pad, output q);
  assign pad=oe?d:1'bz;
  assign q=pad;
endmodule
```

Routed config SHA-256: `281fafbabd8e3172489fa971bc7248c3332f62ace2fedf42645bcebda8e3c79b`.

#### X12 / R08: SDR output followed by output delay

**Fixed (former rank 6).** Q=A2; D=1. IOLDOD now passes IOLDO with zero propagation time,
from the IO register or bypassed fabric TXDATA0. DELAY.OUTDEL=ENABLED is accepted; output
DELAYF CFLAG runs the same counter logic as input DELAYF. An input DELAYG and a fabric FF
feeding DELAYG did not originally exercise this planner path: the fabric-FF case already
passed through the PIO's default fabric route, while bypassing delay validation. It now
explicitly plans and checks the delay too.

[HI11](inventory/LFE5U-85F/hi11_output_delay/README.md) measured correct, uninverted UART data
through OFS1P3BX/DELAYF, OFS1P3BX/DELAYG(127), and fabric FF/DELAYG(127). Startup measurements
at DEL_VALUE 1/10/63/64/100/127 found 0/10/62/0/36/62 on left-bank output L4 and input A2:
IOLOGIC starts at DEL_VALUE & 0x3E. Bottom-bank input R1 starts at 0, agreeing with HI08.
All post-load samples match HI08's seven-bit counter model. Right IOLOGIC and top SIOLOGIC
use their kind's startup rule as an unmeasured assumption. `tests/output_delay.rs` compares
kernel/fast pad traces and the native HI11 control/CFLAG traces against the board records.

DELAY.WAIT_FOR_EDGE remains refused with an error naming the setting; public nextpnr
DELAYG/DELAYF packing does not enable it. This is separate from the fixed R08 output path.

```verilog
module top(input clk,d,output q);
  wire v; OFS1P3BX r(.SCLK(clk),.SP(1'b1),.PD(1'b0),.D(d),.Q(v)); DELAYG #(.DEL_VALUE(1)) u(.A(v),.Z(q));  endmodule
```

Routed config SHA-256: `bdb028b6c3506d949e06b49d6515288396a2d34f6a9e2fbf6748f4e467cfc5a2`.

#### X13 / R09: normal clock divider — fixed (HI12)

No explicit GSR or DIV parameter; the routed synchronizer chain contains MODE NONE.
R09 is fixed for this qualified used route with unrouted STOP: ECLKO=ECLKI,
without startup gating, in the event kernel and fast mode. The resource owner,
MODE and STOP path are checked; NONE with logic/constant-connected STOP remains
refused. [HI12 run_2](inventory/LFE5U-85F/hi12_eclksync_none/README.md#reading)
contains three SRAM loads of each of five variants. Default/explicit /2 have
I=P+F at both snapshots; /3.5 has the expected 14-observer-sample period and
integer edge counts consistent with zero deficit and a free initial phase.
The qualified NONE rule also executes for BRGECLKSYNC; HI12 directly measured
bank 7 lane 1, so other placements and BRGECLKSYNC NONE are model extensions.

**Enabled-start accuracy item closed by HI13 run_1.** Three SRAM loads give
P=(262144,262144), then (524288,524288), matching HI12 sync_2 with zero deficit.
A DCCA enabled 64 pin cycles after wake-up has G_in=I-64 and
G=(262110,262110), then (524254,524254): four missing input periods at both
snapshots. The planner initializes the four STOP stages to 0 for a primary
clock input pin through routes/buffers and constant-CE=1 DCC/DCCA; fabric and
other DCC enables retain HI04's stages at 1. Runtime STOP still shifts on four
falling edges. PLL/OSCG configuration-time activity is unmeasured and deliberately
keeps stages at 1. Physical PUR/configuration history and an externally withheld
primary pin clock are not simulated. Packed HI13 and HI12 sync_2 replay tests
check both board snapshots and complete event/fast transition equality; HI04
native tests preserve the fabric-source rule. See [HI13 Reading](inventory/LFE5U-85F/hi13_eclksync_start/README.md#reading)
and [the clock rule](CLOCK_NETWORK.md#eclksync-startup).

```verilog
module top(input clk,output q); CLKDIVF u(.CLKI(clk),.RST(1'b0),.ALIGNWD(1'b0),.CDIVX(q));  endmodule
```

Routed config SHA-256: `bf8f700c2f0162cac2fb2809611ef6eac8f860dccdaa7146533026a212db3a80`.

#### X14 / R10: fabric JTAG

Used JTDI makes the unsupported JTAG producer observable.

```verilog
module top(input clk,output q); JTAGG u(.JTDO1(1'b0),.JTDO2(1'b0),.JTDI(q));
endmodule
```

Routed config SHA-256: `1a6aa70f90d0c45ef6ab1cbb23f2462ed3ed48ac08dabcc26c716d19ec7c3f61`.

#### X15 / R11: missing ordinary input stimulus

D is left unspecified with PULLMODE=NONE. Set D=1 to make it binary; ordinary FF execution itself is supported.

```verilog
module top(input clk,d, output reg q);
  always @(posedge clk) q<=d;
endmodule
```

Routed config SHA-256: `b1cd69647291f5bbddc1e5c7eaeeb6e5c13f353166cda770bef818d75d4ac7e9`.

#### X20 / R15: synchronous global reset

**R15 fixed by HY01.** RST=1 deasserts the active-low GSR pin. The model is
`user_gsr = active(GSR pin) OR s2`, with plain rising-SGSR-CLK stages initially
`11`, sampling `(s1,s2) <- (active,old s1)`. There is no asynchronous preset,
local reset or global reset on the stages; the effective reset acts asynchronously
on GSR-enabled resources. With RST=1 and D=1, q stays 0 at rising edges 1 and 2,
and first captures 1 at edge 3. `tests/sgsr_native.rs` rebuilds this exact HDL
as `.config` and `.bit` and checks equal event/fast settled traces, both wake
phases, pulse phases and the six-FF HY01 board records. This is new execution
coverage of the X20 HDL, not a claim that the old frozen binary was rerun.

[HY01](inventory/LFE5U-85F/hy01_sgsr/README.md#accepted-functional-rule-and-timing-hazards)
leaves 120 exact residual channel-bins: 71 at s2 assertion, 15 at raw release,
34 at s2 release. Every bin is within two observer ticks (20 ns) after a
modelled reset transition. They are propagation-delay hazards; the emulator
assigns propagation delay zero time. S has no residuals; hy01b/c startup offsets
match two held rising edges and one/two held falling edges (no-SGSR control 0/0).
Unmeasured non-ACTIVE_LOW SYNC and missing CLK routes remain refused.
`automatic_compile` now checks its explicitly edited ACTIVE_HIGH SYNC profile's
polarity refusal, rather than a blanket failure of public SGSR.

```verilog
module top(input clk,rst,d,output reg q); SGSR u(.GSR(rst),.CLK(clk));
  always @(posedge clk)q<=d;
endmodule
```

Routed config SHA-256: `2a5b58eb2b1a9eec014247c80b6e1cbe75731a845fd1fa7584c7cac5212411d5`.

#### X21: temperature primitive (supported control since 1c79418)

Default DTR, STARTPULSE=0; the event kernel and fast mode execute it. DTR is no longer part of R16; DCSC remains refused. On a definite STARTPULSE fall, the fast host schedules the profile byte at exactly fall + 32 740 000 ps, cancels/restarts on retrigger, and publishes completion with any coincident clock edge before captures. Fast run snapshots retain DTR state. The follow-up below re-ran the 7 DTR kernel tests and 2 native tests, including exact completion, capture ordering and the 0x9E/0xA1 profiles.

```verilog
module top(input clk,output q); DTR u(.STARTPULSE(1'b0),.DTROUT0(q));
endmodule
```

Routed config SHA-256: `8fd08ca535ecf7d6ba1232e052b36c29bb75c5468ddfe117b90b2a318573f4aa`.

#### X18 / R13: independent-port memory collision

WE=1, A=B=0, D=1, E=2. clk2 starts 0 and follows clk at 20,40,…,400 ns; both write at 20 ns.

```verilog
module top(input clk,clk2,we,input [9:0] a,b,input [8:0] d,e,output [8:0] q); reg [8:0] m[0:1023]; reg [8:0] qa,qb; integer i; initial for(i=0;i<1024;i=i+1)m[i]=0;
  always @(posedge clk) begin if(we)m[a]<=d; qa<=m[a]; end always @(posedge clk2) begin if(we)m[b]<=e; qb<=m[b]; end assign q=qa^qb;
endmodule
```

Routed config SHA-256: `8e80d1696db790557afd4d554e626c068fe61c4af64337f86a1a7b2e7cd20b1b`.

#### Shared ecppll module; X17 / R12 and X23 / R17 reference variants

The shared module is the local blinky_pll ecppll output; the positive X29 control uses it directly from `clk`. The minimum divider/reference wrapper is quoted below it. For X23, add a `refclk` top-level input and connect `.clkin(refclk)` instead; keep the independent harness clock on `clk`.

```verilog
module pll
(
    input clkin, // 25 MHz, 0 deg
    output clkout0, // 100 MHz, 0 deg
    output locked
);
(* FREQUENCY_PIN_CLKI="25" *)
(* FREQUENCY_PIN_CLKOP="100" *)
(* ICP_CURRENT="12" *) (* LPF_RESISTOR="8" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)
EHXPLLL #(
        .PLLRST_ENA("DISABLED"),
        .INTFB_WAKE("DISABLED"),
        .STDBY_ENABLE("DISABLED"),
        .DPHASE_SOURCE("DISABLED"),
        .OUTDIVIDER_MUXA("DIVA"),
        .OUTDIVIDER_MUXB("DIVB"),
        .OUTDIVIDER_MUXC("DIVC"),
        .OUTDIVIDER_MUXD("DIVD"),
        .CLKI_DIV(1),
        .CLKOP_ENABLE("ENABLED"),
        .CLKOP_DIV(6),
        .CLKOP_CPHASE(2),
        .CLKOP_FPHASE(0),
        .FEEDBK_PATH("CLKOP"),
        .CLKFB_DIV(4)
    ) pll_i (
        .RST(1'b0),
        .STDBY(1'b0),
        .CLKI(clkin),
        .CLKOP(clkout0),
        .CLKOS(),
        .CLKOS2(),
        .CLKOS3(),
        .CLKFB(clkout0),
        .CLKINTFB(),
        .PHASESEL0(1'b0),
        .PHASESEL1(1'b0),
        .PHASEDIR(1'b1),
        .PHASESTEP(1'b1),
        .PHASELOADREG(1'b1),
        .PLLWAKESYNC(1'b0),
        .ENCLKOP(1'b0),
        .LOCK(locked),
        .ENCLKOS(),
        .ENCLKOS2(),
        .ENCLKOS3(),
        .INTLOCK(),
        .REFCLK()
	);
endmodule
```

```verilog
module top(input clk,output q); reg div=0;
  always @(posedge clk)div<=~div;
  wire c,l; pll u(.clkin(div),.clkout0(c),.locked(l)); reg [3:0] n=0;
  always @(posedge c)n<=n+1;
  assign q=n[3]^l;
endmodule
```

X17's toggle FF makes a periodic reference that the event kernel accepts; fast rejects its *logic source*. X23 uses a separate input instead of the toggle FF and this stimulus:

```json
[{"time":20,"set":{"net:refclk":"1"}},
 {"time":40,"set":{"net:refclk":"0"}},
 {"time":60,"set":{"net:refclk":"1"}},
 {"time":80,"set":{"net:refclk":"0"}},
 {"time":110,"set":{"net:refclk":"1"}}]
```

With initial refclk=0, the event kernel rejects the 50 ns period after learning 40 ns; fast first rejects the secondary clock grid. For the grid-aligned period-change test, use rising edges at 20,68,124 ns (falling at 44,92,148 ns). To exercise the separate binary reference check, use 20/40/60/80 ns edges then `refclk=x` at 90 ns. To exercise binary reset, start X09 RST=0 then set RST=x at 90 ns. Fast CLI refuses nonbinary timed input before the kernel receives it.

#### X22 / R16: DCSC trigger within the successfully routed HC02 fixture

The fresh proof copies **all** of `docs/inventory/LFE5U-85F/hc02_dcs/dynamic.v` to scratch, renames only `clk_25mhz`→`clk`, `ftdi_rxd`→`q`, and uses G2/L4 constraints. The minimal *feature trigger* is the ordinary pair below; the full local fixture supplies its clocks/control/observers. A standalone reduced wrapper was **not** shown to route, so this snippet is not presented as an independently reproduced tiny top.

```verilog
(* BEL="X66/Y22/DCS0", DCSMODE="POS" *)
DCSC #(.DCSMODE("POS")) dcs0(
  .CLK0(c0), .CLK1(c1), .SEL0(control[0]), .SEL1(control[1]),
  .MODESEL(control[2]), .DCSOUT(d0));
(* BEL="X66/Y70/DCS1", DCSMODE="NEG" *)
DCSC #(.DCSMODE("NEG")) dcs1(
  .CLK0(c0), .CLK1(c1), .SEL0(control[0]), .SEL1(control[1]),
  .MODESEL(control[2]), .DCSOUT(d1));
```

#### Positive behavioural controls

X24: `always @(posedge clk or posedge rst) if(rst) q<=0; else q<=d;` (RST=0,D=1). X16 uses the same with ready-unknown startup: fast warms one cycle. X26: `always @(posedge clk) q<=a*b;` with 32-bit A/B and 64-bit Q (A=3,B=7). X25: write a 1024×9 RAM at clk and read it in `always @(posedge clk or posedge rst) if(rst)q<=0; else q<=m[a];` (RST=1,WE=0). X27 instantiates `ODDRX1F(.SCLK(clk),.RST(rst),.D0(d),.D1(e),.Q(q))`, RST=0,D=0,E=1. X28 instantiates `OFS1P3BX(.SCLK(clk),.SP(en),.PD(rst),.D(d),.Q(q))`, EN=1,RST=0,D=1. These passed both engines.

For SERV X31, the actual unmodified vendored module is used, with this wrapper (and i_ren tied high, rather than accidentally leaving it floating):

```verilog
module top(input clk,wen, input [9:0] wa,ra,
           input [1:0] d, output [1:0] q);
  serv_rf_ram #(.width(2),.depth(576)) u(
    .i_clk(clk),.i_waddr(wa),.i_wdata(d),.i_wen(wen),
    .i_raddr(ra),.i_ren(1'b1),.o_rdata(q));
endmodule
```

The relevant behavioural memory in that source is:

```verilog
reg [width-1:0] memory [0:depth-1];
reg [width-1:0] rdata;
always @(posedge i_clk) begin
  if (i_wen) memory[i_waddr] <= i_wdata;
  rdata <= i_ren ? memory[i_raddr] : {width{1'bx}};
end
```

The vendored x0 masking and the rest of its RTL were retained in the experiment. This is a control for the previous SERV refusal, not a claim that ten cycles exercise an entire CPU.

## Coverage-inventory cross-check

The frozen-snapshot CSV has 322 rows: {'executed': 204, 'refused': 22, 'ignored-no-effect': 41, 'resource-not-executed': 23, 'partly': 32}. Every one of its **54 refused/partly rows** is accounted for below. This checklist is not itself a reachability oracle: legal nextpnr defaults and fixed aliases can make an inventory refusal unreachable, while the inferred-memory fast failures are not expressed by a single refused setting. The concurrent CSV now records both-edge DSP execution in fast mode; the removed divider refusal is not restored from stale prose.

R08 follow-up: OUTDEL is executed (HI11), so the current CSV recount is 205 executed,
21 refused, 32 partly, 23 resource-not-executed and 41 ignored-no-effect (322 total).
WAIT_FOR_EDGE remains refused with the setting named explicitly.

R15 follow-up: GSR.SYNCMODE is executed for ASYNC/NONE and qualified ACTIVE_LOW
SYNC with routed CLK. The current 322-row recount is **209 executed, 28 partly,
21 refused, 23 resource-not-executed and 41 ignored-no-effect**. The snapshot
counts above remain historical; no unrelated field was changed for R15.

| Group / field | CSV status | Class | Audit disposition |
|---|---|---|---|
| `ALU54_` / `FORCE_ZERO_BARREL_SHIFT` | refused | C | R24 |
| `ALU54_` / `LEGACY` | refused | C | R24 |
| `ALU54_` / `MASKPAT_SOURCE` | refused | C | R24 |
| `ALU54_` / `MCPAT_SOURCE` | refused | C | R24 |
| `ALU54_` / `REG_OPCODEIN_1_CLK` | refused | C | R22 |
| `ALU54_` / `REG_OUTPUT0_CLK` | partly | C | R23 |
| `ALU54_` / `REG_OUTPUT1_CLK` | partly | C | R23 |
| `BRGECLKSYNC` / `MODE` | executed | B | R09 fixed: NONE buffer with unrouted STOP (HI12); ECLKSYNCB STOP model (HI04); HI13 source-dependent start; PLL/OSCG history unmeasured |
| `CLKDIV_L` / `GSR` | partly | C | R18; nextpnr default DISABLED |
| `CLKDIV_R` / `GSR` | partly | C | R18; nextpnr default DISABLED |
| `CLKOP_ENABLE` / `(group value)` | partly | C | R27 |
| `CLKOP_TRIM_DELAY` / `(group value)` | refused | C | R27 |
| `CLKOP_TRIM_POL` / `(group value)` | partly | C | R27 |
| `CLKOS2_ENABLE` / `(group value)` | partly | C | R27 |
| `CLKOS3_ENABLE` / `(group value)` | partly | C | R27 |
| `CLKOS_ENABLE` / `(group value)` | partly | C | R27 |
| `CLKOS_TRIM_DELAY` / `(group value)` | refused | C | R27 |
| `CLKOS_TRIM_POL` / `(group value)` | partly | C | R27 |
| `DPHASE_SOURCE` / `(group value)` | refused | C | R27; LiteX enables it only after explicit expose_dpa(), not by default |
| `DSP_LEFT` / `CIBOUT` | partly | D | D-DSP; live MULT/ALU writer sets CIBOUT ON |
| `DSP_RIGHT` / `CIBOUT` | partly | D | D-DSP; live MULT/ALU writer sets CIBOUT ON |
| `EBR` / `ASYNC_RESET_RELEASE` | partly | C | R26; conditional reset only; inferred mapper sets matched release |
| `ECLKSYNC0_BK` / `MODE` | executed | B | R09 fixed: NONE buffer with unrouted STOP (HI12); ECLKSYNCB STOP model (HI04); HI13 source-dependent start; PLL/OSCG history unmeasured |
| `ECLKSYNC1_BK` / `MODE` | executed | B | R09 fixed: NONE buffer with unrouted STOP (HI12); ECLKSYNCB STOP model (HI04); HI13 source-dependent start; PLL/OSCG history unmeasured |
| `FEEDBK_PATH` / `(group value)` | partly | C | R27; INT_OS3 itself supported |
| `GSR` / `SYNCMODE` | executed | B, fixed | R15 fixed: HY01 ACTIVE_LOW SYNC with routed CLK; ASYNC/NONE unchanged; qualified polarity/route guards retained |
| `INTFB_WAKE` / `(group value)` | refused | C | R27 |
| `IOLOGIC` / `CEIMUX` | partly | C | R19; SDR and X1 GSR profiles supported |
| `IOLOGIC` / `CEOMUX` | partly | C | R19; SDR and X1 GSR profiles supported |
| `IOLOGIC` / `CLKIMUX` | partly | C | R28; deliberately omitted clock / raw IOLOGIC |
| `IOLOGIC` / `CLKOMUX` | partly | C | R28; deliberately omitted clock / raw IOLOGIC |
| `IOLOGIC` / `DELAY.OUTDEL` | executed | B, fixed | R08 fixed; HI11 measured output data and DELAYF startup; zero propagation time |
| `IOLOGIC` / `DELAY.WAIT_FOR_EDGE` | refused (named setting) | D | R08 output-delay blocker fixed; WAIT_FOR_EDGE is not set by public DELAYG/F packing and remains refused |
| `IOLOGIC` / `GSR` | partly | C | R19; SDR and X1 GSR profiles supported |
| `IOLOGIC` / `IOLTOMUX` | refused | C | R28; explicit memory tristate/raw latch combination |
| `IOLOGIC` / `MIDDRX.MODE` | refused | B | R04; ordinary memory-DDR primitive; other raw combinations C |
| `IOLOGIC` / `MODDRX.MODE` | refused | B | R04; ordinary memory-DDR primitive; other raw combinations C |
| `IOLOGIC` / `MODE` | partly | B | R04; ordinary memory-DDR primitive; other raw combinations C |
| `IOLOGIC` / `MTDDRX.MODE` | refused | C | R28; explicit memory tristate/raw latch combination |
| `IOLOGIC` / `SRMODE` | partly | C | R19; SDR and X1 GSR profiles supported |
| `IOLOGIC` / `TSREG.OUTREGMODE` | refused | C | R28; explicit memory tristate/raw latch combination |
| `MULT18_` / `REG_INPUTC_CLK` | partly | C | R21 |
| `MULT18_` / `SOURCEB_MODE` | partly | C | R20 |
| `OUTDIVIDER_MUX` / `(group value)` | refused | C | R27 |
| `PIO` / `BASE_TYPE` | partly | C (remaining restrictions) | R03 fixed for all differential INPUT/OUTPUT values; BIDIR/open-drain/ambiguous pairs remain refused |
| `REFIN_RESET` / `(group value)` | refused | C | R27 |
| `SYNC_ENABLE` / `(group value)` | refused | C | R27 |
| `SYSCONFIG` / `DONEPHASE` | partly | C | R29 |
| `SYSCONFIG` / `GOEPHASE` | refused | C | R29 |
| `SYSCONFIG` / `GSRPHASE` | refused | C | R29 |
| `SYSCONFIG` / `GWEPHASE` | refused | C | R29 |
| `SYSCONFIG` / `MASTER_SPI_PORT` | partly | C | R29 |
| `SYSCONFIG` / `SLAVE_PARALLEL_PORT` | partly | C | R29 |
| `SYSCONFIG` / `SLAVE_SPI_PORT` | partly | C | R29 |

The frozen-snapshot CSV also has **31 rows whose status is neither refused nor partly but whose refused_values column is nonempty**. Those must not disappear from the audit. The current R15 recount is **35**: the three R09 MODE rows are already accounted for in the table above, and the qualified GSR.SYNCMODE row is added below:

| Group / field | Status | Remaining refusal / disposition |
|---|---|---|
| `ALU54_` / `CLK0_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `ALU54_` / `CLK1_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `ALU54_` / `CLK2_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `ALU54_` / `CLK3_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `ALU54_` / `REG_OPCODEIN_1_CE` | resource-not-executed | all; C, R22: non-NONE clock rejected first; bypassed stage does not consume CE/RST |
| `ALU54_` / `REG_OPCODEIN_1_RST` | resource-not-executed | all; C, R22: non-NONE clock rejected first; bypassed stage does not consume CE/RST |
| `ALU54_` / `REG_OUTPUT0_RST` | executed | the two fields on different lines ('ALU54B REG_OUTPUT0_RST=... and REG_OUTPUT1_RST=... differ: not measured'); C, R23: mismatched reset selections |
| `ALU54_` / `REG_OUTPUT1_RST` | executed | the two fields on different lines ('ALU54B REG_OUTPUT0_RST=... and REG_OUTPUT1_RST=... differ: not measured'); C, R23: mismatched reset selections |
| `CI` / `JCLK0MUX` | executed | 0 with an unexpected pattern ('unqualified CIB clock/LSR constant encoding'); E: unexpected physical constant encoding / catalogue mismatch; valid writer code is qualified |
| `CI` / `JCLK1MUX` | executed | 0 with an unexpected pattern ('unqualified CIB clock/LSR constant encoding'); E: unexpected physical constant encoding / catalogue mismatch; valid writer code is qualified |
| `CI` / `JLSR0MUX` | executed | 0 with an unexpected pattern ('unqualified CIB clock/LSR constant encoding'); E: unexpected physical constant encoding / catalogue mismatch; valid writer code is qualified |
| `CI` / `JLSR1MUX` | executed | 0 with an unexpected pattern ('unqualified CIB clock/LSR constant encoding'); E: unexpected physical constant encoding / catalogue mismatch; valid writer code is qualified |
| `DCS` / `DCSMODE` | executed | Fast mode refuses DCSC; B, R16: event mode works; fast mode exclusion proved by X22 |
| `EBR` / `CSDECODE_A` | executed | unresolved word bits; E: fixed three-bit word decode invariant |
| `EBR` / `CSDECODE_B` | executed | unresolved word bits; E: fixed three-bit word decode invariant |
| `EBR` / `MODE` | executed | NONE when reached ('reachable inactive/unsupported EBR mode'); D: inactive/invalid writer state; see exact ledger site |
| `EBR` / `REGMODE_A` | executed | 36-bit PDP read with REGMODE_A != REGMODE_B; C, R26: explicit mismatched PDP36 register groups |
| `EBR` / `REGMODE_B` | executed | 36-bit PDP read with REGMODE_A != REGMODE_B; C, R26: explicit mismatched PDP36 register groups |
| `EBR` / `WID` | executed | duplicate/missing/invalid image -> live array X, not refused; Not a refusal itself: unresolved array contents propagate X; R02/R13/R32 govern fast reads |
| `GSR` / `SYNCMODE` | executed | HY01 requires ACTIVE_LOW polarity and a routed CLK for SYNC; unknown/ambiguous profiles remain refused; R15 public SGSR now executes |
| `MODE` / `(group value)` | executed | NONE when reached ('PLL ... MODE is not EHXPLLL'); D: inactive/invalid writer state; see exact ledger site |
| `MULT18_` / `CIBOUT_BYP` | executed | unresolved/conflicting selector bits; E: unqualified raw bypass selector; measured normal profile has an accepted view |
| `MULT18_` / `CLK0_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `MULT18_` / `CLK1_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `MULT18_` / `CLK2_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `MULT18_` / `CLK3_DIV` | executed | Mixed divider profiles or ALU/MULT disagreement (HD09); R25 (mixed profiles); uniform DISABLED no longer fast-refused |
| `MULT18_` / `MODE` | executed | NONE when reached ('inactive/unsupported multiplier mode'); D: inactive/invalid writer state; see exact ledger site |
| `OS` / `DIV` | executed | a bit pattern with no measured divider; D for valid DIV=2..128: every emitted bit pattern is covered by the measured table |
| `OS` / `MODE` | executed | NONE when reached ('OSC.MODE ... is not OSCG'); D: inactive/invalid writer state; see exact ledger site |
| `PIO` / `OPENDRAIN` | executed | ON on a differential pair ('open-drain pair has no complementary push-pull digital model'); C: unusual open-drain pair remains refused after R03 |
| `SLICE` / `MODE` | executed | mixed LUTRAM modes ('unsupported mixed LUTRAM modes'); OFX in CCU2 ('OFX in CCU2 mode needs a separate qualified model'); D: normal RAMW packing and wide-mux/carry placement prohibit these combinations |
| `SYSCONFIG` / `DONE_EX` | executed | other combinations; EFB0/EFB2 fragments disagreeing; C, R29 for custom wake combinations; E for disagreeing replicated fragments |

The **23 resource-not-executed rows** were also checked against public writer support, rather than silently discarded because they were not in the requested status filter:

| Group / field | Can this pinned student pipeline request it? |
|---|---|
| `ALU54_` / `REG_OPCODEIN_1_CE` | C, R22: explicit second IN opcode register. Its non-NONE clock is rejected first; CE/RST are irrelevant when the stage is bypassed. |
| `ALU54_` / `REG_OPCODEIN_1_RST` | C, R22: explicit second IN opcode register. Its non-NONE clock is rejected first; CE/RST are irrelevant when the stage is bypassed. |
| `DDRDLL` / `FORCE_MAX_DELAY` | B, public DDRDLLA: R05 proved on LOCK; other DLL controls do not fix absence of its model. |
| `DDRDLL` / `GSR` | B, public DDRDLLA: R05 proved on LOCK; other DLL controls do not fix absence of its model. |
| `DDRDLL` / `MODE` | B, public DDRDLLA: R05 proved on LOCK; other DLL controls do not fix absence of its model. |
| `DLLDEL_` / `DEL_ADJ` | D for a normal public mapping in this writer; architectural metadata alone does not prove an emitted standalone cell. |
| `DLLDEL_` / `DEL_VAL` | D for a normal public mapping in this writer; architectural metadata alone does not prove an emitted standalone cell. |
| `DQS` / `DDRDEL` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `DQS_LI_DEL_ADJ` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `DQS_LI_DEL_VAL` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `DQS_LO_DEL_ADJ` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `DQS_LO_DEL_VAL` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `GSR` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `MODE` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `PAUSE_USED` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `RDLOADN_USED` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `READ_USED` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `DQS` / `WRLOADN_USED` | B, public DQSBUFM: R05 proved with native DLL connection. |
| `JTAG` / `ER1` | B, public JTAGG: R10 proved; fabric TAP/user chains absent. |
| `JTAG` / `ER2` | B, public JTAGG: R10 proved; fabric TAP/user chains absent. |
| `SE` / `CHECKALWAYS` | D for ordinary public mappings on LFE5U-85F; generic unsupported-resource guard retained. No inferred trigger demonstrated. |
| `SE` / `CLK_FREQ` | D for ordinary public mappings on LFE5U-85F; generic unsupported-resource guard retained. No inferred trigger demonstrated. |
| `SE` / `SEDEXCLK_USED` | D for ordinary public mappings on LFE5U-85F; generic unsupported-resource guard retained. No inferred trigger demonstrated. |

## Non-macro refusals and unmeasured annotations

These were inspected in addition to the explicit macro ledger. A decoder status becomes a planner error only when its resource/path is needed. Comments or status strings that do not stop execution are identified as such.

| Site(s) | Message / boundary | Class and conclusion |
|---|---|---|
| `src/decode/pio.rs:136` | `no_base_type` | E: fixed catalogue must contain one BASE_TYPE per PIO; not requested by RTL. |
| `src/decode/pio.rs:143,149` | `unsupported_overlay` | D for this pinned catalogue: closure over fields uses only WRITE_ORDER, maximum 37 covered bits, below the 128-bit representation limit. |
| `src/decode/pio.rs:228,422` | `unmatched` | D for malformed/other-writer CRAM; qualified public writer assignments have a witness in the decoder's model. Not a claim of full support for all electrical types. |
| `src/decode/pio.rs:275,295` | `search_limit` | D for this catalogue, by a conservative all-pattern bound: see below. |
| `src/decode/pio.rs:426,435` | `ambiguous`, `ambiguous_complement` | C qualification boundary for unusual electrical/pair settings; propagated via PIO-mode checks. R03 native differential input/output aliases and ordinary LVCMOS aliases execute; ambiguous ownership remains refused. |
| `src/decode/ebr.rs`, `src/decode/dsp.rs` | no bail/ensure/Err feature refusal | Overlay/mode decoding retains ambiguous/unmatched/raw status; planner guards in the ledger decide whether a used mode executes. No hidden standalone “not implemented” branch was found in these two files. |
| `src/compile/oscillator.rs:115` | `OSC.DIV bits match …; no measured divider for them` | D for legal OSCG DIV=2…128: the table covers every emitted bit pattern (128 is written as 127). Invalid DIV encodings fail pack; DIV=3 control passes. |
| `src/compile/mod.rs:817` | RAMW slice-C FF with SD=1 gets X data | Explicitly recorded boundary, not a bail; normal packer reserves the C combinational positions. Fast unknown-read policy can expose X if such a raw/profile path is observed. |
| `src/compile/dsp.rs:39,375,407,959`; `src/kernel/alu.rs:38,392` | flag profile, unknown stages, unsupported operand-only ALU execution, unmeasured flag reset/SIGNEDR | Actual blockers are R21–R24/R32. Some unmeasured values remain X or measured constant flags rather than refusing the whole plan. SIGNEDR is not promised binary. |
| `src/compile/dcs.rs:38`; `src/compile/dtr.rs:26` | descriptive resource reports | DCSC still hits R16 (`src/kernel/fast.rs:339` at bf47e64). The DTR report string in source retains obsolete X/fast-refusal wording; it is stale metadata, not an executable refusal. Commit 1c79418 removed the DTR fast exclusion and added host-scheduled completion. This docs-only follow-up leaves that source string untouched. |
| `src/fast.rs:2002,2307,2547,2610` | performance candidate `not measured` | E / performance bookkeeping, not a feature refusal. Failed tuning candidates keep their root error and do not create another unsupported mode. |
| fallible `.context`/`with_context` for missing native ports, peers, owner fields, timer/controller state and cache options | missing metadata, failed IO or arithmetic/time overflow | Fixed-topology ones are E; invalid external requests/environment are D. The OSC divider boundary above is the only additional unmeasured-mode context requiring separate feature analysis. |

**PIO bound.** A scratch-only script merged both PIO-containing configuration catalogues, applied the same writer-bit closure, and grouped shared PIOs. There are 201 groups (164 pairs and 37 singles). Every BASE_TYPE exists; no closure pulls in a field outside WRITE_ORDER; maximum covered size is 37 bits. After the decoder's exclusive-bit filtering, group every option (including unspecified/default) by its exclusive-bit value, take the largest bucket per field, and multiply these independent maxima. This deliberately overestimates every possible observed-CRAM assignment. Maximum per-cell bound is **3240**, maximum group-product bound **9,797,760**, below SEARCH_LIMIT=20,000,000 even before private-cell filtering, nextpnr-write restrictions or role checks. Thus neither search-limit branch is a student gap in this pinned catalogue. This is a structural bound, not a sampling claim based only on the example IOs.


## Exhaustive refusal-site ledger

Every explicit macro site is listed below; repeated text is grouped by R/D/E ID. Line numbers are for the final frozen audited source snapshot. E rows are the skipped invariants, with the reason for skipping stated in the final column. D rows are retained even when they are merely validation rather than feature refusals. A line can reject several profiles: its most student-relevant class is shown, with other classes called out in its reason. This avoids inventing a separate feature for every formatting variant.

| Site | Message template | Class / group | Reachability / reason |
|---|---|---|---|
| `src/compile/config_ports.rs:86` | `unsupported {field} directive {value}; expected DISABLE or ENABLE` | **D**, D-file | Only an invalid text directive; normal nextpnr enums accept DISABLE/ENABLE or packing rejects first. |
| `src/compile/config_ports.rs:103` | `unresolved {field}: {values:?}` | **E**, E-catalog | Pinned writer emits coherent enum replicas; malformed/catalogue state, not an RTL feature. |
| `src/compile/config_ports.rs:109` | `inconsistent {field} CRAM replicas` | **E**, E-catalog | Pinned writer emits coherent enum replicas; malformed/catalogue state, not an RTL feature. |
| `src/compile/config_ports.rs:146` | `{}=ENABLE retains package {} ball {} ({}); user IO is not supported` | **C**, R29 | Explicit configuration-port retention plus conflicting user-pin/USRMCLK ownership; not default synthesis. |
| `src/compile/config_ports.rs:187` | `{}=ENABLE retains package {} ball {} (CCLK); ownership conflicts with configured USRMCLK` | **C**, R29 | Explicit configuration-port retention plus conflicting user-pin/USRMCLK ownership; not default synthesis. |
| `src/compile/dcs.rs:16` | `ambiguous DCSMODE bits: {modes:?}` | **E**, E-catalog | NONE/NEG aliases must decode to the same implemented mode; catalogue consistency. |
| `src/compile/dsp.rs:98` | `unknown multiplier execution view` | **E**, E-catalog | Serialized view / fixed DSP topology, word length or qualified bypass representation invariant. |
| `src/compile/dsp.rs:110` | `unsupported MULT18 output {output}; shift/C forwarding is not implemented` | **C**, R20 | Explicit DSP source/shift/C forwarding profile; not emitted for inferred multiplication. |
| `src/compile/dsp.rs:120` | `inactive/unsupported multiplier mode` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:136` | `unsupported ALU54 mode sharing the DSP slice` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:148` | `active ALU54 outside the measured profile ({:#}): only ROA/ROB operand observation is implemented` | **C**, R21–R24 | Wrapper refusal after ALU settings fail: ROA/ROB-only observation remains possible; product/ALU execution cannot silently fall back. |
| `src/compile/dsp.rs:151` | `DSP slice CIB output is disabled` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:162` | `unknown MULT18 output path` | **E**, E-catalog | Serialized view / fixed DSP topology, word length or qualified bypass representation invariant. |
| `src/compile/dsp.rs:166` | `registered MULT18 SIGNEDP is unresolved D01/D05; the product output stage does not model SIGNEDP` | **C**, R21 | Explicit standalone INPUTC or registered product sign profile; ordinary Yosys MULT stages bypassed. |
| `src/compile/dsp.rs:222` | `MULT18 unsupported/unresolved {stage} stage: {choices:?}` | **C**, R21 | Explicit standalone INPUTC or registered product sign profile; ordinary Yosys MULT stages bypassed. |
| `src/compile/dsp.rs:228` | `MULT18 C/dynamic/highspeed source mode is not implemented` | **C**, R20 | Explicit DSP source/shift/C forwarding profile; not emitted for inferred multiplication. |
| `src/compile/dsp.rs:270` | `invalid DSP CE selection` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:274` | `invalid DSP reset selection` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:282` | `unsupported DSP RESETMODE` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:303` | `unsupported DSP GSR` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:324` | `invalid DSP output reset selection` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:331` | `unsupported DSP RESETMODE` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:411` | `mixed ENABLED/DISABLED DSP register cells are not measured (HD09)` | **C**, R25 | Explicit mixed disabled/enabled register divider or shared ALU/MULT disagreement. |
| `src/compile/dsp.rs:447` | `unsupported DSP {clock}_DIV={value}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:452` | `mixed ENABLED/DISABLED DSP register clocks are not measured (HD09)` | **C**, R25 | Explicit mixed disabled/enabled register divider or shared ALU/MULT disagreement. |
| `src/compile/dsp.rs:478` | `ALU/MULT18 CLK_DIV disagreement in one DSP slice is not measured (HD09)` | **C**, R25 | Explicit mixed disabled/enabled register divider or shared ALU/MULT disagreement. |
| `src/compile/dsp.rs:483` | `ALU/MULT18 CLK_DIV disagreement cannot be resolved (HD09)` | **C**, R25 | Explicit mixed disabled/enabled register divider or shared ALU/MULT disagreement. |
| `src/compile/dsp.rs:501` | `mixed ENABLED/DISABLED DSP register clocks in one slice are not measured (HD09)` | **C**, R25 | Explicit mixed disabled/enabled register divider or shared ALU/MULT disagreement. |
| `src/compile/dsp.rs:520` | `DSP slice without two MULT18` | **E**, E-catalog | Serialized view / fixed DSP topology, word length or qualified bypass representation invariant. |
| `src/compile/dsp.rs:526` | `ALU54 is not in MODE ALU54B` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:533` | `ALU54B {field}={value}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:559` | `ALU54B {field}: {values:?}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:576` | `ALU54B {field}={value}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:592` | `ALU54B REG_OPCODEIN_1_CLK={clock} is not understood: on the board (HD08 o4) its values follow no register model in 6% of the clocks` | **C**, R22 | Explicit second opcode IN stage; HD08 exception. |
| `src/compile/dsp.rs:616` | `ALU54B REG_OUTPUT0_RST={first_reset}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:620` | `ALU54B REG_OUTPUT0_RST={first_reset} and REG_OUTPUT1_RST={other_reset} differ: not measured` | **C**, R23 | Explicit output clock/reset mismatch or shared ALU/MULT C reset mismatch. |
| `src/compile/dsp.rs:629` | `unsupported ALU54B GSR {other}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:638` | `ALU54B {field}={value} is not measured (only {fallback})` | **C**, R24 | Explicit pattern/legacy/barrel/CO/flag configuration beyond the measured ALU profile. |
| `src/compile/dsp.rs:665` | `ALU54B {name} is short` | **E**, E-catalog | Serialized view / fixed DSP topology, word length or qualified bypass representation invariant. |
| `src/compile/dsp.rs:712` | `unresolved CIBOUT_BYP beside an ALU54B: {} (raw {})` | **E**, E-catalog | Serialized view / fixed DSP topology, word length or qualified bypass representation invariant. |
| `src/compile/dsp.rs:727` | `unsupported DSP RESETMODE {other}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:739` | `ALU54B C register with RESETMODE differing between the ALU and its MULT18 is not measured` | **C**, R23 | Explicit output clock/reset mismatch or shared ALU/MULT C reset mismatch. |
| `src/compile/dsp.rs:753` | `ALU54B output registers on two clocks are not measured` | **C**, R23 | Explicit output clock/reset mismatch or shared ALU/MULT C reset mismatch. |
| `src/compile/dsp.rs:803` | `unsupported ALU54 output {port} (CO is not measured)` | **C**, R24 | Explicit pattern/legacy/barrel/CO/flag configuration beyond the measured ALU profile. |
| `src/compile/dsp.rs:817` | `flag {port} of an inactive ALU54: {key}` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dsp.rs:845` | `ALU54B {key}: registered flags beside CIBOUT_BYP of the second MULT18 are not measured` | **C**, R24 | Explicit pattern/legacy/barrel/CO/flag configuration beyond the measured ALU profile. |
| `src/compile/dsp.rs:964` | `unsupported MULT18 {port}MUX` | **D**, D-DSP | Legal public DSP writer has the handled modes/enum values and CIBOUT=ON; malformed/inactive/invalid choices do not encode a new feature. |
| `src/compile/dtr.rs:42` | `unsupported DTR.MODE {mode}` | **D**, D-enum | Writer only emits DTR for a live DTR; NONE is handled before this branch. |
| `src/compile/ebr.rs:42` | `unsupported EBR {port}MUX: {choice}` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:64` | `unresolved EBR CSDECODE` | **E**, E-catalog | CSDECODE is a fixed three-bit resolved catalogue word. |
| `src/compile/ebr.rs:81` | `reachable inactive/unsupported EBR mode {mode}` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:96` | `unsupported EBR width {widths:?}` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:107` | `unknown EBR output {output}` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:115` | `EBR output {output} outside validated data view` | **C**, R26 | Explicit out-of-width data pin or mismatched wide-PDP output register modes. |
| `src/compile/ebr.rs:127` | `unsupported EBR reset mode` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:130` | `unsupported EBR reset release` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:135` | `unsupported EBR GSR` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:150` | `unsupported EBR write mode` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:156` | `unsupported EBR output register mode` | **D**, D-EBR | Legal DP/PDP writer emits only handled widths/modes/control enums; invalid strings fail tool/pack first. |
| `src/compile/ebr.rs:162` | `36-bit PDP read with REGMODE_A != REGMODE_B is not measured` | **C**, R26 | Explicit out-of-width data pin or mismatched wide-PDP output register modes. |
| `src/compile/ebr.rs:176` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/compile/eclk.rs` (`execute_eclksync`, `qualify_eclksync_none`) | Historical MODE NONE refusal removed; unknown MODE and MODE=NONE with connected STOP still name the setting | **R09 fixed**; remaining enum/profile checks | X13 now executes; HI12 confirms the qualified buffer. HI13 closes enabled pin-clock startup with source-dependent initial stages; PLL/OSCG history remains unmeasured. |
| `src/compile/eclk.rs:75` | `ECLKBRIDGECS {key} MODE {mode} is not implemented` | **D**, D-enum | Writer has NONE/ECLKBRIDGECS and divider 2.0/3.5; those modes are handled. |
| `src/compile/eclk.rs:108` | `CLKDIVF {key} DIV {div} is not a CLKDIVF ratio` | **D**, D-enum | Writer has NONE/ECLKBRIDGECS and divider 2.0/3.5; those modes are handled. |
| `src/compile/eclk.rs:111` | `CLKDIVF {key} GSR={gsr} is not measured` | **C**, R18 | Explicit divider GSR enabled, unlike nextpnr default. |
| `src/compile/eclk.rs:234` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/compile/eclk.rs:367` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/compile/iologic.rs:44` | `unsupported PIO path {key}:{name}={value}` | **D**, D-IO | Invalid PIO/IO mux name/path/REGSET enum; handled public packer profiles do not emit another choice. |
| `src/compile/iologic.rs:48` | `IOLOGIC memory path is not implemented: {key}:{name}` | **D**, D-IO | Invalid PIO/IO mux name/path/REGSET enum; handled public packer profiles do not emit another choice. |
| `src/compile/iologic.rs:55` | `two PIO paths select the IOLOGIC: {key}` | **D**, D-IO | Invalid PIO/IO mux name/path/REGSET enum; handled public packer profiles do not emit another choice. |
| `src/compile/iologic.rs:100` | `unqualified shared IOLOGIC sources: {:?}` | **C**, R28 | Qualification boundary for an unhandled explicit IOLOGIC combination/alias; no normal inferred-FF trigger demonstrated. |
| `src/compile/iologic.rs:143` | `{} {name} is ambiguous: {:?}` | **C**, R28 | Qualification boundary for an unhandled explicit IOLOGIC combination/alias; no normal inferred-FF trigger demonstrated. |
| `src/compile/iologic.rs:159` | `unsupported IOLOGIC {mux}={value}: {key}` | **C**, R28 | Omitted SCLK can make CLKIMUX/CLKOMUX=0; deliberately incomplete explicit IO primitive, not a normal clocked register. |
| `src/compile/iologic.rs:201` | `IOLOGIC output {port} is not implemented: {key}` | R08 IOLDOD case **fixed** | IOLDOD now executes with zero propagation time (HI11); this generic guard remains for other unsupported outputs. |
| `src/compile/iologic.rs:195` | `{key} MODE is ambiguous: {modes:?}` | **C**, R28 | Qualification boundary for an unhandled explicit IOLOGIC combination/alias; no normal inferred-FF trigger demonstrated. |
| `src/compile/iologic.rs:233` | `{key} MODE={mode} drives {path}, but the PIO does not select it with {field}` | **E**, E-routing | Packer must select the same PIO path as the IOLOGIC mode it created. |
| `src/compile/iologic.rs:268` | `IOLOGIC {name} is not implemented: {key} (MODE {mode}, candidates {values:?})` | **B**, R04 | Default public OSHX2A produces the refused memory submode; other raw unsupported MODE/output combinations at the latter site are C. |
| `src/compile/iologic.rs:280` | `IOLOGIC GSR={gsr} is not measured for MODE={mode}: {key}` | **C**, R19 | Nondefault gearbox GSR, raw DDR SRMODE or CE; normal X1/SDR profiles are handled. |
| `src/compile/iologic.rs:284` | `IOLOGIC SRMODE={srmode} is not measured for MODE={mode}: {key}` | **C**, R19 | Nondefault gearbox GSR, raw DDR SRMODE or CE; normal X1/SDR profiles are handled. |
| `src/compile/iologic.rs:320` | `IOLOGIC DELAY.WAIT_FOR_EDGE is not implemented (requires DISABLED): {key}` | **D**, R08 OUTDEL case fixed | OUTDEL is accepted (HI11); only the named WAIT_FOR_EDGE setting remains refused and is not enabled by public DELAYG/F packing. |
| `src/compile/iologic.rs:321` | `IOLOGIC clock enable CE{direction}MUX={value} is not measured for MODE={mode}: {key}` | **C**, R19 | Nondefault gearbox GSR, raw DDR SRMODE or CE; normal X1/SDR profiles are handled. |
| `src/compile/iologic.rs:328` | `unsupported IOLOGIC LSR{direction}MUX={value}: {key}` | **D**, D-IO | Invalid PIO/IO mux name/path/REGSET enum; handled public packer profiles do not emit another choice. |
| `src/compile/iologic.rs:348` | `IOLOGIC latch is not implemented: {key} {mode}` | **C**, R28 | Explicit raw latch, gearbox output mix or TDDR profile; public FF packer does not infer these. |
| `src/compile/iologic.rs:355` | `unsupported IOLOGIC {name}={value}: {key}` | **D**, D-IO | Invalid PIO/IO mux name/path/REGSET enum; handled public packer profiles do not emit another choice. |
| `src/compile/iologic.rs:361` | `IOLOGIC MODE={mode} output {path} is not implemented: {key}` | **C**, R28 | Explicit raw latch, gearbox output mix or TDDR profile; public FF packer does not infer these. |
| `src/compile/iologic.rs:378` | `IOLOGIC latch is measured only with SRMODE=ASYNC, GSR=DISABLED and REGSET=RESET (HI10): {key}` | **C**, R28 | Explicit raw latch, gearbox output mix or TDDR profile; public FF packer does not infer these. |
| `src/compile/iologic.rs:435` | `IOLOGIC IOLTOMUX {values:?} is not implemented: {key}` | **C**, R28 | Explicit raw latch, gearbox output mix or TDDR profile; public FF packer does not infer these. |
| `src/compile/iologic.rs:463` | `IOLOGIC MODE={mode} output {path} is not implemented: {key}` | **B**, R04 | Default public OSHX2A produces the refused memory submode; other raw unsupported MODE/output combinations at the latter site are C. |
| `src/compile/lutram.rs:42` | `unsupported mixed LUTRAM modes: {slice} must be {expected}` | **D**, D-LUTRAM | Packer owns A/B DPRAM+C RAMW, writes C INIT=0, and only handled CLK/WRE encodings. |
| `src/compile/lutram.rs:48` | `nonzero/unknown C INIT in RAMW needs a separate model` | **D**, D-LUTRAM | Packer owns A/B DPRAM+C RAMW, writes C INIT=0, and only handled CLK/WRE encodings. |
| `src/compile/lutram.rs:57` | `unsupported LUTRAM CLK1 polarity` | **D**, D-LUTRAM | Packer owns A/B DPRAM+C RAMW, writes C INIT=0, and only handled CLK/WRE encodings. |
| `src/compile/lutram.rs:62` | `unsupported LUTRAM WREMUX` | **D**, D-LUTRAM | Packer owns A/B DPRAM+C RAMW, writes C INIT=0, and only handled CLK/WRE encodings. |
| `src/compile/lutram.rs:69` | `inconsistent shared WREMUX/LSR1 inversion bit` | **E**, E-owner | Shared inversion bit / single execution owner consistency. |
| `src/compile/lutram.rs:129` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/compile/lutram.rs:144` | `LUTRAM column already has an execution owner` | **E**, E-owner | Shared inversion bit / single execution owner consistency. |
| `src/compile/mod.rs:58` | `empty pin reference` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:64` | `use sys: references for logical system controls` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:71` | `reference is outside this device` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:217` | `reference is outside execution plan` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:351` | `oscillator frequency must be positive` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:361` | `{} is read-only or unavailable in this startup profile` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:373` | `two initial drivers name the same boundary; use one reference` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:393` | `board wires connect package pins` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:481` | `observed scope requires at least one probe` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:544` | `unknown package {}` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:658` | `ambiguous/unmatched {}:{name}: {values:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:721` | `package ball has no confirmed GPIO or CCLK binding` | **D**, D-command | Reference/package/request/environment validation; no RTL construct can itself supply an invalid command or board API request. |
| `src/compile/mod.rs:745` | `ambiguous control selector {selector}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:780` | `unsupported control inversion {attribute}={value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:791` | `unsupported input mux {slice}.{local}MUX={value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:808` | `unsupported LSRMODE {value}: {base}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:830` | `unknown SD {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:844` | `unsupported CE {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:850` | `unsupported SRMODE {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:858` | `unsupported REGSET {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:872` | `unsupported slice GSR {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:904` | `unsupported PLC mode {slice}={mode}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:909` | `OFX in CCU2 mode needs a separate qualified model` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:966` | `unsupported node output {}.{port} in {mode}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:983` | `unsupported INJECT1` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:994` | `unsupported node {}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1008` | `missing or multiple composed fields: {suffix}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1013` | `unresolved resource field {}: {}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:1046` | `ambiguous {suffix}: {values:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:1059` | `missing or multiple qualified fields` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1088` | `{} PIO mode {key}: {values:?}` | **C**, remaining electrical qualification | Differential INPUT/OUTPUT execute after R03; unqualified single-ended electrical/overlay combinations remain explicit. |
| `src/compile/mod.rs:1104` | `inactive/unknown or ambiguous BASE_TYPE {key}: {values:?}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs` (`pio_differential`) | historical blanket differential refusal removed; specific bidirectional/ambiguous ownership/direction errors remain | R03 **fixed** for INPUT/OUTPUT; remaining checks **C**, with IO_TYPE alternatives and reason. |
| `src/compile/mod.rs:1126` | `unsupported electrical BASE_TYPE: {values:?}` | **C**, remaining electrical qualification | Differential INPUT/OUTPUT execute after R03; unqualified single-ended electrical/overlay combinations remain explicit. |
| `src/compile/mod.rs:1162` | `unsupported PULLMODE {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1183` | `unsupported OPENDRAIN {value}` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1187` | `PIO {key} has no digital direction` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs` (`pio_complement`) | `complement without differential IO_TYPE` | **D**, invariant/ownership check; differential INPUT/OUTPUT aliases are supported by R03. |
| `src/compile/mod.rs` (`pio`, `pio_complement`) | `unsupported differential IO_TYPE ... open-drain pair has no complementary push-pull digital model` | **C**: differential open-drain output remains refused with the IO_TYPE and reason. |
| `src/compile/mod.rs:1341` | `ambiguous PIO {} {name}: {values:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:1351` | `unknown PIO output` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1353` | `PIO receiver is {} in this mode` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1412` | `DCC MODE {mode} is not implemented` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1421` | `unsupported reachable resource {} ({}) output {port}` | **B**, R05 / R10 | DDRDLLA, DQSBUFM and JTAGG experimentally refused; absent small-DSP/other-family producers are D for normal public mappings. |
| `src/compile/mod.rs:1444` | `PLL {key} {name} is ambiguous: {:?}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1459` | `PLL {key} MODE is not EHXPLLL` | **D**, D-enum/topology | Public packer emits the already-handled enum/output/direction combinations. CCU2 wide-OFX/unsupported node shapes are not ordinary mappings; pseudo-differential mismatches are normally rejected earlier. |
| `src/compile/mod.rs:1478` | `PLL {key} {name}={value} is not implemented (only {expected})` | **C**, R27 | Explicit PLL feature/divider/trim/dynamic output/feedback or REFCLK observation beyond supported static profile. |
| `src/compile/mod.rs:1557` | `PLL {key} output {port} is disabled` | **C**, R27 | Explicit PLL feature/divider/trim/dynamic output/feedback or REFCLK observation beyond supported static profile. |
| `src/compile/mod.rs:1572` | `PLL {key} output {port} is not implemented` | **C**, R27 | Explicit PLL feature/divider/trim/dynamic output/feedback or REFCLK observation beyond supported static profile. |
| `src/compile/mod.rs:1598` | `PLL {key} CLKFB is not fed by one of its outputs; user feedback is not implemented` | **C**, R27 | Explicit PLL feature/divider/trim/dynamic output/feedback or REFCLK observation beyond supported static profile. |
| `src/compile/mod.rs:1640` | `multiple native producers for {wire:?}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1665` | `ambiguous CIB constant selection {field}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1675` | `unqualified CIB clock/LSR constant encoding {field}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1706` | `conflicting physical CIB constant fragments at {wire:?}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1756` | `DSP preadder path is not implemented at {wire:?}` | **D**, D-DSP | No PRADD mapping in the pinned public writer or inferred multiplier mapping; requires raw internal preadder routing. |
| `src/compile/mod.rs:1800` | `incoming index mismatch` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/mod.rs:1814` | `ambiguous physical mux candidates: {:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:1857` | `unqualified fixed/mux overlap at {wire:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs` (ECLKSYNC internal-arc guard) | Historical blanket bypass refusal replaced by unique-owner and MODE/STOP qualification | **R09 fixed**; remaining profile/invariant checks | Only qualified MODE NONE with unrouted STOP takes the buffer path (HI12); the fixed link never bypasses enabled STOP state. |
| `src/compile/mod.rs:1904` | `unqualified multiple fixed sources at {wire:?}` | **C**, R28 / qualification | Explicit unusual mode/routing/electrical alias outside the qualified combinations; not an established behavioural-RTL gap. Some malformed/edited images also hit these checks. |
| `src/compile/mod.rs:1966` | `IO tile R{}C{} has PIOs in banks {banks:?}` | **E**, E-routing | Pinned metadata cardinality / single selected producer or coherent physical encoding invariant; bug/corrupt metadata, not a feature selection. |
| `src/compile/oscillator.rs:82` | `OSCG output {port} is not implemented: {key}` | **D**, D-enum | Only OSC output and OSCG active mode are emitted by the public writer. |
| `src/compile/oscillator.rs:96` | `{key} OSC.MODE {:?} is not OSCG` | **D**, D-enum | Only OSC output and OSCG active mode are emitted by the public writer. |
| `src/compile/system.rs:39` | `unresolved global user GSR field {}` | **E**, E-catalog | Fixed system resource cardinality / catalogue field / replicated DONE_EX consistency. |
| `src/compile/system.rs:55` | `expected one system tile {kind}` | **E**, E-catalog | Fixed system resource cardinality / catalogue field / replicated DONE_EX consistency. |
| `src/compile/system.rs:66` | `multiple global GSR resources` | **E**, E-catalog | Fixed system resource cardinality / catalogue field / replicated DONE_EX consistency. |
| `src/compile/system.rs:70` | `ambiguous global user GSR mode: {modes:?}` | **D**, D-enum | Normal writer emits NONE or one legal GSR mode; invalid/ambiguous raw image is not a separate inferred feature. |
| `src/compile/system.rs:101` | `unsupported or ambiguous global user GSR synchronization` | **D**, mode qualification | ASYNC/NONE aliases and a unique SYNC are accepted; malformed/ambiguous synchronization is refused. R15 public SGSR is implemented. |
| `src/compile/system.rs:107` | `synchronous global user GSR requires HY01-measured ACTIVE_LOW polarity` | **C**, remaining measured-profile boundary | ACTIVE_HIGH/NONE with SYNC is outside HY01; ASYNC retains its supported polarities. `automatic_compile` and `system_controls` check this qualification. |
| `src/compile/system.rs:129-132` | `synchronous global user GSR has no CLK pin / no CLK route` | **E**, missing catalogue pin; **C**, unrouted profile | A real routed SGSR CLK is required; `sgsr_native` checks a deliberately disconnected clock. |
| `src/compile/system.rs:85` | `unsupported user GSR mode {mode}` | **D**, D-enum | Normal writer emits NONE or one legal GSR mode; invalid/ambiguous raw image is not a separate inferred feature. |
| `src/compile/system.rs:111` | `inconsistent DONE_EX fragments` | **E**, E-catalog | Fixed system resource cardinality / catalogue field / replicated DONE_EX consistency. |
| `src/compile/system.rs:119` | `custom wake phase {field} is unsupported` | **C**, R29 | Explicit nondefault wake fields, evaluated only for wake-phases startup. |
| `src/compile/system.rs:127` | `unsupported wake sequence/DONE_EX/DONEPHASE combination: {sequence}/{done}/{done_phase}` | **C**, R29 | Explicit nondefault wake fields, evaluated only for wake-phases startup. |
| `src/compile/timing.rs:128` | `design does not receive the clock` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/timing.rs:182` | `cached signal reused a foreign scope` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/timing.rs:187` | `cached driver reused a foreign scope` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/timing.rs:194` | `kernel snapshot differs after cache read` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/timing.rs:209` | `cache state diverged` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/timing.rs:215` | `cache probe diverged at {name}` | **E**, E-test | Test-only compile-timing benchmark assertions, not compiled into ordinary planner execution. |
| `src/compile/usrmclk.rs:21` | `expected one CCLK ball in package {package}` | **E**, E-catalog | Fixed package/system resource cardinality. |
| `src/compile/usrmclk.rs:38` | `expected one USRMCLK resource` | **E**, E-catalog | Fixed package/system resource cardinality. |
| `src/compile/usrmclk.rs:49` | `expected one CCLK EFB3_PICB1 tile` | **E**, E-catalog | Fixed package/system resource cardinality. |
| `src/compile/usrmclk.rs:63` | `unresolved EFB3_PICB1 CCLK.MODE: {values:?}` | **D**, D-enum | Pinned writer selects USRMCLK or inactive CCLK; unknown raw CCLK mode is not a normal public mapping. |
| `src/decode.rs:85` | `unsupported CRAM snapshot format` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:89` | `invalid CRAM dimensions` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:97` | `CRAM snapshot length differs from dimensions` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:106` | `invalid CRAM hex digit` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:115` | `ECP5 BRAM data exceeds nine bits` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:146` | `CRAM import failed: {}` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:398` | `configuration catalogue belongs to another device/layout` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:412` | `conflicting resource family {}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:420` | `conflicting field kind {}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:431` | `conflicting fragment {}:{}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:468` | `configuration fragment exceeds owner tile {}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:490` | `unsupported decode catalogue` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:494` | `invalid catalogue dimensions` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:500` | `duplicate resource ID {}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:503` | `duplicate field {}:{}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:504` | `field {} has no fragments` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:507` | `inconsistent word width {}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:514` | `catalogue bit outside CRAM: {frame}:{bit}` | **E**, E-catalog | Catalogue assembly/structure/owner bounds invariant; fixed evidence or database bug/corruption, not a mode requested by RTL. |
| `src/decode.rs:547` | `no PIO decode catalogue entry: {id}` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:570` | `CRAM/catalogue mismatch` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/decode.rs:577` | `resource has no decode catalogue entry: {id}` | **D**, D-file | Malformed/other-target image, wrong catalogue selection or failed native image reader; no Verilog feature changes snapshot format or schema. |
| `src/fast.rs:278` | `clang failed: {}` | **D**, D-environment | Compiler/loader/generated ABI availability; an environment or code-generator problem, not an HDL resource mode. |
| `src/fast.rs:309` | `dlopen: {}` | **D**, D-environment | Compiler/loader/generated ABI availability; an environment or code-generator problem, not an HDL resource mode. |
| `src/fast.rs:314` | `missing symbol {name:?}` | **D**, D-environment | Compiler/loader/generated ABI availability; an environment or code-generator problem, not an HDL resource mode. |
| `src/fast.rs:400` | `answering runs cannot tune` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:401` | `start a clock first` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:435` | `fast_step_input is missing` | **D**, D-environment | Compiler/loader/generated ABI availability; an environment or code-generator problem, not an HDL resource mode. |
| `src/fast.rs:492` | `this input cannot be set to Z` | **A**, R11 | Ordinary inferred input FF with unknown/high-impedance external stimulus; explicit Z update requires a pad that accepts it. |
| `src/fast.rs:509` | `an answer's trace cannot answer again` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:657` | `{time} is not a whole number of fast-mode ticks` | **D**, D-time/API | A caller time not on the current rational lattice; normal public set/run APIs refine the lattice first. |
| `src/fast.rs:697` | `fast mode runs one clock` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:793` | `the clock drives this input` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:800` | `fast mode is two-valued; this input cannot be set to Z` | **A**, R11 | Ordinary inferred input FF with unknown/high-impedance external stimulus; explicit Z update requires a pad that accepts it. |
| `src/fast.rs:825` | `cannot set inputs in the past` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:857` | `{} reference period changed; relock is not modelled` | **B**, R17 | Secondary externally driven PLL reference fails the lattice check experimentally; changing a learned reference period is also deliberately rejected. |
| `src/fast.rs:871` | `{} reference period is not a whole number of fast-mode steps` | **B**, R17 | Secondary externally driven PLL reference fails the lattice check experimentally; changing a learned reference period is also deliberately rejected. |
| `src/fast.rs:901` | `cannot run backwards` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1074` | `the schedule has no clock edge` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1242` | `fast mode stopped at {at}: pad {pad} is driven by the design and from outside with different levels; the event kernel makes it X` | **A**, R14 | Dynamic contention on an inferred pad, proved at 90 ns; not an independent unsupported primitive. |
| `src/fast.rs:1255` | `fast mode stopped at {at}: asynchronous PRLD register {unit} released LSR while M changed in the same instant; the event kernel makes Q unknown` | **D**, D-PRLD | Runtime asynchronous PRLD release/M-change corner; default synth_ecp5 rejects inferred nonconstant async load, so only explicit raw PRLD (C) can reach it through HDL. |
| `src/fast.rs:1259` | `fast mode stopped at {at}: the logic reads an output of EBR {ebr} that the event kernel makes unknown here (a cross-port collision on one row, a control combination the board has not shown, or a read of unknown memory); fast mode is two-valued, so run this with the event kernel` | **A**, R02 / R13 | Original PDP36/X03 refusal fixed by `be71ccb` hunk A; independent dual-port collisions and unmeasured/unknown cases still use this diagnostic. Selected-demand hunk B remains held. Earlier Err(self.failure()) sites propagate this diagnostic. |
| `src/fast.rs:1265` | `fast mode stopped at {at}: EBR {ebr} has an active local reset with synchronous release, which the event kernel rejects` | **C**, R26 | Explicit asynchronous EBR reset with synchronous release; same event-kernel restriction. |
| `src/fast.rs:1269` | `fast mode did not converge at {at}: 10000 capture waves without a stable state` | **D**, D-convergence | Runtime 10000-capture-wave guard; ordinary combinational loops rejected by default nextpnr. Exotic sequential zero-time loops or enormous networks are a residual possibility, not an invariant or proved unreachable maximum-depth bound. |
| `src/fast.rs:1345` | `the schedule has no clock edge` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1720` | `choosing fast mode's settings was cancelled` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1734` | `choosing fast mode's settings was cancelled` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1740` | `choosing fast mode's settings was cancelled` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/fast.rs:1883` | `choosing fast mode's settings was cancelled` | **D**, D-command | Host API use, timestamp/schedule request or cancellation; no HDL-only trigger. One host clock() does not limit internal design clock domains. |
| `src/kernel/alu.rs:395` | `resource is not an ALU54` | **E**, E-owner | Planner/builder type, ownership and 54-bit disjoint storage-mask invariants. |
| `src/kernel/alu.rs:396` | `ALU54 already has an execution owner` | **E**, E-owner | Planner/builder type, ownership and 54-bit disjoint storage-mask invariants. |
| `src/kernel/alu.rs:400` | `ALU54 word wider than 54 bits` | **E**, E-owner | Planner/builder type, ownership and 54-bit disjoint storage-mask invariants. |
| `src/kernel/alu.rs:407` | `an ALU54 bit is both unregistered and without a clock` | **E**, E-owner | Planner/builder type, ownership and 54-bit disjoint storage-mask invariants. |
| `src/kernel/cache.rs:252` | `trailing plan payload` | **E**, E-cache | Serialized cache payload and wire-ID consistency, not a resource feature. |
| `src/kernel/cache.rs:268` | `duplicate cached wire` | **E**, E-cache | Serialized cache payload and wire-ID consistency, not a resource feature. |
| `src/kernel/dcs.rs:37` | `unsupported DCSMODE {value}` | **D**, D-enum | All legal public DCS modes including NONE/NEG alias are parsed; invalid enum rejected by tools or malformed raw input. |
| `src/kernel/dsp.rs:266` | `MULT18 has no execution owner` | **E**, E-owner | Multiplier execution owner/view/type invariant. |
| `src/kernel/dsp.rs:299` | `MULT18 operand-only view cannot bind product/output register stages` | **E**, E-owner | Multiplier execution owner/view/type invariant. |
| `src/kernel/dsp.rs:312` | `resource is not a MULT18` | **E**, E-owner | Multiplier execution owner/view/type invariant. |
| `src/kernel/dsp.rs:313` | `MULT18 already has an execution owner` | **E**, E-owner | Multiplier execution owner/view/type invariant. |
| `src/kernel/dsp.rs:509` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/dsp.rs:574` | `MULT18 {} uses unknown/nonparallel SOURCEA/B; shift, C and highspeed paths are not implemented` | **C**, R20 | Explicit nonparallel SOURCEA/B or unknown control; inferred multiplier hardwires both zero. |
| `src/kernel/ebr.rs:126` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/ebr.rs:132` | `invalid EBR data coordinate` | **E**, E-owner | Builder width/coordinate/type/decode/owner invariants; planner validates public EBR configuration first. |
| `src/kernel/ebr.rs:251` | `resource is not an EBR` | **E**, E-owner | Builder width/coordinate/type/decode/owner invariants; planner validates public EBR configuration first. |
| `src/kernel/ebr.rs:255` | `EBR already has an execution owner` | **E**, E-owner | Builder width/coordinate/type/decode/owner invariants; planner validates public EBR configuration first. |
| `src/kernel/ebr.rs:266` | `unsupported EBR port width {}` | **E**, E-owner | Builder width/coordinate/type/decode/owner invariants; planner validates public EBR configuration first. |
| `src/kernel/ebr.rs:271` | `CSDECODE exceeds three bits` | **E**, E-owner | Builder width/coordinate/type/decode/owner invariants; planner validates public EBR configuration first. |
| `src/kernel/ebr.rs:416` | `EBR {} has active/unknown local reset with unsupported synchronous release` | **C**, R26 | Conditional runtime check: ASYNC local reset + SYNC release only fails when reset is active or unknown. |
| `src/kernel/fast.rs:339` (bf47e64; originally :327) | `fast mode does not run DCSC (HC02); the event kernel does` | **B**, R16 | DCSC alone remains excluded from fast code. X22 records the routed proof; DTR has been supported since 1c79418. |
| `src/kernel/fast.rs:348` | `fast mode does not drive the system controller ({})` | **D**, D-command | Externally driving sys:* controller boundaries, e.g. wake-phases; not a netlist resource inference. |
| `src/kernel/fast.rs:365` | `fast mode needs one active driver per net; R{}C{}:{} has {} gates, {registers} registers and {} external drivers` | **D**, D-driver/API | Single collapsed driver invariant / inappropriate external drive of an output-only pad; ordinary Yosys merges tri-state drivers to one mux. A custom board/API can exceed this supported boundary. |
| `src/kernel/fast.rs:414` | `fast mode is two-valued; a cell of the LUTRAM read at {} is unknown` | **A**, R32 | StartsUnknown: a retry condition for inferred storage (or an explicit IO/preload profile); warm-start experiment confirms the mechanism, not a persistent whole-plan refusal. |
| `src/kernel/fast.rs:441` | `fast mode reads a constant {v}` | **A**, R01 | Narrow inferred LUTRAM proved this general unknown-term guard; explicit DSP floating cascade can also use it. |
| `src/kernel/fast.rs:454` | `fast mode reads R{}C{}:{}, which is {}` | **A**, R01 | Narrow inferred LUTRAM proved this general unknown-term guard; explicit DSP floating cascade can also use it. |
| `src/kernel/fast.rs:698` | `fast mode needs acyclic logic; {} gates are in combinational loops, one through {}` | **D**, D-loop | Default nextpnr timing rejects the tested combinational loop first. Raw/internal or --ignore-loops profiles can reach this; no ordinary latch inference because synth rejects it. |
| `src/kernel/fast.rs:739` | `fast mode does not invert a tristate net ({})` | **C**, R28 / tristate control | An explicit inverted tri-state signal or tri-state mux selector; no demonstrated ordinary inferred boundary for this internal mux representation. |
| `src/kernel/fast.rs:752` | `fast mode does not select by a tristate net ({})` | **C**, R28 / tristate control | An explicit inverted tri-state signal or tri-state mux selector; no demonstrated ordinary inferred boundary for this internal mux representation. |
| `src/kernel/fast.rs:811` | `fast mode needs one active driver per net; {} has 1 gate, 0 registers and {} external drivers` | **D**, D-driver/API | Single collapsed driver invariant / inappropriate external drive of an output-only pad; ordinary Yosys merges tri-state drivers to one mux. A custom board/API can exceed this supported boundary. |
| `src/kernel/fast.rs:818` | `fast mode does not drive a tristate pad from outside ({})` | **D**, D-driver/API | Single collapsed driver invariant / inappropriate external drive of an output-only pad; ordinary Yosys merges tri-state drivers to one mux. A custom board/API can exceed this supported boundary. |
| `src/kernel/fast.rs:831` | `fast mode needs one active driver per net; {} has 1 gate, 0 registers and {} external drivers` | **D**, D-driver/API | Single collapsed driver invariant / inappropriate external drive of an output-only pad; ordinary Yosys merges tri-state drivers to one mux. A custom board/API can exceed this supported boundary. |
| `src/kernel/fast.rs:843` | `fast mode is two-valued; input {} starts at {}` | **A**, R11 | Undriven ordinary input FF tested; binary initial stimulus is needed in fast mode. |
| `src/kernel/fast.rs:862` | `fast mode flags at most 254 tristate pads driven from outside` | **D**, D-capacity | CABGA381 has fewer than 254 externally usable tri-state pads; only a synthetic/custom board/kernel could exceed this unit encoding. |
| `src/kernel/fast.rs:1119` | `fast mode does not run logic that reads the {} of a GSR driven by logic; the event kernel does` | **D**, D-command | Reads of logical sys:gsr/user_gsr through a custom kernel/API; public GSR primitive has no fabric output pin. Its ordinary reset effect is supported. |
| `src/kernel/fast.rs:1182` | `fast mode is two-valued; the configured preload of {} is unknown` | **A**, R32 | StartsUnknown: a retry condition for inferred storage (or an explicit IO/preload profile); warm-start experiment confirms the mechanism, not a persistent whole-plan refusal. |
| `src/kernel/fast.rs:1596` | `fast mode is two-valued; an IOLOGIC stage is unknown` | **A**, R32 | StartsUnknown: a retry condition for inferred storage (or an explicit IO/preload profile); warm-start experiment confirms the mechanism, not a persistent whole-plan refusal. |
| `src/kernel/fast.rs:2035` | `fast mode refuses a pad that can float with no pull while the logic reads it ({}); the event kernel makes that read X` | **A**, R07 | Inferred readback inout without pull, experimentally refused. |
| `src/kernel/fast.rs:2900` | `fast mode is two-valued; input {} starts at {}` | **A**, R11 | Undriven ordinary input FF tested; binary initial stimulus is needed in fast mode. |
| `src/kernel/fast.rs:2924` | `fast mode is two-valued; the register output R{}C{}:{} is unknown` | **A**, R32 | StartsUnknown: a retry condition for inferred storage (or an explicit IO/preload profile); warm-start experiment confirms the mechanism, not a persistent whole-plan refusal. |
| `src/kernel/fast.rs:2935` | `fast mode does not run {} with RST/STDBY enabled (HC07); the event kernel does` | **B**, R06 | Explicit PLLRST_ENA/STDBY_ENABLE="ENABLED" with a logic/input control excludes fast mode even while deasserted (Lattice IP/Clarity profiles or hand instances). Default LiteX sets neither enable parameter and is supported. |
| `src/kernel/fast.rs:2944` | `fast mode runs {} only from a clock input; its reference comes from logic` | **B**, R12 | Ordinary PLL wrapper with behavioural divider reference, experimentally refused by fast mode only. |
| `src/kernel/fast.rs:3193` | `fast mode is two-valued; output {bit} of EBR {key} port {side} is unknown` | **A**, R32 | StartsUnknown: a retry condition for inferred storage (or an explicit IO/preload profile); warm-start experiment confirms the mechanism, not a persistent whole-plan refusal. |
| `src/kernel/fast.rs:3694` | `fast mode needs a known LUT; entry {address} is {other}` | **D**, D-file | Packed LUT INIT contains binary bits; X/Z LUT entries require edited state/raw API, not a normal ecppack bitstream. |
| `src/kernel/fast.rs:4971` | `an opaque line is kept as it is` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/fast.rs:5591` | `gear7_c is for the 7:1 gearbox paths` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/fast_dsp.rs:252` | `fast mode is two-valued; a register stage of the MULT18 {key} is unknown` | **C**, R32 / DSP | StartsUnknown for explicit internal DSP/ALU register stages; ordinary inferred MULT stages bypassed. |
| `src/kernel/fast_dsp.rs:306` | `fast mode is two-valued; a register of the ALU54 {key} is unknown` | **C**, R32 / DSP | StartsUnknown for explicit internal DSP/ALU register stages; ordinary inferred MULT stages bypassed. |
| `src/kernel/fast_dsp.rs:412` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/fast_dsp.rs:452` | `fast mode runs parallel MULT18 inputs; {} has SOURCEA/B other than 0` | **C**, R20 | Explicit nonparallel SOURCEA/B; inferred multiplication hardwires both zero. |
| `src/kernel/fast_dsp.rs:526` | `(unreachable exhaustive arm)` | **E**, E-exhaustive | Exhaustive internal enum/width dispatch; validated planner state cannot select this arm. |
| `src/kernel/fast_dsp.rs:1024` | `fast mode does not run the ALU54B {key}: its OP can read R, and some bits of R are unregistered (a loop on the chip)` | **C**, R31 | Explicit bypassed ALU result with feedback-R operation. |
| `src/kernel/iologic.rs:272` | `IO register REGSET must be 0 or 1` | **E**, E-owner | Builder REGSET value is a binary constant, fixed by planner reset/set decoding. |
| `src/kernel/mod.rs:413` | `state belongs to a different configuration` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:417` | `cannot resume a wire snapshot without driver/timer state` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:423` | `kernel handle scopes exhausted` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:455` | `delta limit must be positive` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:464` | `invalid kernel wire` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:496` | `signal belongs to a different kernel` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:509` | `empty or duplicate execution source: {label}` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:551` | `driver belongs to another builder` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:553` | `only an external driver can receive initial stimulus` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:563` | `initial FF value must be 0 or 1` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:564` | `node already has an execution owner` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:690` | `node already has an execution owner` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:719` | `node already has an execution owner` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:765` | `FF storage cannot start in high impedance; use X` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:769` | `node already has an execution owner` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:776` | `local reset value must be 0 or 1` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:783` | `global reset value must be 0 or 1` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:854` | `initial preload requires PRLD` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:939` | `wire state slot differs from kernel signal` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1091` | `signal belongs to a different kernel` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1099` | `signal belongs to a different kernel` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1109` | `kernel is stopped after a fault: {}` | **D**, D-fault | Secondary refusal to continue after an earlier fault; root error is retained, not an additional design gap. |
| `src/kernel/mod.rs:1118` | `empty or duplicate timer label` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1133` | `timer belongs to a different kernel` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1161` | `generated edge is not in the future` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1179` | `timestamp is already closed: {time} <= {}` | **D**, D-time/API | Invalid scheduling timestamp/conflicting host stimulus/time reversal; CLI sorts/coalesces events before these APIs. |
| `src/kernel/mod.rs:1189` | `invalid external driver or different kernel` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1195` | `conflicting values in one stimulus batch` | **D**, D-time/API | Invalid scheduling timestamp/conflicting host stimulus/time reversal; CLI sorts/coalesces events before these APIs. |
| `src/kernel/mod.rs:1217` | `cannot run backwards` | **D**, D-time/API | Invalid scheduling timestamp/conflicting host stimulus/time reversal; CLI sorts/coalesces events before these APIs. |
| `src/kernel/mod.rs:1227` | `message` | **D**, D-fault | Re-emits the recorded root fault from processing an event; not an independent feature. |
| `src/kernel/mod.rs:1255` | `conflicting events for driver {} at one timestamp` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/mod.rs:1292` | `did not settle after {} delta waves (possible zero-time feedback); recent source changes: {activity}` | **D**, D-convergence | 4096-delta-wave guard; tested simple loop is rejected by default nextpnr. Very deep acyclic / exotic sequential zero-time networks remain a finite-depth limitation; not skipped as E. |
| `src/kernel/mod.rs:1331` | `conflicting internal driver updates` | **E**, E-kernel | Kernel state/handle/type/ownership/event-generation invariant or Rust API caller bug; valid planner/request cannot express it. |
| `src/kernel/osc.rs:32` | `oscillator period must be positive` | **E**, E-kernel | Positive generator period invariant; planner validates positive base frequency and legal dividers. |
| `src/kernel/pll.rs:124` | `PLL frequency ratio must be positive` | **E**, E-kernel | Planner creates positive divider ratios and scheduled windows only after learning a period. |
| `src/kernel/pll.rs:125` | `PLL reference divider must be positive` | **E**, E-kernel | Planner creates positive divider ratios and scheduled windows only after learning a period. |
| `src/kernel/pll.rs:145` | `PLL output divider must be positive` | **E**, E-kernel | Planner creates positive divider ratios and scheduled windows only after learning a period. |
| `src/kernel/pll.rs:214` | `{} RST/STDBY is not binary: {previous} -> {value}` | **B**, R17 | Binary/periodic reference or reset boundary of an ordinary public PLL; changing period experimentally refuses. |
| `src/kernel/pll.rs:304` | `{} reference is not a binary clock: {previous} -> {value}` | **B**, R17 | Binary/periodic reference or reset boundary of an ordinary public PLL; changing period experimentally refuses. |
| `src/kernel/pll.rs:331` | `{} reference period changed at {now} (expected {}); relock is not modelled` | **B**, R17 | Binary/periodic reference or reset boundary of an ordinary public PLL; changing period experimentally refuses. |
| `src/kernel/pll.rs:350` | `{} window without a reference period` | **E**, E-kernel | Planner creates positive divider ratios and scheduled windows only after learning a period. |
| `src/kernel/system.rs:35` | `unknown system reference sys:{name}` | **D**, D-command | Only logical system references/stimulus can request invalid sys:* name or nonbinary step; no Verilog pin is sys:step. |
| `src/kernel/system.rs:77` | `system controller already exists` | **E**, E-owner | Unique controller / complete resume-state history invariant. |
| `src/kernel/system.rs:78` | `cannot resume system state without its controller history` | **E**, E-owner | Unique controller / complete resume-state history invariant. |
| `src/kernel/system.rs:183` | `sys:step must start at a binary level` | **D**, D-command | Only logical system references/stimulus can request invalid sys:* name or nonbinary step; no Verilog pin is sys:step. |
| `src/kernel/system.rs:208` | `sys:step must remain binary; uncertain phase progression is unsupported` | **D**, D-command | Only logical system references/stimulus can request invalid sys:* name or nonbinary step; no Verilog pin is sys:step. |
| `src/kernel/time.rs:32` | `time denominator is zero` | **D**, D-time/API | Invalid denominator/negative time in explicit host time API, not an inferred primitive. |
| `src/kernel/time.rs:64` | `time subtraction would be negative` | **D**, D-time/API | Invalid denominator/negative time in explicit host time API, not an inferred primitive. |
| `src/kernel/time.rs:75` | `time scale denominator is zero` | **D**, D-time/API | Invalid denominator/negative time in explicit host time API, not an inferred primitive. |
| `src/prepared.rs:90` | `configuration manifest and architecture disagree` | **E**, E-catalog | Pinned configuration/architecture manifest, catalogue set or ownership invariant. |
| `src/prepared.rs:97` | `configuration catalogue set differs` | **E**, E-catalog | Pinned configuration/architecture manifest, catalogue set or ownership invariant. |
| `src/prepared.rs:212` | `configuration image and architecture dimensions/device disagree` | **D**, D-file/API | Wrong target image dimensions or invalid requested mux sink; not a valid 85F feature setting. |
| `src/prepared.rs:217` | `configuration catalogue and image disagree` | **E**, E-catalog | Pinned configuration/architecture manifest, catalogue set or ownership invariant. |
| `src/prepared.rs:225` | `configuration resource family differs: {}` | **E**, E-catalog | Pinned configuration/architecture manifest, catalogue set or ownership invariant. |
| `src/prepared.rs:232` | `configuration has no architecture owner: {}` | **E**, E-catalog | Pinned configuration/architecture manifest, catalogue set or ownership invariant. |
| `src/prepared.rs:386` | `unknown mux sink {sink}` | **D**, D-file/API | Wrong target image dimensions or invalid requested mux sink; not a valid 85F feature setting. |

### Err propagation and successful sentinel paths

These lexical `Err(...)` occurrences were inspected but are not additional refusals. `Err(self.failure())` forwards one of the errors in `src/fast.rs:1242–1269`; `Err(e)` propagates or matches an existing result. `fast_dsp.rs` deliberately uses Err to carry a nonconstant expression for code generation.

| Site | Expression | Meaning |
|---|---|---|
| `src/fast.rs:450` | `Err(self.failure())` | diagnostic propagation |
| `src/fast.rs:537` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:538` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:540` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:773` | `Err(self.failure())` | diagnostic propagation |
| `src/fast.rs:1055` | `Err(self.failure())` | diagnostic propagation |
| `src/fast.rs:1220` | `Err(self.failure())` | diagnostic propagation |
| `src/fast.rs:1364` | `Err(self.failure())` | diagnostic propagation |
| `src/fast.rs:1804` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:1807` | `Err(_)` | match/sentinel/control flow |
| `src/fast.rs:2275` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:2561` | `Err(e)` | diagnostic propagation |
| `src/fast.rs:2665` | `Err(e)` | diagnostic propagation |
| `src/kernel/fast.rs:1035` | `Err(_)` | match/sentinel/control flow |
| `src/kernel/fast.rs:1038` | `Err(_)` | match/sentinel/control flow |
| `src/kernel/fast.rs:1039` | `Err(e)` | diagnostic propagation |
| `src/kernel/fast.rs:1039` | `Err(e)` | diagnostic propagation |
| `src/kernel/fast_dsp.rs:523` | `Err((expr, read))` | match/sentinel/control flow |
| `src/kernel/fast_dsp.rs:573` | `Err(line)` | match/sentinel/control flow |
| `src/kernel/fast_dsp.rs:1100` | `Err((expr, _))` | match/sentinel/control flow |
| `src/kernel/fast_dsp.rs:1175` | `Err((format!("{} \| {}", sa.c(), sb.c()), vec![sa, sb]))` | match/sentinel/control flow |
| `src/kernel/mod.rs:1224` | `Err(error)` | diagnostic propagation |

Original-snapshot audit accounting (before the dated follow-ups): **357** lexical macro/Err/unreachable occurrences inspected; **335** classified diagnostic/invariant sites ({'D': 135, 'E': 110, 'C': 58, 'B': 17, 'A': 15}), plus **22** Err match/propagation/sentinel occurrences. This includes brace-delimited ensure! calls, so DCSC/DTR/PLL-control exclusions are not lost by searching only `ensure!(`. No remaining unclassified explicit site was found in that snapshot. The dated follow-ups update specific findings without claiming a new whole-source census. Internal invariants are skipped from feature ranking, not silently dropped from the accounting.

## Follow-up: R11 versus R07 on an ULX3S (bf47e64, including 1c79418)

**R11 is easiest to reach:** one used ordinary GPIO, explicitly `PULLMODE=NONE`, with no external level supplied. **R07 needs a readback inout bus that can float.** Neither is a normal stock-ULX3S button failure: the copied LPF pulls PWR up and the other buttons down, and the live board runner also supplies their idle levels. An omitted ball or omitted PULLMODE is not equivalent to explicitly removing the pull. Revised ranking: **R11 rank 9 (2×1=2), R07 rank 15 (0.5×2=1)**; these are qualitative likelihood estimates, not measured student incidence.

Scratch: `/tmp/ecp5-audit-inputs-6tt893bn` on the Mac and rserv. `python3 /tmp/ecp5-audit-inputs-6tt893bn/run.py` on rserv built current `dev`, a scratch Rust helper calling the actual `live::spawn`/Button/Step APIs, and the initial 13 cases below; the same rserv environment then built F14 and checked F12 with external drive. Each used Yosys 0.70 `synth_ecp5`, nextpnr-ecp5 0.11.1-77, CABGA381/85k, seed 1, and ecppack. CLI runs used `--startup configured --scope observed --names routed.json --clock net:clk --period-ns 40 --cycles 10 --probe net:q`, repeated with `--fast`; no input level was supplied except the bus control/data levels noted below. Full commands, RTL, LPFs, configurations, `.bit` files and logs remain outside the repository. F01 is an expected tool rejection; the other 13 cases packed successfully. GPIO/clock/output balls were all constrained except the three deliberately unlocated-input cases.

The scalar top was:

```verilog
module top(input clk, input d, output reg q=0);
  always @(posedge clk) q <= d;
endmodule
```

Common constraints: clk=G2/LVCMOS33/PULLMODE=NONE/25 MHz, q=B2/LVCMOS33/PULLMODE=NONE. `d` is A2 for a new GPIO, D6 for PWR, or R1 for FIRE1. The variants change only its LOCATE/PULLMODE. The inout top was:

```verilog
module top(input clk, input oe, input [1:0] d,
           inout [1:0] bus, output reg [1:0] q=0);
  assign bus = oe ? d : 2'bzz;
  always @(posedge clk) q <= bus;
endmodule
```

Here bus=A2/A4, q=B2/C2, oe=E8, d=D8/D7. CLI supplies `--set net:oe=0 --set net:d[0]=0 --set net:d[1]=0`; the live runner's default OFF switch positions supply the same controls. F12 has PULLMODE=NONE on both bus bits; F13 has UP. All these data/control pins are otherwise LVCMOS33.

| Case | Changed constraint | Event CLI | Fast CLI | Actual live board runner | Smallest honest remedy |
|---|---|---|---|---|---|
| F01 `input_unlocated_strict` | no LOCATE for d | No image: nextpnr rejects unconstrained IO | same tool failure | not run | Supply the real package ball in the LPF. Neither R11 nor R07 fires. |
| F02 `input_unlocated_allow` | no LOCATE, `--lpf-allow-unconstrained` | names load fails: R95C96/PIOA has no CABGA381 ball | same | same | Constrain the real ball. This seed chose an unbonded site; it is not evidence of R11. |
| F03 `input_unlocated_none_allow` | same, plus PULLMODE=NONE | same unbonded-site failure | same | same | Real ball first; then supply its actual pull/driver. |
| F04 `input_omitted_pull` | d=A2, PULLMODE omitted | q=0 | passes | passes, LED0=0 | No initial-Z fix needed: the pinned writer/database defaults decode as DOWN on this input. |
| F05 `input_none` | d=A2, explicit NONE | q=X | **R11**, starts at Z | **R11** | For a known external signal, supply `--set net:d=0/1` or its device model. Use an LPF pull only if electrically intended. Both `--set d=0` and explicit DOWN controls pass. |
| F06 `input_down` | d=A2, explicit DOWN | q=0 | passes | passes | A real LPF pull resolves Z without inventing a strong external driver. |
| F07 `pwr_none` | d=D6, explicit NONE | q=X | **R11** | passes: idle 1, pressed 0, released 1 | No live button fix needed; the existing board default supplies the external idle level. Bare CLI needs `--set net:d=1` or the stock UP constraint. |
| F08 `pwr_up` | d=D6, stock UP | q=1 | passes | passes; press/release correct | Stock PWR LPF already resolves the idle level. |
| F09 `fire_none` | d=R1, explicit NONE | q=X | **R11** | passes: idle 0, pressed 1, released 0 | No live button fix needed; bare CLI needs `--set net:d=0` or stock DOWN. |
| F10 `fire_omitted_pull` | d=R1, PULLMODE omitted | q=0 | passes | passes; press/release correct | This pin also decodes as default DOWN; omission is not NONE. |
| F11 `fire_down` | d=R1, stock DOWN | q=0 | passes | passes; press/release correct | Ordinary copied ULX3S button LPF; neither refusal. |
| F12 `bus_none` | released, read 2-bit bus, explicit NONE | q=XX | **R07** | **R07** | Add a real device driver or an electrically justified pull. A genuinely undriven read is X: retain event semantics rather than substitute a binary value. |
| F13 `bus_up` | same bus, UP | q=11 | passes | passes, both LEDs=1 | An LPF pull honestly fixes an open-drain/idle bus when a pull-up is part of the circuit. |
| F14 `bus_omitted_pull` | same bus, PULLMODE omitted | q=00 | passes | passes, both decoded bus pulls DOWN | No R07 here: the pinned default supplies a real weak DOWN; omitted PULLMODE is not NONE on this bus either. |

Config SHA-256 prefixes: F05 `7ce7df2feb324e47`, F07 `1d130e7029235987`, F09 `5dc6cde9906df29c`, F12 `6b73867cea67d7df`, F13 `4f292e25a16ad85f`. Full hashes and logs are in the scratch result records.

Exact sites at **bf47e64**: R11's ordinary initial-Z check is `src/kernel/fast.rs:3016` (message at :3018); :881 (message :882) is the separate tri-state-external X guard, which did not fire in these initial-Z cases. R07 is `src/kernel/fast.rs:2084` (condition :2083, message :2085). Strict unconstrained IO fails first in nextpnr `ecp5/main.cc:287`; this seed's unbonded mapping fails at `src/names.rs:136`. Pure input NONE failures name external:R35C0/PIOA (A2), R0C6/PIOB (PWR), or R95C4/PIOA (FIRE1). F12 names a floating PAD_A at R35C0 or R11C0; whichever bus bit is visited first is immaterial.

The temporary helper checked the decoded pulls with `board::input_pull`: F04/A2 and F10/R1 are `Some(false)` (DOWN); F07/D6 is `None`. It then called the real live runner, stepped ten cycles, pressed/released each button, and checked the LED after another ten cycles each time. `src/board.rs:97–133` defines PWR idle high and other buttons idle low; `src/live.rs:1226–1230` installs used button inputs before planning. This functionally supplies the external board idle level, even when no FPGA pull is configured; it is not a resistor/analog simulation. The stock rules are in `examples/ulx3s/debounce/top.lpf:11–17`. A new generic GPIO has no automatic board source. Plain CLI does not import the live button defaults, and `--peripherals` also left F05/F07/F09 at Z; that flag attaches peripheral models, not the live button map.

**Fix boundary:** For known physical buttons, a CLI board-default option could reuse the existing BUTTONS idle map; no new live default is required. For an externally driven new GPIO, the smallest fix is honest stimulus/device input, not silently forcing every unknown input to zero. For a physical pull, preserve it through the LPF (or a board pull model). For an actually floating bus, use the event kernel's X or reject the read in fast mode. F12 still hits R07 after `--set net:bus[0]=0 --set net:bus[1]=1`, and the event CLI confirms q[0]=0, q[1]=1 with those levels: the current guard rejects **float capability** at plan construction. A useful fast-mode change would check actual resolved unknown reads (including read enables) at runtime, permitting known driven periods while still stopping on a real X. Initial input Z is the simpler R11 trigger; persistent unknown bus reads are a distinct semantic limit.

DTR validation in this follow-up used the same rserv invocation: `cargo test --release --offline --test event_kernel dtr_ -- --nocapture` (**7 passed**) and `cargo test --release --offline --test dtr_native -- --include-ignored --nocapture` (**2 passed**). Native checks compare the exact HY04 configured primitive's fast/event bytes for profiles 0x9E and 0xA1; kernel checks include non-clock completion, coincident capture, retrigger, held high, rational deadlines, snapshots and handoff. They validate the current fixed profile policy; no new silicon run or temperature calibration was performed.
