# clock_domains: content across four clock domains

Three clock-domain crossings run together, without a CPU. Every pass transfers
262,144 16-bit FIFO words, 4,096 handshake words and 4,096 pulses. The word
streams are seeded by the pass number. Receivers check every word and compute
CRC32 over the actual received content. Passes repeat forever.

```text
25 MHz crystal -------- UART, control, reports, pulse source
  |
  +-- EHXPLLL: 600 MHz VCO
  |     +-- CLKOP 40 MHz ------ FIFO writer, pulse receiver
  |     |       | FIFO (one DP16KD)
  |     +-- CLKOS 33 1/3 MHz -- FIFO reader/checker
  |
  +<-------------------------- OSCG / 8: handshake source, nominal 38.75 MHz
          four-phase req/ack

25 MHz -- request toggle --> 40 MHz -- acknowledgement toggle --> 25 MHz
```

The PLL outputs share a VCO and have an exact 6:5 frequency ratio. Their
relationship repeats, but the FIFO does not depend on their phase. OSCG is
unrelated to the crystal. DIV=8 is an exact, unambiguous divider: the emulator's
nominal 310 MHz base gives 38.75 MHz. The HC06 board measurement of
311.8–311.97 MHz gives 38.975–38.99625 MHz, about +0.6%.

## The three circuits

**FIFO, 40 → 33 1/3 MHz.** [fifo.v](fifo.v) keeps local binary pointers for
addressing and registered Gray pointers for crossing. Two flip-flops in the
receiving domain synchronise each Gray pointer. Only one Gray bit changes per
source increment, so a sample represents an old or a new pointer rather than
a mixture of unrelated binary bits. Full compares the next write Gray pointer
with the synchronised read pointer with its top two bits inverted; empty
compares the next read Gray pointer with the synchronised write pointer.
The extra pointer bit distinguishes a full ring from an empty one.

The producer has 8,192 clocks of traffic followed by 8,192 clocks of silence.
During traffic it honours full; during silence the reader drains the FIFO.
Simulation asserts that full occurred and that empty occurred after at least
4,096 words, so startup emptiness alone cannot satisfy coverage. Counts of full
or empty cycles depend on phase and are deliberately absent from UART output.

Yosys maps the RAM to **one DP16KD, width 18 on both ports**, with 1,024 entries.
Sixteen bits carry data; each physical entry owns two complete nine-bit INIT9
rows. Different entries therefore never share a collision row. The read clock
enable is asserted only for a nonempty FIFO. The synchronised write pointer
announces completed writes; the read pointer frees an entry only after its
synchronous read has been issued. A one-cycle valid pipeline checks the EBR's
previous output. Pointer synchronisation delays reuse until that value is safe.
There is no speculative read at the empty pointer and no legal simultaneous
write/read of the same physical row, even when clock edges coincide.

FIFO data needs no bit synchronisers. The RAM holds an entry stable while the
write pointer crosses, and the protocol prevents its reuse until the reader
has removed it. The pointers transfer ownership of storage.

**Handshake, OSCG → 25 MHz.** [handshake.v](handshake.v) uses four phases:
request rises, acknowledgement rises, request falls, acknowledgement falls.
Each control crosses two flip-flops. The sender establishes the 32-bit payload
one source cycle before raising request and holds it through the entire
handshake. The receiver captures it once, after request has crossed. This is
a bundled-data crossing: the control establishes when the held bus is valid.

**Pulse, 25 → 40 MHz.** A narrow pulse could disappear between destination
edges. Instead the source toggles a bit, and the receiver recognises a change
after two synchroniser stages. It counts once and copies that toggle into an
acknowledgement, synchronised back. The source waits for acknowledgement before
launching another pulse. The final count returns by a **held-bus handshake**,
rather than a running multi-bit sample: the receiver holds its count and done
flag until control lowers run. After done crosses two flops, control captures
the stable count. The FIFO result bundle uses the same result protocol.

Pass seeds are also held buses under the run/ready handshake. Control changes
the seed while idle, then raises run; destination run synchronisers provide
settling time before the pattern begins. Control waits for every domain to
return idle before beginning another pass.

## Reset and synchronisers

[cdc.v](cdc.v) contains the two-flop synchroniser, local reset synchroniser and
registered Gray frequency counter. A first synchroniser stage can become
metastable on silicon; the second gives it time to settle. Application logic
uses the second stage only. The stages carry `ASYNC_REG` and `syn_preserve`
attributes. Physical Gray-bus routing must also avoid overlapping successive
source transitions: keep its delay/skew below a source period. The routed
build's worst cross-clock path is 8.23 ns, below the shortest 25 ns source
period; the FIFO pointer paths are at most 1.44 ns.

A binary multi-bit value must not be synchronised independently bit by bit.
For example, 0111 → 1000 changes four bits: different arrival times and
metastability can make the receiver briefly see a value that was never sent.
Two flops on every bit do not restore bus coherence. Use registered Gray code
for successive counters, held data plus a handshake for a word, or a FIFO for
a stream.

