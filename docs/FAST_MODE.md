# Fast mode

Added on 2026-10-29. Fast mode runs a design at the board's speed or faster: `uart_hello` runs
25 million cycles of the 25 MHz clock in 0.7–0.8 s. It is the first step of the fast executor
planned for Stage 3 in [ROADMAP.md](../ROADMAP.md).

The event kernel stays the reference. It keeps every routing segment, four-valued logic and delta
waves so that it can explain each transition, and it runs 18 000–91 000 cycles per second.
Fast mode keeps only what decides the values, compiles that to C, and is checked against the event
kernel cycle by cycle.

DCSC implements every DCSMODE and the MODESEL bypass with the event kernel's
HC02 policies ([CLOCK_NETWORK.md](CLOCK_NETWORK.md)). Its clock/control history,
pending source and two-edge hold live in simulation slots, so snapshots and tuner
swaps preserve a handoff. Inputs are captured before each wave publishes registers;
DCSOUT edges reach downstream captures in later waves. A stage cannot consume an
edge at its launch timestamp, even across a DTR host pause. Ambiguous controls,
unknown selected clocks and uncertain handshake edges are refused when they would
make DCSOUT X; irrelevant unknown constants are allowed. Fast inputs themselves
accept only known bits (or Z on supported external tristate pads).

## Use

```bash
ecp5_emu emulate top.config --fast --startup configured --scope observed \
  --clock pin:G2 --period-ns 40 --cycles 25000000 --probe pin:L4 --vcd uart.vcd
```

- `--fast` needs clang on `PATH` or `ECP5_CLANG` set to an executable path or a command name on `PATH` (never split on spaces; empty uses `clang`; use the wrapper below for `zig cc`). The C source and library are cached in
  `$TMPDIR/ecp5_emu_fast/` by the hash of the source, flags, compiler's `--version` output and `ECP5_CLANG` when non-empty. `FAST_CFLAGS` replaces the
  default `-O3 -march=native`. On x86_64, the default also disables AVX-512 with `-mno-avx512f`
  unless `FAST_AVX512=1`. A large program is compiled in pieces on every core (*Compiling*,
  below): Life in about 2.6 s, `life_full` in about 7 s, where one clang took 18 s and 449 s.
  For `zig cc`, set `ECP5_CLANG` to an executable wrapper beside `zig` containing:

  ```sh
  #!/bin/sh
  exec "$(dirname "$0")/zig" cc "$@"
  ```

- With at least two million cycles left after warm start, a first run starts with a size-based
  thread count and tunes in the background. Later runs recheck the remembered best with one
  throwaway simulation, searching further only below half its remembered speed. `FAST_CHOOSE=0`
  skips this start-up choice; `FAST_CHOOSE=full` measures everything again. The choice is printed.
  The run then measures other programs as it goes (*Tuning*, below); `FAST_TUNE=0` turns that off.
- `--period-ns` sets the clock period for both executors. The default is 2 ns, which keeps earlier
  runs unchanged; the board clock is 40 ns. It matters for OSC, which runs at its own frequency.
- Output:
  - stdout gets the probe values at 0 ns and after the last clock edge, plus the speed.
  - `--vcd` gets every probe after every clock edge and every stimulus time. It is the same file
    the event kernel writes; the checks below compare them byte for byte. A probe fast mode
    holds as unknown is written `x`, and a floating pad `z`. Before 2026-11-11 both were
    written `0`.
- `--stimulus` works as in the event kernel. Values must be 0 or 1. An event at the time of a clock
  edge is applied in the same instant as the edge. Events after the last clock edge are refused.
- `--output` is refused: fast mode keeps no event trace.
- **Console.** `--console-out REF` adds a UART terminal on that design output: 8N1 at `--baud`
  (default 115200), decoded by `ecp5_emu::uart::Receiver` from the line after every clock edge.
  - The bytes go to stdout as they arrive. The status lines and the probe values move to stderr.
  - `--console-in REF` is the design input the keys drive. It idles high, and each key is sent as
    one frame, back to back with the previous one (`uart::Transmitter`). The bits are input
    changes at their own times, applied the same way as `--stimulus`.
  - On a terminal, stdin is put in raw mode with `stty`: there is no local echo, Enter sends CR
    and Ctrl-C ends the run. The settings are restored on exit. A pipe is read as it is.
  - Without `--cycles`, a console run lasts until Ctrl-C.
  - The console is for fast mode only.

  ```bash
  ecp5_emu emulate top.config --names top_routed.json --fast --startup configured \
    --scope observed --clock net:clk_25mhz --period-ns 40 \
    --console-out net:ftdi_rxd --console-in net:ftdi_txd
  ```
- **Warm start.** While the kernel's state has an unknown value that the program would read, the
  event kernel runs whole cycles first, and fast mode takes over from its state once everything
  is known. Under the `configured` startup the registers, the EBR latches (HB02) and the DSP
  output registers (HD07) start known; under `ready-unknown` they do not. The output says how
  many cycles that took, and the VCD covers both parts.

In Rust it is `ecp5_emu::fast::FastSim`:
- `new(&kernel)` builds from the kernel's state and time, and fails with `StartsUnknown` while
  something it would read is unknown; then `clock(driver, period)`;
- `run_cycles` / `run_cycles_sampled` / `run_edges_sampled` / `run_until`;
- `set` + `step`, or `set_at(time, inputs)` to apply inputs together with an edge;
- `value(signal)`, `memory(b)` for the array of an EBR, and `lutram(w)` for the sixteen cells of
  a LUTRAM column. A probed signal is always up to date. Another net may be left from an earlier
  cycle until `step`: one that only the data input of a register with an enable reads (gates on
  demand, below), or one folded into the table of the gate that reads it (folding, below; with
  `FAST_FOLD=cones`, of the gates that read it).
- `read(&mut reader, &mut out)` reads a long list of signals (`Reader::new(&signals)`) as
  `value` would, but looks their slots up only once, and again only when tuning has changed
  program: a sweep over the list instead of a lookup in the signal table apiece.
- `tune(&kernel)` measures programs with other options as the design runs (*Tuning*, below).

## What it covers

- LUT4, CCU2 carry halves, muxes, buffers, PIO inputs and outputs.
- Flip-flops with clock enable, synchronous and asynchronous resets, and clocks made by logic or
  flip-flops.
- IOLOGIC: ODDRX1F, IDDRX1F and the IO registers, with the timing measured on the board (HI02,
  [IOLOGIC_EXECUTION.md](IOLOGIC_EXECUTION.md)). The IO registers also take CE, a synchronous
  set/reset and GSR=ENABLED, the global reset acting asynchronously like their ASYNC LSR (HI05).
  Input delays (DELAYG, DELAYF) pass the pad with zero delay, as in the kernel (HI08).
  DELAYF's counter is an IOLOGIC arm clocked by MOVE: a DIRECTION stage at the rise, seven
  counter bits at the fall. LOADN reaches it as an asynchronous reset with a value per bit
  (DEL_VALUE). CFLAG is ordinary logic over the counter bits.
  An IO latch (HI10) has no clock-edge capture. It is one line in `asynchronous()`, which
  follows D while CLK is low and CE high, and its LSR wins there.
- DCCA clock gates with a dynamic CE, as the board showed (HC01, [CLOCK_NETWORK.md](CLOCK_NETWORK.md)):
  CE taken at each rising edge of CLKI passes or stops that whole high phase.
- The X2 and 7:1 gearboxes (ODDRX2F, IDDRX2F, ODDR71B, IDDR71B) and the edge-clock blocks, as
  the event kernel runs them
  (HI03, HI04; [IOLOGIC_EXECUTION.md](IOLOGIC_EXECUTION.md),
  [CLOCK_NETWORK.md](CLOCK_NETWORK.md)):
  - ECLKSYNCB with its STOP synchronizer, also after the bridge (BRGECLKSYNC);
  - ECLKBRIDGECS, a mux by SEL (HI06);
  - CLKDIVF /2 and /3.5 with RST and ALIGNWD.
- LUTRAM as the event kernel runs it ([LUTRAM_EXECUTION.md](LUTRAM_EXECUTION.md)): the reader is
  asynchronous; the write is stored at the clock edge from the state before it, with the enable and
  the startup write inhibit.
- EBR as the event kernel runs it ([EBR_EXECUTION.md](EBR_EXECUTION.md)).
  - DP16KD with every width pair, and PDPW16KD with every read width. A 36-bit read (every
    `reg [31:0] mem[0:511]` yosys infers) fills both read paths, DOA and DOB, on CLKB.
  - Write modes, output registers, CE, OCE, CS decoders, byte masks, synchronous and
    asynchronous resets, GSR and the write inhibit.
  - Inside an EBR, X is tracked as the kernel does: a cross-port collision, an unmeasured
    control combination, unknown INIT or a latch not read yet. The program stops, naming the
    EBR, only when the logic would read such an X. A probe shows it as X.
- PLL outputs ([PLL_EXECUTION.md](PLL_EXECUTION.md)). As in the event kernel, the PLL learns the
  reference period from its first two rising edges and starts its outputs from the second. A
  period the kernel has already learnt carries over.
- OSC outputs ([OSC_EXECUTION.md](OSC_EXECUTION.md)): low at 0, at the frequency the plan gives
  (`--osc-hz`), in phase with the kernel when fast mode takes over.
- An unknown carry-in that the propagate term can never pass, as for the `FCO` of a LOGIC-mode
  slice (S04).
- Tristate and bidirectional pads (`inout`, `assign pad = oe ? d : 1'bz`, open drain), also with
  one external driver (`set_at_levels`, `set_levels`, `None` for Z):
  - a pad is the design's value while its output is enabled, else the external driver's level if
    not Z, else its pull (PULLMODE UP or DOWN); if the design and the external driver drive
    different levels, fast mode stops with a runtime error naming the pad (the event kernel makes
    it X);
  - fast mode stays two-valued. The one exception is a pad with no pull while neither the design
    nor the external driver drives it: it holds Z, which a probe sees as Z (in the VCD too), as in
    the kernel. Logic, a capture or a memory port reading such a pad is refused, since the kernel
    gives it X there.
