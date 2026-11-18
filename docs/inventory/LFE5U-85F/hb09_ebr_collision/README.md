# HB09 slice: DP16KD collisions on one clock edge (B05, B06)

What a DP16KD outputs, and what its array stores, when both ports access the same word
on one clock edge. The SERV stock register file stopped fast mode at
cycle 32 on such a collision, while the same bitstream ran correctly on the board.
The emulator previously made cross-port reads and double writes in one INIT9 row X.
HB09 resolves the equal-width, equal-WRITEMODE, full-word subset of
[B05/B06](../../../EBR_ADDRESSING.md).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381.
  Loaded over JTAG into SRAM with `openFPGALoader -b ulx3s`, under `out/board.lock`;
  the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** written by [hb09_gen.py](hb09_gen.py) with [hb09.lpf](hb09.lpf):
  - **hb09** ([hb09.v](hb09.v), [hb09.config](hb09.config)): free-running traffic;
    after a line is sent, the next 128-sample window starts on the next clock.
  - **hb09s** ([hb09s.v](hb09s.v), [hb09s.config](hb09s.config)): records the first
    128-sample window from configuration, when the array equals `INITVAL`, and resends
    it while traffic keeps running.
  - **Memory.** Eight DP16KD on the same 25 MHz clock, with `CEA = CEB = 1`,
    `OCEA = OCEB = 1`, `RSTA = RSTB = 0`, `CS = 0`, `CSDECODE = 0b000`,
    `GSR = DISABLED`, `RESETMODE = SYNC`, and fixed pseudo-random `INITVAL_00..3F`:

    | EBR | DATA_WIDTH_A | DATA_WIDTH_B | WRITEMODE_A | WRITEMODE_B | REGMODE |
    |---|---|---|---|---|---|
    | e0 | 2 | 2 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e1 | 1 | 1 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e2 | 4 | 4 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e3 | 9 | 9 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e4 | 18 | 18 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e5 | 9 | 9 | NORMAL | NORMAL | NOREG |
    | e6 | 9 | 9 | WRITETHROUGH | WRITETHROUGH | NOREG |
    | e7 | 9 | 9 | READBEFOREWRITE | READBEFOREWRITE | OUTREG |

  - **Traffic.** A 64-bit maximal-length LFSR (`x^64 + x^63 + x^61 + x^60 + 1`),
    stepped 32 times per clock from `0x1D2CACE15EED0001`, drives both ports:
    `w_a = lfsr[1:0]`, `we_a = lfsr[4]`, `d_a = lfsr[23:6]`;
    `w_b = lfsr[3:2]`, `we_b = lfsr[5]`, `d_b = lfsr[41:24]`.
    Data is truncated to each width. Both ports address the same four words; each
    writes with probability 1/2. Word addresses occupy `AD[shift+1:shift]`.
    At width 18, `AD0 = AD1 = 1` enables both nine-bit lanes throughout.
  - **Line.** `R<16 hex win_lfsr><3968 hex>` (`128 * 31` hex), 115200 8N1.
    Each sample packs the 122 DOA/DOB bits of e0..e7, A then B, MSB first, followed
    by two zero pad bits. `win_lfsr` is the state at the first edge of the window.
    Sample `k` is captured before edge `k`; sample `k + 1` reflects edge `k` for
    NOREG, and sample `k + 2` reflects edge `k` for OUTREG.
- **Runner:** [run.py](run.py) `hb09.bit R 3 30` and `hb09s.bit R 3 8`.
- **Analysis:** [hb09_analyse.py](hb09_analyse.py) separates clean reads/writes,
  read/write collisions in each direction, and double writes, comparing old, new
  and own-mode outputs. Later clean reads classify the array after double writes.
  Reports: [analysis.txt](analysis.txt), [analysis_s.txt](analysis_s.txt).

## Observations

- **hb09** ([observations.log](observations.log)): 3 loads × 30 s on 2026-11-17,
  02:32–02:34Z. The loads recorded 87, 87 and 86 lines, including 86, 86 and 85 full
  windows; 107 distinct full windows in total. Every load agrees with the rules below.
- **hb09s** ([observations_s.log](observations_s.log)): 3 loads × 8 s on 2026-11-17,
  02:35–02:36Z. Each recorded 23 lines, with the same single full window in all three loads.

## Reading

1. **A reading port gets the old word against an RBW writer.** There are zero exceptions
   at widths 1/2/4/9/18, NOREG and OUTREG: 758–827 known read/write collisions per port
   for NOREG, 748–817 for OUTREG in the free-running record.
2. **A reading port gets the new word against a NORMAL or WRITETHROUGH writer.**
   e5/e6 have zero exceptions in 827 A-side and 758 B-side read/write collisions.
   Both ports have the same mode in every block, so this does not separate whether
   the reader's or writer's mode decides.
3. **Each writing port outputs per its own WRITEMODE**, whether the other port reads
   or writes: RBW outputs the pre-write word, NORMAL holds its output, and
   WRITETHROUGH outputs its own input data. Zero exceptions; 752 double writes per
   NOREG port, 749 for OUTREG.
4. **At widths 1/2/4/9, B's data remains after a same-word double write.** Each block
   has 381 later clean reads of such words. A never wins; cases where both data words
   were equal are classified as "same": e0 B=305/same=76, e1 B=195/same=186,
   e2 B=364/same=17, and e3/e5/e6/e7 B=381/same=0.