At power-on, `r`, or FIRE1, the board-clock controller holds a reset request
high for at least 64 board edges. Every domain, including 25 MHz, has its own
reset synchroniser: assertion is asynchronous, release takes two local clock
edges. The controller waits for PLL lock and all synchronised ready responses
before a pass starts. Reset clears pointers, counters, requests, results,
frequency measurement and error history. RAM contents need no clearing because
reset pointers make old entries inaccessible.

The UART and its 256-byte RX FIFO survive application restart. The transmitter
finishes its current byte, then prints `RESTART\n`; the remainder of an old
report is abandoned. Power-on starts pass 1 without a marker. Buttons use the
same 10 ms debounce and serial plumbing as [ebr_life](../ebr_life/README.md).

## Output and controls

All numeric fields are **eight hexadecimal digits**, including pass numbers,
word counts, pulse counts and optional frequency readings. CRC32 is the
reflected Ethernet polynomial 0xEDB88320, initial and final XOR 0xFFFFFFFF;
FIFO words contribute their two low-to-high bytes, handshake words four.

```text
RESTART
PASS 00000001 FIFO 00040000 ERR 00000000 CRC 9869C317 HS 00001000 ERR 00000000 CRC 77A7B89E PULSE 00001000 00001000
PASS 00000002 FIFO 00040000 ERR 00000000 CRC 569BA6B7 HS 00001000 ERR 00000000 CRC 6DF82F2E PULSE 00001000 00001000
```

| Control | Action |
|---|---|
| `r` or FIRE1 | restart all four domains, error history and pass numbering; print the marker after the current TX byte |
| `f` | measure all three generated clocks for exactly 25,000,000 board cycles, then print at the next pass boundary |

The frequency counters are registered 28-bit Gray counters sampled through
two-flop synchronisers in the board domain. Subtracting samples a board second
apart gives Hz, with endpoint quantisation of a few counts. The PLL should read
40,000,000 and approximately 33,333,333 on the emulator and the board: these
are ratios to the board clock. OSCG reads nominally 38,750,000 in the emulator,
while its silicon frequency differs from nominal. On 2026-11-17, commit
`9d6db85`, the command-line emulator on rserv printed:

```text
FREQ 02625A00 01FCA056 024F4730
```

That is **40,000,000, 33,333,334 and 38,750,000 Hz**. The PLL outputs are
exact ratios; the 33 1/3 MHz count varies by ±1 with the window's phase because
25,000,000 × 4/3 = 33,333,333.3. OSCG is exactly the nominal 310 MHz / 8.
The run used input of 30 spaces followed by `f`, with 30,000,000 board cycles
taking 232.7 s.

An extra SRAM load on the HC06 board with `f`, outside the byte comparison,
reported **40,000,000, 33,333,333 and 39,005,279 Hz**. Its OSCG base was
**312.04 MHz, +0.66%**, compared with HC06's earlier 311.8–311.97 MHz.
The PLL ratios agree across the emulator and silicon; the OSCG reading
reflects the chip's own frequency.

`f` is ignored while a measurement or its report is pending. Restart cancels
it. No FREQ line appears without the key; frequency is informative and never
part of the byte comparison. Pass reports contain content only, with no
phase-dependent durations, stall counts or arrival-order information.

| LED | Meaning |
|---|---|
| 0 | 25 MHz heartbeat, about 2.98 Hz |
| 1 | 40 MHz heartbeat, about 4.77 Hz |
| 2 | 33 1/3 MHz heartbeat, about 3.97 Hz |
| 3 | OSCG heartbeat, nominally 4.62 Hz |
| 4 | PLL lock |
| 5, 6 | off |
| 7 | an error has been reported since restart; stays lit until restart |

Heartbeat frequency means complete on/off periods. At about one twentieth of
board speed they still change visibly over a minute.

## What the emulator proves

The emulator samples ideally: **there is no metastability, no setup/hold
window and no routing skew between the bits of a bus**. It shows how a
crossing is built and that the data arrives. It cannot show what goes wrong
without one. A binary counter crossed bit by bit through two flops would look
perfect in the emulator and fail on the board. Its possible mixed-bit failure
is described above; there is no unsafe crossing in the checked design.
These runs verify protocols and content. A board run checks them on silicon.

Fast mode schedules four edge trains: the board input, both PLL
outputs, and OSCG. It merges simultaneous events, captures old state together,
and runs a table of the repeating hyperperiod once the trains are periodic.
Generated clocks are not rounded onto board edges. The PLL learns its reference
period, and OSCG uses nominal base/DIV; the gallery checker passes no
`--osc-hz` override. See [FAST_MODE](../../../docs/FAST_MODE.md),
[PLL_EXECUTION](../../../docs/PLL_EXECUTION.md),
[OSC_EXECUTION](../../../docs/OSC_EXECUTION.md) and
[EBR_EXECUTION](../../../docs/EBR_EXECUTION.md).

## Build and verification

```sh
sh examples/gallery/clock_domains/sim/run.sh
examples/ulx3s/build.sh examples/gallery/clock_domains target/gallery/clock_domains
python3 examples/gallery/check.py clock_domains --emulator
```