- A GSR primitive driven by logic. Without wake phases (refused) the system controller's GSR is
  the primitive's input, polarity applied, and the registers, EBR read stages and MULT18 stages
  that take GSR read that net.
- MULT18X18D as the event kernel runs it ([DSP_EXECUTION.md](DSP_EXECUTION.md),
  [DSP_REGISTERS.md](DSP_REGISTERS.md), [kernel/fast_dsp.rs](../src/kernel/fast_dsp.rs)): without
  registers, as yosys writes every `*`, and with any of the INPUTA, INPUTB, PIPELINE and OUTPUT
  stages:
  - each product bit is a line `v[p] = mul18(A, B, SA, SB) >> k & 1`, `mul18` being a
    `static inline` in the part of the program every unit has;
  - the 36 lines of one multiplier read the same slots and stand side by side, but clang
    merges few of them. Each line gathers its 36 operand bits and multiplies again: in `mac`
    the 152 such lines of `converge` compile to 128 multiplications. `mac` (seven
    multipliers) still runs at 25.6–26 MHz of board clock. One line writing all 36 bits needs
    lines with several outputs (layout, `FAST_ACTIVITY`): see *Next*;
  - SIGNEDP is SIGNEDA | SIGNEDB, and ROA/ROB are the A/B inputs.
  - **Register stages.** Each stage bit is a capture, like an IOLOGIC stage: `enable ? data :
    itself` at a rising edge of its clock, a synchronous reset first. Its slots follow the
    LUTRAM cells, so every program of one kernel numbers them the same.
    - The operands are the INPUTA/B registers when those stages are on.
    - P is the OUTPUT register, else the PIPELINE register.
    - SIGNEDP passes through PIPELINE and is X after OUTPUT, as in the kernel.
    - An asynchronous local reset and GSR clear a stage at any time, the OUTPUT stage included
      (HD07).
    - The stages start once the kernel has made them known (warm start).
  - A program without multipliers is the same text as before.
- **ALU54B** ([DSP_ALU.md](DSP_ALU.md), `kernel/fast_dsp.rs`), as the kernel runs it:
  - `alu54` in the prelude gives the 54-bit result from OP and the sources, each gathered from
    its slots.
  - A registered bit of R is a capture of its bit of the result, enabled by the ALU's CE0
    pin. A bit before the register is a line, and a bit without a clock is the constant 0.
  - The opcode and C registers (HD08) are captures too, and the result reads their slots
    where they are on. A C register's capture is gated by its divider, a slot that toggles at
    each edge of its clock and that GSR holds at 0.
  - The reset clears the registered bits and wins over the enable: R[17:0] and R[44:27] take
    the line REG_OUTPUT0/1_RST selects, R[26:18] and R[53:45] RST0. It clears at the clock
    edge where it is synchronous and at any time where asynchronous (by RESETMODE of the ALU
    and of its two MULT18, HD06, HD07). An enabled GSR clears every registered bit at any
    time (HD07).
  - A source that the known bits of OP cannot select is passed as 0. A constant OP therefore
    reads only what it uses.
  - A product that comes straight from a multiplier with no register after it is computed
    inside the expression from the multiplier's operands. The 36 product nets are then read by
    nothing, and the per-cycle settle no longer multiplies: HD02's configuration went from 2.1
    to 4.1 MHz of board clock (40 M cycles, compile included).
  - The flags are constants, or captures of constants when they have a register.

It refuses with the reason, and the event kernel remains the way to run these:
- a MULT18 OUTPUT stage whose CE0..3 or RST0..3 are not tied to 1 and 0 (the kernel checks
  driven ones as the design runs), SOURCEA/B other than 0, and an ALU54B whose OP can read R
  through unregistered bits (a loop on the chip, [DSP_ALU.md](DSP_ALU.md));
- a tristate pad that can float (no pull) while the logic reads it, a non-tristate pad driven from
  outside, a tristate pad with more than one external driver, or a board wire on a pad the design
  also drives;
- an X or Z that the program would read: an input that starts at X or Z (give it with `--set`),
  a register that does not become known (the `ready-unknown` start), an X constant;
- a net with two active drivers, and combinational loops;
- a design that drives the system controller (`wake-phases`), and logic that reads the GSR of a
  GSR primitive driven by logic (the registers, EBR and MULT18 stages that it resets run);
- a PLL whose reference is not a clock input.

## How it works

1. **Netlist** ([kernel/fast.rs](../src/kernel/fast.rs)). The kernel's plan, after its routing
   buffer chains are collapsed, becomes nets and gates.
   - Each net has one byte slot. Gates are in dependency order, and flip-flops and IOLOGIC are
     grouped by clock net.
   - Folded away:
     - constant nets: system controls, tied inputs, PIO tristate and output-enable muxes;
     - buffers, receivers, and LUTs and carries that depend on at most one net. The output becomes
       an alias of that net, possibly inverted. A counter's propagate LUT is its own Q, so its carry
       becomes `q & cin`.
   - Reads that cannot matter are left out of the order:
     - a LUT or carry input that its table ignores (nextpnr may route a signal there);
     - the carry-in of a carry whose propagate term the constant inputs hold at 0. nextpnr starts
       each chain with such a CCU2 (a GND input), and its FCI is wired to the FCO of the chain
       placed just before it in the row. Where the two chains feed each other through logic, the
       plan has a loop that only exists in the wiring; the kernel settles it, and fast mode must
       not refuse it. Constants are folded through the gates first, so a GND or VCC made by a LUT
       counts as a constant.
