# HB11 slice: DP16KD READBEFOREWRITE at narrow widths 1, 2 and 4 (B07)

What a DP16KD outputs on a write with `WRITEMODE=READBEFOREWRITE` at the narrow widths 1, 2
and 4. The Lattice Memory User Guide allows READBEFOREWRITE only at widths 9 and 18, and the
emulator refused narrow READBEFOREWRITE ("narrow READBEFOREWRITE is outside documented
support"). The SERV RISC-V core's register file `serv_rf_ram`
(576×2) maps through Yosys 0.70 to a DP16KD with `DATA_WIDTH` 2 and READBEFOREWRITE on both
ports. This closes question B07 for these widths ([EBR_ADDRESSING.md](../../../EBR_ADDRESSING.md)).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`, under `out/board.lock`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** written by [hb11_gen.py](hb11_gen.py) with [hb11.lpf](hb11.lpf):
  - **hb11** ([hb11.v](hb11.v), [hb11.config](hb11.config)): free-running capture; after each
    line is sent, the next 128-sample window starts on the next clock while traffic keeps
    running.
  - **hb11s** ([hb11s.v](hb11s.v), [hb11s.config](hb11s.config)): records only the first
    128-sample window starting at clock 0 after configuration (when memory equals `INITVAL`)
    and resends that same window while traffic keeps running.
  - **Memory.** Eight DP16KD blocks on the 25 MHz clock with `CEA = CEB = 1`,
    `OCEA = OCEB = 1`, `RSTA = RSTB = 0`, `CS = 0`, `CSDECODE = 0b000`, `GSR = DISABLED`,
    `RESETMODE = SYNC`, and fixed pseudo-random `INITVAL_00..3F` patterns per block:

    | EBR | DATA_WIDTH_A | DATA_WIDTH_B | WRITEMODE_A | WRITEMODE_B | REGMODE |
    |---|---|---|---|---|---|
    | e0 | 1 | 1 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e1 | 2 | 2 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e2 | 4 | 4 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e3 | 2 | 2 | READBEFOREWRITE | READBEFOREWRITE | OUTREG |
    | e4 | 1 | 4 | READBEFOREWRITE | READBEFOREWRITE | NOREG |
    | e5 | 4 | 4 | READBEFOREWRITE | READBEFOREWRITE | OUTREG |
    | e6 | 2 | 2 | NORMAL | WRITETHROUGH | NOREG |
    | e7 | 9 | 9 | READBEFOREWRITE | READBEFOREWRITE | NOREG |

  - **Traffic.** A 64-bit maximal-length LFSR (`x^64 + x^63 + x^61 + x^60 + 1`) stepped 32
    times per clock from `0x1D2CACE15EED0001` drives both ports (`w_a = lfsr[3:0]`,
    `d_a = lfsr[12:4]`, `we_a = lfsr[13]`; `w_b = lfsr[17:14]`, `d_b = lfsr[26:18]`,
    `we_b = lfsr[27]`), with data truncated to each port's width. Port A sets word-address
    MSB `AD13 = 0` and port B sets `AD13 = 1` with the low four word-address bits carrying `w`
    and all other `AD` bits 0, so the two ports access disjoint halves of each block (`INIT`
    words `0..15` vs `1024..1039`).
  - **Line.** `R<16 hex win_lfsr><1792 hex>` (`128 * 14` hex) at 115200 8N1: `win_lfsr` is the
    LFSR state at clock 0 of the window, followed by 128 samples of 56 bits (53 output bits of
    `e0..e7` ports A and B plus 3 pad zero bits), captured before each rising edge. Sample
    `k + 1` reflects edge `k` for `NOREG` and sample `k + 2` reflects edge `k` for `OUTREG`.
- **Runner:** [run.py](run.py) `hb11.bit R 3 25` and `hb11s.bit R 3 8`.
- **Analysis:** [hb11_analyse.py](hb11_analyse.py) scores every port under the `RBW`, `NORMAL`
  and `WRITETHROUGH` write-cycle models; output in [analysis.txt](analysis.txt) and
  [analysis_s.txt](analysis_s.txt).

## Observations

- **hb11** ([observations.log](observations.log)): 3 loads × 25 s on 2026-11-16, 14:57–14:59Z.
  Each load recorded 159 lines and 158 full windows, the same 158 windows in all three loads.
- **hb11s** ([observations_s.log](observations_s.log)): 3 loads × 8 s on 2026-11-16,
  15:00–15:02Z. Each load recorded 51 lines, all the same single full window.

## Reading

1. **Every narrow READBEFOREWRITE port outputs the old word at the written address on a
   write**, with 0 mismatching bits across widths 1, 2 and 4, `NOREG` and `OUTREG`, and the
   mixed `1`/`4` pair (`e4`). Known write cycles per port are 8 680–8 774 in `hb11` and
   69–70 in `hb11s`.
2. **The `NORMAL` and `WRITETHROUGH` models miss on about half the bits** of those writes, so
   the traffic tells the write modes apart.
3. **Reads match in every known cycle**, in every EBR (0 mismatching bits across 8 688–8 784
   known reads per port in `hb11` and 56–57 in `hb11s`).
4. **The controls fit their documented modes:** `e6` port A matches `NORMAL`, `e6` port B
   matches `WRITETHROUGH`, and `e7` matches `READBEFOREWRITE` at width 9 (0 write mismatches
   in their configured modes).
5. **`hb11s` confirms the narrow `INIT` mapping:** its window starts from configuration, and
   its reads and writes match `INITVAL`, confirming that widths 1, 2 and 4 use the low 8 bits
   of each 9-bit `INIT` word.
6. **Narrow READBEFOREWRITE behaves as READBEFOREWRITE at widths 9 and 18.** The Memory User
   Guide restriction does not reflect the chip.

**Not covered:**
- collisions between the two ports (HB09/HB10);
- `CE` and `OCE` gating (`CE` and `OCE` were 1 throughout);
- resets and `GSR` during READBEFOREWRITE;
- READBEFOREWRITE in PDP;
- widths 1, 2 or 4 paired with 9 or 18 on one block.

## Emulator check

`src/kernel/ebr.rs` no longer refuses narrow READBEFOREWRITE
([EBR_EXECUTION.md](../../../EBR_EXECUTION.md)).
[tests/ebr_narrow_rbw_native.rs](../../../../tests/ebr_narrow_rbw_native.rs) ran on `rserv` and
all three replay checks passed (1 766 s total across the three tests; the event-kernel test
is the slow one):
- **Event kernel, [hb11s.config](hb11s.config):** passed (prints `hb11s`'s board line).
- **Fast mode, [hb11s.config](hb11s.config):** passed (prints `hb11s`'s board line).
- **Fast mode, [hb11.config](hb11.config):** passed (window 0 matches `hb11s`'s line, and its
  first free-running windows are among `hb11`'s board lines).

The existing EBR tests still pass: `ebr_start_native` 2/2, `ebr_native` 3/3.