The simulation substitutes only EHXPLLL and OSCG under `CDC_SIM`; the testbench
drives their edge trains and runs the unchanged synthesised top.
It decodes UART pads, checks every pass against an independent C++ pattern/CRC
oracle, and fails loudly on an error, wrong count, CRC, framing error or missing
FIFO coverage. It runs nominal clocks, changed PLL phase with OSCG +0.6%, and
another phase with OSCG −3%. PLL outputs keep their exact 6:5 ratio throughout.
The bytes from the marker onward must be identical across all three runs.
`run.sh` writes the golden file only after that comparison succeeds.

A separate 28-million-cycle control exercise restarts during UART output and
an active transfer, debounces FIRE1, and checks a complete frequency measurement,
including its exact 25-million-cycle window. `--fst` additionally writes the
first 60,000 board cycles to `target/gallery/clock_domains/sim/synchronisers.fst`.
Generated files and waveforms stay under `target/`.

[check.json](check.json) covers 2,000,000 board cycles: `RESTART` and three pass
lines, 356 bytes. Nominal RTL reports end at cycles 609,641, 1,199,816 and
1,789,991. The steady report interval is about 590,175 board cycles (23.61 ms),
with approximately 340,400 cycles of concurrent computation. These timings
vary slightly with phase and never appear in the checked UART bytes. The
board check's one-second capture leaves ample margin.

Development verification on 2026-11-17: all phase/frequency reruns and the
control exercise passed; a repeated `run.sh` regenerated identical bytes.
No emulator finding occurred; in particular, no legal FIFO access triggered
the conservative EBR row rule. The three checks of commit `9d6db85` are
recorded below.

## Resources and timing

Yosys/nextpnr for LFE5U-85F, CABGA381, seed 1:

- 4,505 total LUT4 equivalents: 3,313 logic, 1,000 carry, 128 RAM and 64 RAMW;
- 1,580 flip-flops and 4,999 packed TRELLIS_COMB;
- one DP16KD, one EHXPLLL, one OSCG, 32 TRELLIS_RAMW for the UART RX FIFO;
- no CPU, DSP or external peripheral model.

| Domain | Constraint | Routed Fmax |
|---|---|---|
| board/UART | 25 MHz | 78.96 MHz |
| CLKOP, FIFO writer/pulse receiver | 40 MHz | 131.27 MHz |
| CLKOS, FIFO reader | 33.333333 MHz | 84.19 MHz |
| OSCG, handshake source | 38.75 MHz nominal | 223.91 MHz |

All domains meet their constraints, with ample OSCG margin for the measured
+0.66%. `pll.v` records the ecppll solution: input divider 5, feedback divider 8,
VCO 600 MHz, output dividers 15 and 18. `top.lpf` constrains all four clocks.

## Window

[gui.script](gui.script) runs for about 60 seconds, types `r`, lets several
passes print, then types `f`. `app.rs` supports `watch NAME` and `waves
open|close`, but this build's internal synchroniser names are absorbed by
synthesis. The script leaves waves closed; the optional RTL FST retains the
useful synchroniser signals. At the measured window rate of about 0.13 MHz,
the FREQ window of 25 million board cycles takes about three minutes, longer
than the script. The run below did not reach FREQ; the emulator's frequency
reading above was measured from the command line.

## Checks

All three checks below ran on **2026-11-17, commit 9d6db85**. Records are in
[HR03](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).

**Emulator:** the earlier `--build --emulator` check used rserv's own build.
On 2026-11-17, `check.py all --emulator` on rserv, without
rebuilding, passed the **board's Mac-built configuration**: the RTL simulation's
**356 bytes from RESTART** matched. Fast mode ran **2,400,000 cycles in 17.444 s,
0.14 MHz**. Verified configuration hashes and PASS/rate lines are in
[emulator_board_configs.log](../../../docs/inventory/LFE5U-85F/hr03_gallery/emulator_board_configs.log).
The separate command-line FREQ measurement is recorded with the controls above.

**Board:** **3 of 3 SRAM loads passed**, at 15:57:49Z, 15:57:57Z and 15:58:05Z,
each matching all 356 expected bytes from RESTART. The three transcripts are
identical over **4,979 bytes from RESTART**: 42 passes (43 in two of the loads),
with **zero errors**. Every pass line equals the RTL simulation's line for
the same pass number. The loaded `top.bit` SHA256 is
`6beb96be32d0c8af5b4b83df84552a0ed7978bedb1b4fb1503b879ede81c1c81`.
The extra `f` load, outside the byte comparison, measured the board's OSCG
at +0.66% as described above.

**Window:** `gui_run.py clock_domains` ran [gui.script](gui.script) for 60 s,
with the display awake and unlocked at the start and end:

- 3,588 window frames; median 16.7 ms, 95th percentile 17.2 ms;
- 6,966,496 cycles (0.279 s of board time), **0.13 MHz** shown;
- 9 LED drawings read back, **none wrong**, with 9 distinct patterns showing
  the heartbeats and PLL lock;
- the console **passed on 356 bytes from RESTART**; all 11 passes received
  equal the RTL simulation's lines;
- FREQ was not reached: its 25-million-cycle measurement window outlasts
  the script at this rate.