2. **Program.** The netlist is written as two-valued C. The one exception is Z (byte 3) on a
   tristate pad with no pull while nothing drives it, which only probes read (*What it covers*).
   - An instant sets the inputs that change at it. Then the gates downstream of those inputs are
     evaluated, and the program converges, one wave at a time as the event kernel does:
     - the edges of each clock group are found;
     - the members capture, all from the state before the wave, and a member whose own
       asynchronous reset is on takes its reset value;
     - the asynchronous resets apply to the registers without an edge;
     - the logic downstream is evaluated.

     This repeats until no edge and no reset is left, so clocks made by flip-flops work as in the
     event kernel. Before 2026-11-11 the resets came first, so a register clocked in the same
     instant as another register's reset read the reset value, where the kernel reads the old one.
   - When one group has an edge its members use (the usual case), a branch for that group and edge
     writes only those members. Writes go in place, ordered so that each register is written before
     any register that reads it. Only registers that read each other in a loop go through
     temporaries.
   - Edges of several groups in one instant take a general path. There every register selects its
     value by its edge, and all of them read the state from before the instant.
   - **Gates on demand.** A gate that only the data input of a register with an enable reads
     (directly or through other such gates) leaves the settles. The address and data of a LUTRAM
     write count the same way, with the write's enable.
     - The capture evaluates such gates first, before anything is written, and only when one of
       the enables that read them is 1. Gates that share those enables run under one test, and a
       run of fewer than four gates has none.
     - The values are therefore fresh exactly when a capture reads them, whatever changed in
       between.
     - What else reads a net keeps it in the settles: an enable, a reset, a clock, IOLOGIC, EBR,
       a register without an enable, a probe.
     - A gate that nothing reads, not even a probe, runs only in the full settles.
   - **Folding.** In the settle after a capture, a gate that only one other gate of that settle
     reads, and nothing else (no register, LUTRAM write, EBR, clock or probe), is folded into the
     gate that reads it. That gate becomes a table of its inputs, the folded gate's inputs among
     them, of at most twelve (`FAST_COLLAPSE=8` eight; `0` folds nothing). The widest
     folds are given up first when there are more. By default gates several gates read are
     folded too, within one sink's logic (`FAST_FOLD=cones-widen`, below). Only a gate that folded another becomes a
     table: one that folded nothing stays as written, often cheaper (a counter bit's
     `v[a] ^ v[cin]` on a carry chain). A folded net is not computed by this settle, only by the
     full ones (`step`), so it is stale in between, which nothing reads it in. A net
     whose reader another settle also computes (another clock group's, an input's, the one after
     a reset) while not computing the net itself is never folded: that settle would read it
     stale. A DELAYF's CFLAG, on the design's clock, read mux chains over the counter that the
     MOVE clock's settle had folded into one table, and kept its old value.
     - A table of up to six inputs is a constant in its line, `(M >> index) & 1`; one of seven or
       eight is a byte an entry in one array of the program (`wide`), read with one load where a
       bit table took six operations; nine to twelve, a bit an entry (`wide_bits`). One table per
       function.
     - A LUT that stays a line takes its constant and inverted inputs into its INIT word, and
       reads its nets in slot order, as a table does. Lines that read the same nets then build the
       same index, and in each thread's part (and the shared lines) such a line moves to just after
       the first one: on `life_full` a fifth of the lines have such a twin, and they load their
       inputs and build the index once.
     - `FAST_FOLD=cones` also folds a gate several gates read, into each of them, when that does
       not widen their table, and keeps it only while something still reads it;
       `FAST_FOLD=cones-widen` lets it widen them too, a few rounds that stop it computing a gate
       twice, and only for a gate whose readers all lead to one sink (a register or something
       outside the settle): a Life cell's adder becomes part of the cell's table, a counter's
       carry chain, read on the way to every higher bit, is left alone. This is the default since
       2026-11-16; `FAST_FOLD=single` folds one-reader gates only, as before. On `life_full` at 12
       inputs `cones` has 36 300 lines instead of 42 600 and runs as fast; `cones-widen` has
       13 600, and a 10-thread part runs alone in 0.72–0.95 µs instead of 1.0–1.6 (the second
       phase 0.43 µs instead of 0.23). Without the one-sink rule it turned the population counter
       into a second phase of 2 758 lines on the caller. Three rounds against `single` at 8
       inputs, same machine (load 4–18): `life_full` 10 threads 117–166 kHz against 46–64,
       8 threads 75–108 against 52–81, Life 755–772 against 558–829, `picorv32_console`
       2.38–2.39 MHz against 2.29–2.31.
   - **Threads.** A settle of 2000 lines or more (`FAST_THREAD_MIN`) is split across three threads
     (`FAST_THREADS`; `1` keeps one).
     - Each line that no other line of the settle reads goes, with its cone, to the thread whose
       lines overlap its cone most, while that thread has room; the largest first. A line that
       only one thread's lines read is that thread's; one that several read is shared and runs
       first, on the caller, so that no thread waits for another.
     - One large cone (a counter's adder tree, which all its bits read) would leave one thread
       with most of the work, so the settle may be cut at a depth into two phases, the second
       starting when every thread has done the first. A model of the time (the shared lines, the
       largest part, and a meeting of the threads counted as 300 lines, `FAST_BARRIER_LINES`)
       picks the cut, or none.
     - Lines are weighed by their inputs plus two, in the room a thread has and in that model:
       a line costs about two operations an input, in time and in machine code.
     - `FAST_CODE_BUDGET=<KB>` limits the machine code the caller runs a cycle (its captures, the
       shared lines and its parts): its part of the first phase shrinks to fit and the others
       take the rest. Unset, no limit (see `life_full` below for why).
     - Each thread's slots move to a region of their own, a cache line apart from the others':
       threads writing bytes of one line take it from each other at every write. Without this,
       four threads ran Life at 70 kHz instead of about 400.
     - Owner capture is the default for a threaded group on the single-group path, except with
       `FAST_ACTIVITY`: captures run on the thread that computes what they read, and flip-flops
       move to their owner's region. `FAST_CAPTURE=caller` restores the caller's capture and the
       layout by the thread whose lines read each flip-flop most (the caller's shared lines count
       as the first thread's), so that a thread finds the registers it reads on
       a few cache lines rather than one line apiece. In a region the slots only that thread reads
       come first; the nets something else reads (the captures, probes, gates on demand) follow
       on lines of their own, since the caller reads those every cycle; then the registers, on
       lines of their own again. Registers once shared lines with the nets their thread writes:
       on `life_full` every part then read 7–9 lines another part was writing, and ten parts on
       one array, with no capture at all, took 2.5–2.8 µs each against 1.5 on arrays of their own.
       What the shared lines write comes together in a region of its own: left where they were,
       the 168 slots a `life_full` part read of them sat on 111 cache lines, now on 7–12.
       `FAST_SPLIT=place` separately enables a split by place on the chip; it stays off by default.
     - The workers spin while the design runs and sleep when it stops. One simulation of a design
       uses its pool at a time; a second one of the same design runs the parts in turn.
       `FAST_THREAD_REPORT` prints the split and the program's name.
     - A worker claims its part as it starts it. A worker done with its own part (or that found
       it taken) runs every part nobody has claimed yet, and so does the caller once it has run
       its own part and then waited about 2.5 µs (2000 spins, `POOL_STEAL_SPINS` in the C). On
       a loaded machine a worker the system has not scheduled would otherwise hold the settle
       up until it ran again, a millisecond or more; with only the caller taking them, it ran
       the parts of every absent worker in turn (`life_full`, 10 threads, 8 busy loops: 24 kHz;
       with the workers taking them too, 34, and no change on a quiet machine). The parts of a
       phase are independent, so whoever runs one gets the same values. A part already claimed
       is waited for: a worker preempted in the middle of it still holds the settle up, and
       running it again elsewhere is not safe (the late worker would then write last cycle's
       values into the next one), so fewer threads (*Tuning*) is the cure for that.
       `FAST_THREAD_STEAL=0` never takes a part.
     - Each claim, each part's finishing generation, and what the workers read on every settle
       (the generation, the settle's functions and slots) take cache lines of their own. Whoever
       runs a part writes its line when done, and the caller reads them all: one counter every
       worker added to took the line from core to core once a worker (ten threads with empty
       parts: 0.71 µs a meeting with the counter, 0.33–0.40 with a line a part). The caller
       writes the settle's pointers only when they change, and wakes sleeping workers after its
       own part, not before it.
     - A phase with lines for the caller only (`life_full`'s population counter after the cut)
       runs on the caller directly, without a meeting.
     - `FAST_THREAD_STATS` makes the pool print, when it stops: its waits of 1024 ticks
       (42.7 µs on Apple silicon, 1024 ns elsewhere) or more, the parts the caller and the workers
       took, when the workers claimed theirs, and how long parts took on each kind of core
       (efficiency cores, then each performance cluster).
   - **Tuning.** The thread count, the fewest lines split, the folding width and the meeting cost
     above are defaults measured on one machine. `FastSim::tune` measures programs with other
     options on the design and the machine it runs on, while the design runs, and runs the
     fastest. `emulate --fast` tunes runs of two million cycles or more; `FAST_TUNE=0` turns it
     off, and an option set in the environment stays as it is.
     - With no startup memory, `FastSim::start` guesses from non-routing gates before folding:
       below `FAST_THREAD_MIN` it uses one thread, from 50 000 gates up to ten performance cores,
       otherwise the defaults. It generates that program once; `choose_options` returns it
       without trials or another generation. Pinned options stay pinned. The tuner groups short
       run pieces into at least 50 ms speed samples, so the window's 30 ms publish target still
       gets meaningful measurements. It remembers the start after its first candidate round
       and accepted replacements, with speeds from run pieces. On a clean stop after at least
       two seconds of running it saves a measured home if nothing was saved yet, never a probe
       or an unconfirmed replacement.
       Next time one 250 ms trial rechecks the remembered best. Below half its remembered speed,
       the best, its neighbours, the second best and defaults run a round; still below half,
       everything is searched again.
       The choice key binds the design, plan, generator and options; watches, probe labels,
       live values and clock history do not change it.
       `FAST_CHOOSE=0` skips this start-up choice; `FAST_CHOOSE=full` measures everything again.
     - The candidates are the running options with one changed: 1, 2, 3, 4, 6, 8, 10, 12 or 16
       threads, up to the performance cores (`hw.perflevel0.physicalcpu` on Apple silicon: an
       efficiency core runs a part at half speed and the settle waits for it), none above one
       when no settle has 300 lines; folding to 0, 6, 8 or 12 inputs, the fewest lines split
       lowered to a smaller settle's, the meeting cost raised (when a settle is cut in two
       phases) or lowered. Programs that come out the same are one.
     - They are built in order of likelihood, around the last run's choice when there is one:
       half its threads (where a loaded machine sends a large design), a thread more or fewer,
       every performance core, folding a size smaller, one thread, the rest; for at most 1.5 s,
       on the background worker while the design is already running. Life builds all of them
       in about a second; `life_full` (108 000 lines) builds about one a second, so a run builds
       one or two. Candidates not measured on an earlier run come before those that were, so later runs get to the rest.
     - Tuning copies only the generator's inputs once. Its signal snapshot pools names and
       driver lists and excludes event fanout lists. The event kernel keeps plain vectors:
       running it or watching a net never copies a shared plan. There is no snapshot when
       tuning is disabled or has no candidate options. Its large buffers use fallible
       reservation; a failed reservation generates candidates synchronously from the live
       kernel instead (reported with `FAST_THREAD_REPORT`). The worker drops the snapshot
       as soon as generation finishes, before compiling libraries.
     - Candidate generation and compilation run on the same background worker, at utility QoS
       on macOS and a lower nice priority on Linux. Candidate generation does not delay the
       first cycle. Libraries not in the cache compile one at a time, and each is measured
       once it is there. A compile outlives a short run that started it.
     - Every program of one kernel numbers the slots the same up to the threads' regions
       (`FastProgram::place`), so the state moves between programs exactly. A full settle then
       brings up to date the nets the new program folds or evaluates on demand differently; it
       captures nothing, the state being one between instants. Snapshots keep their layout and
       are moved the same way when restored under another program.
     - A run goes in pieces of about 100 ms, its first 50 ms unmeasured. A measurement runs the
       candidate (4 ms to warm up, then 15 ms measured), then the running program again (the
       same), and keeps their ratio. A candidate at least 85% as fast is measured twice more
       within a second. When the median of its last three to five ratios, and two of the last
       three, are above 1.10 (or twice the spread below, if more), it takes over on trial: once it
       has run ten pieces of 50 ms or more, it stays if their median speed is not 2% below that of
       the program before it over its last ten, else that one comes back, and the candidate is
       not measured for 2 minutes, twice as long after each trial it loses.
     - The trial is what decides. On a busy machine one ratio can be off by a factor of two, and
       measured in turns right after a change of program, the program with fewer threads looks
       faster than it runs: 2 threads measured 159% of 3 and then ran at 71% of them; folding to
       6 inputs measured 179% of 8 and ran at 58%.
     - While the spread of the running program's last ten pieces (median absolute deviation over
       the median) is above 10%, close programs cannot be told apart, so only programs with fewer
       threads are measured, once a second, or every quarter of a second when the running one
       has fallen to half its best since it took over; a candidate must then beat it by twice the
       spread, at most three times. A loaded machine that leaves fewer cores than threads does
       not slow a settle split over many threads by a few percent but by ten times or more
       (`life_full`, 10 threads: 95 kHz at a load of 10, 4 kHz at 13–16 while 6 threads ran 57),
       and a noisy measurement still shows that. Nothing is measured beyond 0.25 s plus 2% of
       the run spent on other programs. After the first
       round each candidate is measured again every 2 s, one below 60% every 30 s, twice as
       seldom after each measurement that changed nothing, down to every 32 and 480 s; at most
       every 10 s, when the running program's speed moves by a fifth (the load changed), as
       often as at first.
     - When a run ends, its choice and the candidates below 60% or that lost a trial are kept
       next to the library (`<hash>.tune`). The next run of the design starts with that choice
       and measures those candidates later, not at the start.
     - `emulate --fast` ends with one tuning summary: the accepted options and median speed
       on actual run pieces, whether it kept the start or switched from it, or that the run
       was too short to tune. A pending probe or unconfirmed takeover is not a final choice.
       When tuning ran, the line points to `FAST_THREAD_REPORT=1` for more detail. That switch
       preserves the setup timing and full closing report (measurements, switches and reversions,
       speed changes and spread, time on other programs, all candidate ratios and build/failure
       states), as well as each change of program during the run. Runs with `FAST_TUNE=0` or attached
       peripherals keep tuning silent.
     - The checks (`tests/fast_mode.rs`) change program every 97 cycles or edges, among every
       candidate and settles split across 2 and 4 threads however small, on Life,
       `picorv32_console`, `uart_ram`, `blinky_pll`, `ddr_pattern` and `uart_hello`: every sample,
       the EBR arrays and the LUTRAM cells agree with a run that keeps one program, also after
       restoring a snapshot taken under another.
   - **Compiling.** A settle is one basic block of straight lines, and clang's instruction
     selection and scheduling grow faster than the block: one part of `life_full`, 14 800 lines,
     took 93 s at `-O3`, 68.6 s of it selecting instructions, and -O1 (90 s) or GlobalISel
     (104 s) did not help. So the program is cut and spread:
     - Every run of more than 500 lines that each write a slot and read only slots, tables and
       constants (`FAST_FUNCTION_LINES`; 0 cuts none) becomes functions of 500 lines, called in
       turn where the lines were: in the settles, the threads' parts, the cases of an input's
       settle and the captures of one clock group. (A capture on several groups at once reads
       the edge flags of `converge`, `rise0`, and stays where it is.) The part above then compiled in 13 s. Cut at 250, 500 or 1000 lines, Life
       ran as fast as uncut (medians 423, 425, 418 kHz against 414, three libraries each).
     - The full settles (all gates, and those downstream of the registers) run only at a start,
       a step, an asynchronous reset or a change of program; their pieces are compiled without
       optimisation (`COLD`, which is `optnone`; `FAST_CFLAGS=... -DCOLD=` optimises them too):
       10 000 of their lines compile in 1.1 s instead of 6.6. A full settle of `life_full` then
       takes 1.3 ms instead of about 0.5, of Life 50 µs instead of 21. After an asynchronous
       reset the full settle now also does the work of the settle downstream of the registers
       when that has half its lines or more (the other gates come out the same), so the program
       has one copy of them, not two.
     - Pieces of more than 3000 lines in all (a cold line counting as an eighth) go to units of
       about 3000 lines, `#if TU == k` in the source, and the rest is unit 0. The units compile
       at once, one per core (two in the background, for tuning), and are linked; the tables
       are in every unit, which keeps the ones it reads.
     - `life_full`: 22 units, compiled in about 7 s against 449 s; its LED trace over 20 000
       cycles is byte for byte that of the uncut program, and it runs as fast.
   - IOLOGIC captures read the same state and are stored after the flip-flops.
   - **LUTRAM.** A column's sixteen cells are slots, and its reader indexes them with the read
     address. The write stores into the cells before any flip-flop is written in place. It reads
     only nets, and the cells are read only by gates, so every capture still sees the cell from
     before the edge. The cells belong to the write clock's group, so the group's settle
     evaluates the readers again.
   - **EBR.** Each EBR is a function called before the flip-flops are written, with the edge
     of each port.
     - It follows `capture_ebr`: writes from the array before the edge, then reads, then the
       conservative collision rule for two ports on one INIT9 row.
     - The array lives in the slot array, a byte per cell of the 2048×9 view; 2 stands for X.
     - New latches and output registers are stored after the flip-flops.
     - A port whose outputs nothing reads and no probe watches is not evaluated.
3. **Host** ([fast.rs](../src/fast.rs)). The host keeps time in whole ticks, a unit that divides
   every period it knows, starting from the kernel's time.
   - The clock, each OSC output and each PLL output are periodic edge trains.
   - The host merges them into instants and calls the program at each one. A PLL output train starts
     at the PLL's second reference edge.
   - Once every train is periodic, the host builds a table of one hyperperiod. The program then runs
     that table without returning to the host, for runs of 4096 clock edges or more and up to 2²⁰
     instants per hyperperiod.
   - With only the clock, a C loop toggles it directly.

## Speed

Apple M3 Max: one second of board time (25 million cycles of the 25 MHz clock), `tests/fast_mode.rs`
and `emulate --fast`. Measured on 2026-10-30, three interleaved runs of the version before gates on
demand and the current one in the same measurement series.

| Design | Program | Before gates on demand | Cycles of 25 MHz per second | Real time |
|---|---|---:|---:|---:|
| `uart_hello` | 111 gates, 47 FF | 30–33 M | 41–46 M | 1.6–1.85× |
| `ddr_pattern` | 6 gates, 3 FF, 4 IOLOGIC | 104 M | 109–116 M | 4.4–4.6× |
| `blinky_pll` | 64 gates, 32 FF, PLL at 100 MHz | 15.4 M | 15.0–15.6 M (60 M PLL cycles) | 0.6× |
| HC06 (`hc06_16`) | 841 gates, 270 FF, OSC at 19.4 MHz | 3.0–3.1 M | 6.7–6.8 M | 0.27× |
| `uart_ram` | 199 gates, 113 FF, 2 EBR | 13.4–13.6 M | 15.5–16.5 M | 0.65× |
| `picorv32_console` | 2396 gates (1326 on demand), 628 FF, 128 LUTRAM columns, 4 EBR | 1.25–1.30 M | 1.90–1.94 M | 0.08× |

On earlier days `ddr_pattern` measured 119–129 M and `uart_hello` 31–37 M with the older version,
so compare within one row.

How `uart_hello` got there:

| Step | MHz |
|---|---:|
| Event kernel, for comparison (`kernel_speed` on HI02 and HC05) | 0.09 |
| First program: full settle three times per cycle | 5.5 |
| Gates downstream of the changed input or the flip-flops only | 11.0 |
| A clock group wakes only on the edges it uses | 22.6 |
| Flip-flops written in place | 25.3 |
| Buffers and one-net LUTs folded into aliases | 32.9 |
| Gates on demand (same measurement series: 30–33 before) | 41–46 |

On the other designs:
- **`blinky_pll`.** The host loop cost 45% of the time (profile), so the hyperperiod table in the
  program took it from 7.4 to 12.6 MHz. Folding gave 14.
- **`ddr_pattern`.** Branches per group and edge took it from 44 to 129 MHz.
- **HC06.** Evaluating only the logic downstream of the group that had the edge took it from 2.1 to
  3.4 MHz.

The cost per instant is the logic downstream of the clock group that captured, about 0.3 ns per
LUT. HC06 evaluated 791 gates on every 25 MHz edge; with gates on demand it runs twice as fast.

### Where a soft CPU spends its time

Before gates on demand, `picorv32_console` (the stock PicoRV32 running its firmware: the sieve, the
UART output, the console) evaluated all 2404 gates and wrote all 637 flip-flops on every rising
edge. Counters in
the generated C, over 346 000 cycles of `tests/fast_mode.rs`, show how little of that changes:

| Measured per cycle | Share |
|---|---:|
| Gates whose output changes | 6.0% |
| Flip-flops whose value changes | 4.8% |
| Gates with at least one changed input: the work an ideal skip would leave | 16.5% |
| The same with the gates taken in blocks of 4 consecutive gates of the evaluation order | 33.8% |
| Blocks of 16 | 68.4% |
| Blocks of 64 | 91.8% |

Three ways to skip work were then measured on this design. The prototypes rewrote the generated C
and were checked by the same test against the event kernel.

1. **Blocks with a changed input** (the approach of ESSENT). Each block of K gates has a flag. A
   gate on a block boundary, or a register, sets the flags of its readers when its value changes;
   a block runs only with its flag set. Every variant was slower than evaluating everything
   (1.2 MHz):

   | K | Gates evaluated | MHz |
   |---:|---:|---:|
   | 1 | 16.6% | 0.26 |
   | 4 | 34.0% | 0.66 |
   | 8 | 49.7% | 0.67 |
   | 16 | 68.3% | 0.79 |

   A gate order that keeps each register's cone together (depth first) gave the same shares as
   Kahn's: the activity is spread out. A test per block whose outcome is nearly random costs a
   branch misprediction, and that costs more than the gates it saves.
2. **Cones of changed registers.** A gate is evaluated only when a register it depends on changed.
   Offline, over the recorded changes: 40.5% of the gates, but in 1765 distinct sets of source
   registers, so about as many tests. The logic is shallow: a gate depends on 17 registers in the
   median.
3. **Gates on demand** (above). Half of the gates (1309 of 2404) feed only the data inputs of
   registers with an enable, or LUTRAM writes. There are 32 enables, so a few dozen tests decide
   it. 59.5% of the gates are evaluated per cycle, and the program runs at 1.9 MHz. This is in fast
   mode now. The rest is mostly the logic of registers that capture on every cycle, such as
   PicoRV32's `alu_out_q <= alu_out`.

The remaining work is about 1400 gates and 630 register writes per cycle, about 0.5 µs. Running the
board's 25 MHz would take 40 ns. Bit-level evaluation cannot close that gap by skipping. The
bit-sliced datapath (32 LUTs with one function on the 32 bits of a word) would have to run as word
operations.

How much of PicoRV32 is bit-sliced was then measured on the program's own gates: a census of the
generated C, with the HDL names from nextpnr's routed netlist. A group of gates becomes one word
operation when every gate in it has the same function and each input is one of these:
- the same net for every gate;
- for gate i, lane i + k of one word.

The words are built in two steps.
1. They start at the registers with vector names (`cpu.reg_pc[0..31]`) and at the EBR outputs.
2. Lanes are then carried along four kinds of path:
   - forward through such groups;
   - back from a register to its D net;
   - back from a gate to its only varying input;
   - along carry chains. Constant folding turns those into chains of ANDs or multiplexers.

Names alone are not enough. `reg_op1`, the ALU's first operand, has no bus name left in the
netlist, and the index in `reg_op1_TRELLIS_FF_Q_<k>_DI` is not the bit.

| Measured on the 2404 gates and 637 flip-flops | Share |
|---|---:|
| Gates in groups of 4 or more under one consistent layout (72 word operations) | 39.6% |
| More gates with the same function and input words, but offsets that layout does not align | +22.6% |
| More gates: LUTRAM readers that layout leaves out (each port reads 32 lanes) | +5.3% |
| Flip-flops in word registers (same clock, enable and reset, aligned D) | 78.2% |

The rest, about a third of the gates, is logic outside the datapath: the decoder, the state
machine, the UART, and comparisons that reduce a word to a bit. The speed estimate assumes these
costs, in gate evaluations:
- a word operation costs two;
- a lane moved between the bit form and the word form costs a third of one.

Under these assumptions a cycle costs about 1890 units under the layout above, and about 1250 at the
bound, against about 3040 now. That is 1.6 to 2.4 times faster, roughly 3 to 4.5 MHz. Word operations
alone do not reach the board's 25 MHz on this design.

For reference, the same SoC was run in Verilator 5.054. The setup:
- 25 million cycles;
- a C++ bench that decodes the UART line. Every run printed the banner and `151493 cycles`.

| Run | MHz |
|---|---:|
| Verilator, RTL (`top.v`, `picorv32.v`; `-O3 --x-assign fast --x-initial fast`) | 12.2–12.5 |
| The same with clang profile-guided optimisation | 16.3–16.6 |
| The same with `--threads 2` / `--threads 4` | 5.1 / 3.5 |
| Verilator, Yosys `synth_ecp5` netlist of the SoC with the RAM behavioural (1337 LUT4, 186 CCU2C, 637 FF, 32 DPR16X4) | 1.16–1.20 |
| Fast mode, the bitstream, same measurement series | 1.93–1.94 |

On the same netlist, fast mode is 1.6 times faster than Verilator. Verilator on the RTL is ten
times faster than on the netlist: it keeps the words, the enables and the structure that synthesis
dissolves into LUTs. Even there, PicoRV32 stays below the board's 25 MHz.

Measured and rejected. All three runs were cycle for cycle correct; the C compiles to more or slower
machine code:
1. **Reduced LUT tables.** Constant inputs substituted into a LUT with two or more nets, and the
   inputs it does not depend on dropped: about 18 MHz instead of 22 on `uart_hello`.
2. **Carry chains as one addition.** P and G packed into a 64-bit word, and every carry taken from
   `(P | G) + G + cin`. The per-bit packing costs more than the chain:
   - `blinky_pll` 12.8 → 5.9 MHz;
   - `uart_hello` 27 → 21 MHz;
   - HC06 3.4 → 2.5 MHz.

   With the LUT folding in place, the packed chain was still slower (`blinky_pll` 7.5 against 14.4).
3. **Selecting each register by the edge** (`edge ? next : q`) in the common path. clang turns a
   select of two loads into a load from a selected address, and every load then waits for the edge
   test: `ddr_pattern` ran at 45 MHz instead of 129 with the same logic. The branches above replace
   it.
4. **Skipping blocks whose inputs did not change**, and gates taken by the registers they depend
   on: see "Where a soft CPU spends its time". Gates on demand replaced them.

## Checks

`tests/fast_mode.rs` (`--ignored`; needs clang and the reference builds in `target/reference`).
Fast mode and the event kernel run the same clock and agree on every sample:
- **`uart_hello`:** 40 000 cycles of the UART line, which decodes to `Hello, ULX3S!`.
- **`debounce`:** all eight LEDs on every cycle over 405 000 cycles. A bouncing button is pressed
  and released, a pulse too short to count is applied, then a second press. Inputs change with
  `set_at`, and the count ends at 2.
- **`blinky_pll`:** all 32 counter bits, `led` and `locked`, before the PLL starts and after.
  - 2000 cycles are compared after each falling edge and a quarter period after each rising edge.
  - 8192 cycles run in one call through the hyperperiod table, and every probe is compared after
    every falling edge.
  - 1000 more cycles check the time keeping after the table.
  - The counter ends at 4n − 6.
- **`ddr_pattern`:** the four ODDRX1F lanes and their LVCMOS33D complements, in both clock phases,
  over 2000 cycles.
- **HC06:** the UART line through the first report, 176 000 cycles; most of them run through the
  table with the OSC as a second edge train. The report reads `F=00C666`, 310 MHz / 16 sampled over
  65 536 cycles.
- **`uart_ram`:** the event kernel runs 25 cycles until the RAM's read latch is known, then fast
  mode takes over. The UART line agrees through the whole message, `Hello from block RAM!`.
- **`picorv32_console`:** the event kernel runs 66 cycles until the EBR latches are known, then
  both agree on 101 bits after every one of 20 000 cycles. The bits are the program counter, the
  ALU output, the store data read from the register file, the LEDs and the UART line, through the
  clearing of `.bss` and into the sieve.
  - Fast mode alone then reaches the console. The text is that of an RTL simulation of `top.v` with
    iverilog: `primes below 2000: 303, 151493 cycles`.
  - `hi` and Enter sent to the receiver come back as `HI` and a new prompt.
  - With the LUTRAM write stored after the flip-flops (a wrong order), the comparison fails at cycle
    9588.
  - Through the CLI (`the_cli_console_talks_to_the_picorv32_firmware`): `--console-out` and
    `--console-in` on pipes. `hi` and CR written after the prompt come back as `HI`. Stdout holds
    the console text and nothing else.
  - A firmware with another banner, swapped in with `examples/ulx3s/swap.sh` (ecpbram, no place
    and route), prints that banner. Its C program is the same source as the first firmware's, so
    the compiled library is reused (`a_firmware_swapped_with_ecpbram_runs_without_place_and_route`).
- **`tests/lutram.rs`, random LUTRAM traffic:** two runs of 20 000 instants, every input at random.
  - One run writes on the rising edge with an enable and the write inhibit; the other on the
    inverted clock with an inverted enable.
  - A flip-flop on the write edge registers the write data, and another one reads the output.
  - The output, both flip-flops and all sixteen cells agree after every instant.
  - Storing the write after the flip-flops fails in both runs: one takes the branch for one group,
    the other the general path.
- **The DP16KD fixtures of `tests/ebr_native.rs`** (inverted clocks, CS decoders, WRITETHROUGH,
  READBEFOREWRITE with OUTREG, inverted or tied WEA):
  - 20 000 random instants each, every output after every instant;
  - then writes with one byte group masked. They show the kernel's X on the pins, and stop fast
    mode exactly when the X reaches `qa[0]`, which a flip-flop reads.
- **`tests/ebr.rs`, 37 configurations:** every DP width pair, and PDP36 with each read width twice
  (36 included, since 2026-11-11), with random write modes, output registers, decoders and reset
  kinds.
  - 3000 random instants each, then the whole array.
  - One pass avoids what the kernel makes X.
  - A second pass draws masks, resets during writes, GWDIS, OCE without access and both clocks
    at once. Its X outputs (46 in 10 configurations) agree with the kernel bit for bit.
- **DSP** (2026-11-11):
  - `tests/dsp.rs` `fast_mode_matches_the_event_kernel_on_combinational_multipliers`: two
    MULT18, one with dynamic signs, one with inverted and tied operand bits and a signed A,
    and a flip-flop on a product bit. 3000 random instants; every P, SIGNEDP, ROA/ROB and the
    flip-flop agree after each.
  - `tests/dsp.rs` `fast_mode_matches_the_event_kernel_on_multiplier_register_stages`: the 15
    combinations of INPUTA, INPUTB, PIPELINE and OUTPUT, each with SYNC and with ASYNC resets.
    Four independent clocks, random CE/RST and GSR (resets at clock edges too), 2000 random
    instants each after the warm start; every P, SIGNEDP, ROA and ROB agree.
  - `fast_mode_refuses_an_output_stage_with_driven_enables` (`tests/dsp.rs`).
  - `tests/dsp_register_native.rs` `fast_mode_writes_the_kernel_vcd_on_the_register_benches`:
    the four-MULT18 SYNC and ASYNC benches (INPUTA, INPUTB, PIPELINE, all three) with their 513
    stimulus events, a user GSR from a pin and resets at clock edges. 148 probes each,
    byte-identical VCD.
- **Tristate and bidirectional pads** (2026-11-11), `tests/fast_bidir.rs`, a design built with
  Yosys/nextpnr:
  - pads: open drain with a pull-up, a tristate with a pull-down (both read back into a shift
    register on the LEDs), and a tristate with no pull that only a probe sees;
  - 300 cycles, every LED and pad agrees after every cycle, the floating pad as Z in both;
  - the CLI's VCD of the two executors is byte-identical, `z` included;
  - with the pull-down removed, the logic would read a floating pad: refused, with the reason.
- **Resets at a clock edge and a user GSR** (2026-11-11):
  - `tests/fast_reset_order.rs`: FF1 with an asynchronous reset, FF2 reading Q1.
    - The reset and FF2's clock rise together: FF2 takes the old Q1, as in the kernel.
    - The reset and FF1's own clock rise together: FF1 takes the reset value.
  - `tests/fast_mode.rs` `fast_mode_follows_a_user_gsr_driven_by_logic`: a counter and
    `GSR(.GSR(~btn[1]))` built with Yosys/nextpnr. The button is pressed between edges and at a
    rising edge; every LED agrees with the kernel on every cycle.
  - Byte-identical VCD files:
    - the `dsp_native` fixture (two MULT18, dynamic signs, 513 stimulus events, 74 probes);
    - a yosys design with an unsigned and a signed 16×16 product, 20 000 cycles. Its LEDs also
      match a Python model of the Verilog in every cycle.
  - `examples/ulx3s/mac` (seven MULT18) prints the line `MAC 003D16367D MIX CE196146` that the
    arithmetic of its Verilog gives.
- **Clock gates and the X2 gearbox against the board** (2026-11-11):
  - `tests/clock_gate_native.rs`: `hc01.config` (DCCA with a dynamic CE) prints the twelve
    lines of the board in both executors.
  - `tests/gearbox_native.rs`: `hi03.config` (ECLKSYNCB, CLKDIVF, ODDRX2F, IDDRX2F) prints
    exactly the board's line in both executors.
    - The CLI's VCD of the two executors is byte-identical over 420 000 cycles.
    - The design is also the case for the carry-in rule above: two carry chains meet in one
      row and feed each other through the UART's character mux.
  - `tests/edge_clock.rs`: the ECLKSYNCB and CLKDIVF rules of HI04 on a synthetic plan, and
    fast mode against the kernel over 40 000 units of random STOP, RST and ALIGNWD.
  - `tests/cross_domain_fast.rs`: clocks made by an IOLOGIC capture, whose edges come one wave
    after their source's in the same instant. All four modes, every net compared after every
    step:
    - A DCCA whose CE comes from a CLKI register, with registers on CLKI and CLKO passing data
      both ways under a random CE; 30 000 random steps.
    - The X2 gearbox: CLKDIVF on ECLK, SCLK ranks into ODDRX2F, its pad into IDDRX2F, and
      SCLK registers taking the holding nets that the mid rise writes in the same instant.
      Random data and divider RST changing with ECLK edges, 40 000 ECLK edges.
    - The same through the 7:1 gearbox with CLKDIVF /3.5.
  - `tests/gearbox71_native.rs`: `hi07.config` and `hi07b.config` print the board's lines
    exactly (HI07 also in the event kernel).
  - `tests/edge_clock_native.rs`: `hi04.config` and `hi04b.config`, two passes each in fast
    mode. Every line printed is a board line, except two of the first pass that the board
    prints before its reader opens the port.
- **IO registers with CE, set/reset and GSR** (2026-11-11):
  - `tests/io_register.rs`: the HI05 rules on a synthetic plan, and fast mode against the
    kernel over 30 000 steps of random clock, data, CE, set/reset and GSR.
  - `tests/io_register_native.rs`: `hi05.config` (eight IO registers, a user GSR driven by
    logic) for two rounds in fast mode. Every line is a board line, except the first window,
    which the board prints before its reader opens the port.

`emulate --fast` and `emulate` write byte-identical VCD files, last checked after the console
loop, for:
- `uart_hello`: 100 000 cycles;
- `debounce`: 450 000 cycles with 751 stimulus events of a bouncing button (the script of
  `tests/reference_designs.rs`), 376 of them on rising edges and the rest between edges;
- `blinky_pll`: 3000 cycles, by HDL name;
- `ddr_pattern`: 3000 cycles;
- HC06: 120 000 cycles;
- `uart_ram`: 60 000 cycles, 31 of them in the event kernel before fast mode takes over;
- `picorv32_console`: 20 000 cycles of the program counter, the ALU output, the store data, `led`
  and the UART line, 127 of them in the event kernel.

### A compiler fault on AVX-512

On an AMD Zen 4 engineering sample under `clang` 21.1.8 with `-O3 -march=native`, the Life demo
produced a deterministic wrong LED trace only when built unfolded on two threads. Eight adjacent
32-bit LUT lines were SLP-vectorized and lowered to AVX-512 (`vpsllvw`, `vpsllvd`, `vpsrlvd`,
`vpermi2d`, `vpternlogd`, `vpmovzxbd/bw`, `vpinsrd/b`), giving wrong outputs on 57 344 of the 2¹⁷
input combinations; `-fno-slp-vectorize`, `-march=x86-64-v3`, `gcc` 15.2.0, and Apple `clang` 17 on
`arm64` all give zero mismatches. An SDM interpreter of the emitted assembly matches the CPU on
all inputs, proving an LLVM x86 backend bug rather than a CPU erratum. It is reduced to seven IR
instructions, triggered by AVX512BW, and was reported as
[LLVM issue #243602](https://github.com/llvm/llvm-project/issues/243602) on 2026-11-17.

The generator now writes LUT constants as 64-bit `ull` (which gives zero mismatches under every
tested flag set), and on x86 builds the generated C without AVX-512 unless `FAST_AVX512=1`. The reproducer and measurements are in [LLVM issue #243602](https://github.com/llvm/llvm-project/issues/243602).

### A large design: Life

`examples/ulx3s/life`, Conway's Game of Life on a 27×27 torus, is 10 376 LUT4, 739 FF and about
3 900 wide muxes, and every one of them works on every cycle: its settle has 14 317 lines.
Apple M3 Max, 2026-11-09, `emulate --fast` for one million cycles, three runs each:

| Settle | Lines | 1 thread | 3 threads | 4 threads |
|---|---:|---:|---:|---:|
| As written | 14 317 | 59–60 kHz | 156–422 kHz | 331–406 kHz |
| Folded, up to 8 inputs | 5 662 | 100–101 kHz | 737–765 kHz | 707–727 kHz |

The LED trace is the same in every row. One thread spends about 1.3 ns a line, which no compiler
flag changed (`-O3` 60 kHz, `-Os` 60, `-O2` 56, `-O1` 48). With several threads each core runs a
part small enough for its instruction cache, which is why three threads run more than three
times as fast. Unfolded, the threaded runs varied from run to run; folded, they agree within 4%.

Folding on the small designs, same build: `uart_hello` 47 → 49.5 MHz, `picorv32_console`
2.06 → 2.18 MHz, `blinky_pll` 17.8 MHz either way. One build ran `blinky_pll` at 10.6 MHz with
`FAST_COLLAPSE=8` and 17.8 with `0`, with the same compiled program: the host's layout in memory
alone moved it. Compare speeds within one build.

### Life on a busy machine

With other programs running, the three threads of a settle lose the most: every settle ends when
the last part is done, so a worker the system has not scheduled holds the settle up until it runs
again. On 2026-11-09, with Blender and a game open, three runs in a row gave 154, 165 and 629 kHz.
The pool now lets the caller run a part that no worker has claimed (see *Threads* above).

Measured on 2026-11-10 on the M3 Max (two clusters of 5 performance cores, 4 efficiency cores),
with Life as it is now: 10 853 LUT4, 6 018 lines after folding, split into 66 shared lines and
parts of 1 866, 2 047 and 2 039 lines. Its quiet speed is about 690 kHz. The load was N copies of a
busy loop at the default QoS, or at `USER_INTERACTIVE` (the pool's own QoS, as a game's render
thread has), on top of whatever else ran. "None" means no busy loops while Blender rendered (load
average about 20). The command is the standalone one below, six runs of each in turns,
one million cycles with no busy loops and 300 000 with them. Median, with the range:

| Load | 1 thread | Pool before | Pool now | Pool now, `FAST_THREAD_STEAL=0` |
|---|---:|---:|---:|---:|
| None | 86 (85–87) | 453 (432–650) | 627 (455–662) | 544 (364–684) |
| 8 busy loops | 80 (74–86) | 183 (167–366) | 215 (159–273) | 223 (175–300) |
| 14 busy loops | 62 (57–68) | 170 (130–186) | 195 (137–200) | 162 (129–192) |
| 8 busy loops at `USER_INTERACTIVE` | 74 (70–85) | 257 (190–297) | 255 (209–290) | 213 (175–351) |

```bash
FAST_THREAD_REPORT=1 ecp5_emu emulate target/reference/life/top.config \
  --names target/reference/life/top_routed.json --fast --startup configured --scope observed \
  --clock net:clk_25mhz --period-ns 40 --cycles 1000000 --probe net:led
```

The LED trace is the same in every run. Run to run, the speed moves by a factor of two or more under
these loads, so the medians say less than the waits do. `FAST_THREAD_STATS` counts the time the
caller spent in waits of 42.7 µs or more, three runs at each load:

| Load | Share of the run waiting, without taking parts | Taking parts | Parts taken in 300 000 settles |
|---|---:|---:|---:|
| None | 0–4.6% | 0.1–1.8% | 63–1 407 |
| 8 busy loops | 14–23% | 3–13% | 14 168–25 103 |
| 14 busy loops | 23–33% | 8–18% | 16 112–21 571 |
| 8 busy loops at `USER_INTERACTIVE` | 0.3–29% | 1.2–12% | 9 358–18 717 |

What is left of the waiting is a worker preempted in the middle of its part: the caller cannot run
that part again, since the worker would go on writing its slots when it came back. The other loss
under load is the kind of core: busy, the system gave the workers efficiency cores for 15 to 45% of
their parts, which take twice as long there, and the settle waits for the slowest part. Smaller
parts would let the others take over from a slow one; split for 6, 9 and 12 threads, Life gets 82
to 146 shared lines (66 for three), uneven parts or a second phase, and this was not taken further.

In no load tried did the threads run slower than one thread: from 2.3 times as fast (the pool
before, 8 busy loops) to 7 (none). A switch to one thread when the threads stall would therefore never have helped, and
was not added. Measured and dropped:
- a waiting worker calling `sched_yield` after 4 000 spins: slower with 0, 8 and 14 busy loops,
  no faster with 20;
- the caller calling `sched_yield` at the meeting, or sleeping on a condition variable until the
  last worker wakes it: no faster under load, and the sleep 3–7% slower with none;
- 20 000 spins before a worker sleeps instead of 200 000, or the `USER_INITIATED` QoS: no faster;
- two threads instead of three: no faster;
- taking parts after 500, 8 000 or 16 000 spins instead of 2 000: no difference beyond the noise.
  A worker claims its part within 0.6 µs in 99.95% of settles; between 0.7 and 5 µs it does so
  a few dozen times in 300 000, and the others, a few hundred at most, come later than 5 µs, so
  2.5 µs sits in the gap.
  With 14 busy loops the caller took parts in 5–7% of settles, so the 2.5 µs it waits first
  costs about 3% there.

On a quiet machine the pool costs nothing measurable. Life in one process, eight libraries of
each pool (the same program built with different unused `-D` flags) run in turns, 30 stretches
of 20 000 cycles each: the 90th percentile of a library's speed was 694.6–699.0 kHz with the pool
before and 694.9–698.9 with this one, over the libraries that ran at full speed (six of eight
before, seven of eight now). The others ran 10 to 15% slower, and so did about one library in
seven over all such runs, with either pool. In one process a second library of the same program
ran 7.6% slower than the first. The code of one part is about 84 KB; why the place a library
lands at moves its speed was not looked into. Compare pools across several libraries, not one.
Several simulations in one process also understate a busy machine: each worker runs only part of
the time and the system preempts it less, so the loaded figures above come from separate runs.

### Tuning on Life

On this M3 Max the defaults are close to the best for Life, so tuning mostly keeps them.
Measured in turns against 3 threads folding to 8 inputs, in three runs at a load average of 7 to
9: 4 threads 78–109%, folding to 6 inputs 91–100%, to 4 inputs 67–82%, 2 threads 77–84%,
8 threads 57–78%, folding off 64–73%, 6 threads 46–49%, one thread 12–16%. Under 8 busy loops,
separate runs with `FAST_TUNE=0` gave 2 threads 9% above 3 and 4 threads 16% below (six runs
each in turns), so there the choice does move.

`emulate --fast` for four million cycles, six runs of each in turns, `FAST_TUNE=0` against tuning,
with no memory of an earlier choice at the start (each tuned run then left one for the next):

| Load | Tuned over untuned |
|---|---:|
| None (concurrent builds with yosys and nextpnr, load average about 9–15) | 1.01 |
| 8 busy loops | 1.03 |
| 8 busy loops at `USER_INTERACTIVE` | 0.955 |

The LED trace is the same in every run. What tuning costs is its measurements: with every
candidate known to be slower and nothing measured, a tuned run took 13.51 s and an untuned one
13.51 s, and 12.18 s against 12.27 s. Before the trial was added, programs that only measured
faster in turns took over, and tuning lost 4–8% under load. Building the eight candidates takes
about 0.9 s before the run starts; a candidate not in the cache took about 18 s to compile at
the time (2.6 s now, *Compiling*).

### The full-chip Life: life_full

`examples/ulx3s/life_full` is Life on a 72×72 torus: 78 700 LUT4, 5 230 flip-flops, 94% of the
chip. Its per-cycle settle has 108 444 lines, 42 587 after folding. On 2026-11-10 it ran at about
29 kHz with three threads; the goal is 500 kHz.

Threads, 100 000 cycles each (`FAST_TUNE=0`, the split of the settle as `FAST_THREAD_REPORT`
prints it):

| Threads | 1 | 3 | 4 | 6 | 8 | 10 | 12 |
|---|---:|---:|---:|---:|---:|---:|---:|
| kHz | 9.5 | 30 | 36 | 43 | 51 | 55 | 46 |

From 8 threads on the split shares 784 lines and adds a second phase of 600 (the population
counter's adder tree). A profile at 10 threads: the caller waited for the workers about half the
time, and a part of 4 300 lines took 5–11 µs on a performance core (1.2–2.5 ns a line, against
0.6 for Life's parts) and twice as long on an efficiency core. The slots (160 KB) and the tables
(5 360 of 32 bytes, 1 092 different) do not fit the 128 KB data cache, and every register was
on a line of its own, written by the caller's capture each cycle and read by every thread.

What the layout changes above did, same machine (Blender rendering at times): 8 threads 60–64 kHz
(from 49–52), 10 threads 54–66 (from 46–61), 3 threads 33–35 (from 30). The VCD over 20 000 cycles
is byte for byte the same.

How far it can go this way. A table line is about eight loads and twenty operations, 0.6–0.8 ns
on a core even from the cache; 42 587 lines are about 30 µs of core time a cycle, which on ten
performance cores is at best 3 µs: about 330 kHz with no meeting or capture at all. 500 kHz needs
fewer lines a cycle. In a cycle 9 000–12 000 of the 160 000 slots change; 18–25% of the settle's
lines have an input that changed, but in any run of 32 lines of the program one does (every cell
has a line that reads the noise register, which moves every cycle). Grouped as ESSENT does
(Beamer and Donofrio, DAC 2020: a line joins its readers' group when they are all in one), the
settle is 13 600 groups of about 3 lines, and 22–30% of the lines are in a group with an input
that changed: evaluating only those would leave about a quarter of the work, before the cost of
the checks and of marking readers.

That is built (`FAST_ACTIVITY=1`, off by default), and it is slower. Each thread's part keeps its
lines in such groups, each with a flag: a group runs only when its flag is set, and a changed
output sets the flags of the groups of the part that read it. What a part reads from outside it
compares at its start with a copy kept from the last time, so any change, whoever made it,
sets the flags of its readers. The traces are the same (Life, `life_full` at 3 and 8 threads,
`picorv32_console`). But on `life_full` at 8 threads the parts read 3 000–4 300 slots from
outside each, nearly one per line (every cell reads its neighbours' registers and the noise),
31 300 comparisons a cycle in all with 68 000 flags to set, more than the lines saved:
50 kHz against 60, Life at 3 threads 300 kHz against 690, `picorv32_console` 1.47 MHz against
2.24. The groups are small (7 375 of one line, 5 100 of six or seven: a cell) and a cell's
inputs change somewhere almost every cycle.

The parts were also larger than the instruction cache: a line is 40–47 bytes of machine code, so
a part of 4 000–5 000 lines was 180–235 KB against the 192 KB of a performance core. How much
that costs, measured by linking the compiled units of one 8-thread part to a loop that calls its
pieces in one thread (no other thread, every slot in the data cache):

| Pieces of the part | Machine code | ns a line |
|---|---:|---:|
| 7 | 135 KB | 0.31 |
| 8 | 163 KB | 0.33 |
| 9 | 197 KB | 0.55 |
| 10 | 222 KB | 1.66 |
| 11 (all) | 231 KB | 1.86 |

A cliff, six times, at the cache's size: a part that does not fit runs from the second-level
cache. From the cache a line takes 0.3 ns, not the 0.6–0.8 estimated above, and its time follows
its loads (3.4 a line, of the inputs): byte tables for lines of up to six inputs saved 6% of the
code and cost 25% of the time, an extra load each, so those keep their constant. What made the
code smaller without more loads: twins and LUT specialization (folding, above) 8%, byte tables
for seven and eight inputs 2%. At 10 threads the workers' parts went from 179–187 KB to 158–171.
Before and after, three rounds each (2026-11-10, machine load 5–11):

| | Before | After |
|---|---:|---:|
| `life_full`, 8 threads | 56–64 kHz | 76–82 kHz |
| `life_full`, 10 threads | 73–95 kHz | 100–112 kHz |
| Life, 3 threads | 384 (first run), 645–707 kHz | 811–862 kHz |
| `picorv32_console` | 2.21–2.30 MHz | 2.31–2.39 MHz |

The traces are the same (`life_full` VCD over 20 000 cycles, Life's LED trace over a million).

Where a 10-thread cycle of `life_full` goes now, from timestamps put into the generated program
(300 000 cycles, `FAST_FOLD=cones`, 12 inputs, the caller's part limited to fit its code; the
default build is within a few percent):

| | µs | Alone in one thread |
|---|---:|---:|
| Whole cycle | 9.2–9.3 | |
| Captures (5 230 registers), caller | 1.85 | 0.29 |
| Shared lines (1 373), caller | 1.72 | |
| First phase, publish to the last worker done | 4.7–4.8 | 1.5 a part |
| Second phase (population counter, 1 069 lines), caller | 0.9 | |

97% of the cycles fall in one bin of 5.3–10.6 µs: not the odd preempted worker but every cycle.
Each part, run alone, takes 1.5 µs, and 2.2 µs when another thread does the captures and shared
lines between its runs. What costs is moving data between cores: the caller reads every register's
next value from the nine workers' caches and writes all 5 230 registers, and every worker then
reads 3 000–4 000 of them, on 48 of the 51 cache lines they fill. A part's cells are not
neighbours (the split shares computed lines, and the cells of a grid share none, only registers),
so every thread reads registers from all over the grid. Neither giving the caller a smaller part
so that its code fits (`FAST_CODE_BUDGET`; it then held more of the population counter as shared
lines, 1 431 instead of 816) nor 15% fewer lines (`FAST_FOLD=cones`) changed the cycle; nor did
favouring, in the split, the thread that already reads a cone's registers (tried and taken out:
on `life_full` each register is read by more than 16 cones once the population counter's are
counted, so the order gives no blocks).

On a loaded machine many threads do not lose a few percent but most of the speed. The window's
choice at start (250 ms a candidate) measured 10 threads at 95 kHz with a load of 10 and at
4.1 kHz with a load of 13–16, while 6 threads ran 57. With 8 busy loops, 100 000 cycles: the
caller ran 830 000 parts of workers the system had not scheduled, in turn after its own, and
waited 1.4 s of 4.8 for workers preempted in the middle of theirs. Two changes (2026-11-11):
workers done with their part take the unclaimed parts too (*Threads*), and tuning, while the
speed is noisy, still measures programs with fewer threads (*Tuning*). Before and after, two
rounds each:

| `life_full` | Before | After |
|---|---:|---:|
| 10 threads, quiet | 108–116 kHz | 110–117 kHz |
| 10 threads, 8 busy loops | 20.4–26.0 kHz | 36.4 kHz |
| 5 threads, 8 busy loops | 32.2–41.9 kHz | 32.1–40.3 kHz |
| Tuned from 10 threads, 8 busy loops, 2 M cycles | 27.9–36.7 kHz (to 3 or 4 threads) | 34.0–36.4 kHz (to 5 threads) |

On the quiet machine both builds had runs at 52–78 kHz when other work added load.
The traces are the same with workers taking parts, also when they take them at once
(`POOL_STEAL_SPINS=0`, 12 busy loops: 570 000 parts taken by workers in 200 000 settles of Life
on 10 threads; the `life_full` VCD).

Floors of that cycle (2026-11-16), from a harness that links the program's compiled units and
runs the real parts of the 10-thread `life_full` program, with a meeting of the threads of its
own (a generation published, a line a part for its end):

| Run | µs |
|---|---:|
| One part alone | 1.03–1.57 |
| Captures / shared lines / second phase, alone | 0.29 / 0.27 / 0.22 |
| Ten parts at once, each on its own copy of the slots | 2.1 an iteration, parts 1.55 |
| Ten parts at once on one array, no capture | 3.5–3.9, parts 2.5–2.8 |
| The whole cycle (captures, shared lines, parts, second phase) | 5.6–6.0 |
| The meeting alone, empty parts: one counter / a line a part | 0.71 / 0.33–0.40 |

Ten cores at once do not slow a part down; one array did, because every part read 7–9 cache
lines on which another part wrote (registers next to the nets a thread writes). With the
registers on lines of their own the same harness, in a busier window, ran parts on one array as
fast as on copies (1.88 µs against 1.82) and the whole cycle at 9.1 µs against 12.0 for the old
layout. The meeting now uses a line a part, and the second phase, which only the caller has,
runs without one. `emulate`, same window, three rounds: 8 threads 79–80 → 82–84 kHz, Life
628–682 → 661–799 kHz, `picorv32_console` unchanged; 10 threads varied more than they gained.

#### Owner capture and a split by place on rserv

The following numbers are rserv's: a 32-core AMD with four CCDs, the process confined with
`taskset` and the harness threads pinned to their cores. Its cores are slower per line than the
M3's, and traffic between CCDs costs more. Of the 13 622 lines of work in a cycle, 8 376 are the
population counter, which reads every register; about 5 200 are the cells.

`FAST_SPLIT=place` cuts the sinks of a phase by their place on the chip (the routed net's tile)
into blocks of equal weight. Each sink then moves to the block where most other sinks reading
its registers are, while keeping the blocks within 3% of equal weight. Lines in the cones of
more than 16 sinks, the noise behind the cells' turn, neither draw a cone to a block nor count
in its load. At 8 threads the number of registers parts read from another thread fell from
25 063 to 9 810, summed over the parts. But every part still reads most of the 47 register
cache lines: the population counter needs them all.

Owner capture is now the default (`FAST_CAPTURE=owner` also selects it);
`FAST_CAPTURE=caller` restores the caller's capture and layout. Owner capture runs group-1
captures on the thread that computes what they read, through
`pool_run` before the settle. Group 2 starts with the registers other captures read, then
closes over the registers its captures read; it stays on the caller after group 1. Group 1
therefore sees the old group-2 values, and group 2 reads nothing group 1 writes. Registers are
laid out in their owner's region, on its own cache lines. This applies only to a threaded group
on the single-group path, and not with `FAST_ACTIVITY`. `FAST_SPLIT=place` remains a separate
flag, off by default.

The harness at 8 threads on one CCD:

| Captures and layout | Capture step, µs | Whole cycle, µs |
|---|---:|---:|
| Caller captures | | 8.63 |
| Owner capture, old layout (a register belongs to the thread that reads it most) | 7.3 | |
| Owner capture, owner layout and place split | 0.64–0.68 | 6.49–6.54 |

On the old layout the owners wrote cache lines shared with other owners; the capture step
alone cost nearly as much as the whole cycle with caller captures. Giving each owner its own
register lines removed that traffic.

End-to-end `emulate`, `life_full`, 200 000 cycles, `FAST_TUNE=0`, three interleaved rounds:

| | caller capture (previous default) | owner capture | caller + place split | owner + place split |
|---|---:|---:|---:|---:|
| 8 threads (one CCD) | 1.97–2.16 s | 1.27–1.41 s | 1.76–1.83 s | 1.26–1.33 s (one 1.78) |
| 10 threads (two CCDs) | 4.31–4.76 s | 2.09–2.24 s | 3.91–3.98 s | 1.96–2.04 s |

At 8 threads that is about 100 → 150–160 kHz; at 10 threads, 45 → 95–100 kHz. The LED trace
over 20 000 cycles is identical with each flag combination at 8 and 10 threads.
`FAST_CAPTURE=caller` generates byte for byte the previous default program. On rserv at 10
threads over two CCDs under load, owner capture was worse in 2 of 3 noisy rounds.

On the quiet M3 Max, `life_full`, 200 000 cycles, `FAST_TUNE=0`:

| | caller | owner | owner + place |
|---|---:|---:|---:|
| 8 threads | 1.84–2.03 s | 0.98–1.08 s | 0.93–1.02 s |
| 10 threads | 1.29–1.44 s | 0.85–0.89 s | 0.81–0.88 s |

Owner capture raised throughput from about 105 to 195 kHz at 8 threads and 145 to 230 kHz at
10 threads. Under 8 busy loops, 100 000 cycles, the 8-thread rounds were caller
1.47 / 1.62 / 1.87 s, owner 1.35 / 1.56 / 2.79 s, and owner + place 1.32 / 1.38 / 1.49 s:
the owner median improved despite one slow outlier. At 10 threads there was one valid round:
caller 2.00 s, owner 1.38 s, owner + place 1.55 s. These M3 results make owner capture the
default; the place split stays opt-in.

#### Measured on the M3 Max, 2026-11-17

Base command, plus the options in each table:

```bash
emulate target/reference/life_full/top.config --names target/reference/life_full/top_routed.json --fast --startup configured --scope observed --clock net:clk_25mhz --period-ns 40 --probe net:led
```

**Throughput.** Seconds, with kHz in brackets (`200 000 / seconds`):

| Threads | Build | Load | Options | caller | owner | owner + place |
|---|---|---|---|---:|---:|---:|
| 8 | `7054d98` | quiet (load average 3.6–4.9) | `--cycles 200000`, `FAST_TUNE=0 FAST_CHOOSE=0`, `FAST_THREADS=8` | 1.84–2.03 s (98–109 kHz) | 0.98–1.08 s (185–204 kHz) | 0.93–1.02 s (196–215 kHz) |
| 10 | `7054d98` | quiet (load average 3.6–4.9) | `--cycles 200000`, `FAST_TUNE=0 FAST_CHOOSE=0`, `FAST_THREADS=10` | 1.29–1.44 s (139–155 kHz) | 0.85–0.89 s (225–235 kHz) | 0.81–0.88 s (227–247 kHz) |

**Under load.** Rounds were skipped when the load from anything else was above 6:

| Threads | Build | Load | Options | caller | owner | owner + place |
|---|---|---|---|---:|---:|---:|
| 8 | `7054d98` | 8 busy loops | `--cycles 100000`, `FAST_TUNE=0 FAST_CHOOSE=0`, `FAST_THREADS=8` | 1.47 / 1.62 / 1.87 s | 1.35 / 1.56 / 2.79 s | 1.32 / 1.38 / 1.49 s |
| 10 (one valid round) | `7054d98` | 8 busy loops | `--cycles 100000`, `FAST_TUNE=0 FAST_CHOOSE=0`, `FAST_THREADS=10` | 2.00 s | 1.38 s | 1.55 s |

**Opening.** Owner capture is the default on this build; one fresh cache folder was used for the
three runs in order (`--cycles 2000000`, long enough to choose settings):

| Run | Build | Load | Options | Fast mode's start (program generated, compiled or loaded) | Settings choice |
|---|---|---|---|---:|---:|
| first open with `FAST_CHOOSE=full` | `20e5d19` | 4.2–4.9 | `--cycles 2000000`, `FAST_TUNE=0`, `FAST_CHOOSE=full` | 6.29 s (cold compile) | 82.4 s (full search) |
| next open | `20e5d19` | 4.2–4.9 | `--cycles 2000000`, `FAST_TUNE=0` | 1.31 s | 6.90 s (the one-trial check fell short of half the remembered speed, so a round of trials ran) |
| next open | `20e5d19` | 4.2–4.9 | `--cycles 2000000`, `FAST_TUNE=0` | 1.23 s | 0.45 s (one trial, reusing the program) |

Since `2850aac` a first open with no memory runs no trials, so its cost is one cold compile (6.3 s
here). The fallback in the middle row comes after a `FAST_CHOOSE=full` search, whose 250 ms trials
measure higher than a later check; in normal use the tuner writes the memory from run speeds.

#### Where a cycle goes now (harness)

M3 Max, 2026-11-17, harness `tools/fast_floors` (owner mode over a `FAST_CAPTURE=caller
FAST_SPLIT=place` program, so owner capture is emulated):

| Threads | Cycle | Captures | Group 2 + shared lines (caller) | Parts + meeting | Phase 1 (caller) |
|---|---:|---:|---:|---:|---:|
| 8 | 4.91–5.06 µs | 0.57–0.59 | 0.71–0.76 | 2.42–2.63 | 1.14 |
| 10 (quietest round) | 4.22 µs | 0.54 | 0.55 | 2.25 | 0.86 |

Alone, the same pieces take 0.29 (captures), 0.25 (shared), 0.79–1.16 (a part) and 0.43 µs
(phase 1). In a cycle they take 2.6–3 times longer, because they read cache lines every other core
has just written.

Deferring phase 1 (the population counter's last adders, which only the population register reads)
to an extra thread that runs it beside the next cycle did not pay. Two versions were correct but
slower on rserv (−4 to −7%): the register that reads phase 1 is captured at the very next rising
edge, after only the falling edge, so there is no time to overlap, and the LEDs read that register
every cycle. The patch is not in the tree.

## Next

- For a large design on many threads:
  - owner capture is done (the default) and the split by place is a flag;
  - left: the shared lines recomputed by each thread that reads them (about 11% of a cycle on
    rserv; to be measured on the M3), and registers in two banks swapped at the edge so that a
    capture is no copy.
- Word operations for bit-sliced datapaths, the next lever for soft CPUs. Measured at 1.6 to 2.4
  times on PicoRV32 (see above), which is not enough alone for 25 MHz.
- A console in the event kernel, for designs fast mode refuses.
- One product computation per multiplier instead of one per product bit: a line with 36
  outputs, or the word operations above. It matters for a soft CPU with a multiplier, whose
  operands change every cycle.
- The EBR read latches after configuration measured on the board. The kernel keeps them X, which
  is why the warm start exists. If the board shows a fixed value, the kernel would take it and the
  warm start would usually be empty.