5. **Width-18 double writes are not deterministic.** Of 381 later clean reads, 354
   match B and 27 mix bits within the low nine-bit lane, although both lanes were
   enabled. There is no deterministic array rule for this case.
6. **Different equal-width narrow words within one INIT9 row are independent.**
   The clean cases at widths 1/2/4 include different words in the same row; all four
   of e0's two-bit words share one row. [analysis.txt](analysis.txt) shows zero
   mismatches in every clean case: 5 550–5 638 known reads and 4 863–4 932 known writes
   per port. A row-wide collision domain is too wide for these equal-width accesses.
7. **The first window from configuration agrees.** `hb09s` starts from known
   `INITVAL` and confirms these read and write outputs with zero exceptions. Its later
   clean reads after double writes all match B, including its three width-18 reads;
   that short window does not remove the width-18 mixes seen in the longer run.

## Not covered

- Mixed widths or WRITEMODEs between the ports (the remaining B05/B06 scope).
- Partial byte masks, including a disabled lane at width 18.
- Different clocks, phases, or a physical timing window around an edge.
- CE/OCE gating, non-matching CS, reset/GSR or write inhibit during collisions.
- PDP collisions.
- A deterministic width-18 write/write array result; the observed mixes prevent one.

## Emulator check

The event kernel in [src/kernel/ebr.rs](../../../../src/kernel/ebr.rs) uses word-level
overlap only for equal-width/mode full-word DP accesses on one definite edge of the same
clock and polarity, with known addresses/WE, active CE/CS/OCE and inactive resets/GWDIS.
Reads take old or new data as above; writers retain their own mode's output; B wins
double writes at widths 1/2/4/9. Different narrow words in a row remain independent.
Width-18 double writes still make both array rows X. Everything outside this measured
scope retains the previous conservative policy ([EBR_EXECUTION.md](../../../EBR_EXECUTION.md)).

[tests/ebr_collision_native.rs](../../../../tests/ebr_collision_native.rs) loads
`hb09s.config` and probes every DOA/DOB bit by wire before each of the 128 clocks.
It compares known bits with the board and independently tracks the exact X domain:
only e4's outputs reading words left unknown by earlier double writes. It also counts
same-word collisions and disjoint narrow words sharing a row, and checks that outputs
made X by the former policy are now known.

The replay passed on rserv: **15 310 known bits matched**, **306 X bits** were exactly
the expected e4 domain, and **2 086 directly witnessed former X bits** became known.
Each NOREG block covers 10 A-read/B-write, 6 B-read/A-write and 9 double-write edges;
OUTREG covers 9, 6 and 9 within its shorter visible range. At widths 2/1/4 it also
covers 69/69/20 cycles with different words in one INIT9 row. Runtime: 3.52 s.

The portable EBR suite also passes: 16 tests, 0 failures, 0.14 s. It checks the
measured full-word rule and keeps mixed widths/modes, partial masks and separate
clocks conservative. Two old rejection assertions were updated to accept narrow
RBW, as already established by HB11. All 219 ordinary Rust tests and 15 catalogue
checks pass on rserv; `cargo fmt --check` and release clippy with `-D warnings` pass.

The existing native EBR regressions also passed on rserv with `--ignored`:

- **ebr_native:** 3/3, 30.38 s.
- **ebr_start_native:** 2/2, 220.79 s.
- **ebr_narrow_rbw_native:** 3/3, 1 616.89 s, including the full HB11 event-kernel
  UART replay and both fast-mode checks.

Fast mode was inspected but not changed. `ebr_program` in
[src/kernel/fast.rs](../../../../src/kernel/fast.rs) currently sets a read to 2 (X)
at line 3119 when the other port writes its INIT9 row, and poisons the whole row
after two writes at line 3171. Lines 3175–3193 check the captured output for X.
The row read also feeds a writing RBW port, so it too captures X on a collision;
NORMAL still holds and WRITETHROUGH still outputs DI, but a later read sees the
poisoned array after a double write.
It stops with error 2 when that X reaches an output read by logic; OUTREG delays this
by one edge, and a probe alone does not stop it. It needs the same HB09 guard and
word-level overlap as the event kernel, old/new forwarding only on the reading path,
and B's final array value without row poisoning at widths 1/2/4/9. The kernel caches
clock-source/polarity equality through plain routing buffers in `Block::same_clock`;
the generator can use `block.same_clock == Some(true)` for that part of its guard.
Width-18 double writes must keep array X while both output paths use their own
mode's result.
The existing failure checks and all unmeasured cases must remain conservative.

On rserv, the current fast mode completes `hb09s.config` with `--cycles 1`, but
`--cycles 2` fails in e0 (`LFE5U-85F/R46C10/EBR3`): the second rising edge is at
60 ns with a 40 ns period. The batched CLI error says "0 ps", the start of the
failed batch, rather than the edge that caused it. This is separate from the
SERV stop at cycle 32. The reproducer uses `--startup configured
--scope observed --clock pin:G2 --period-ns 40 --probe pin:L4 --fast`.

Rerun on rserv:

```sh
cargo test --release --test ebr_collision_native -- --ignored --nocapture
```
