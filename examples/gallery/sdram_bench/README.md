# sdram_bench: why SDRAM bursts matter

A plain-RTL controller writes and checks the ULX3S's MT48LC16M16A2: four banks ×
8192 rows × 512 halfwords, or 32 MiB. No CPU, PLL, cache, or on-chip frame buffer.
One 2 MiB pass runs at power-on, followed by 32 MiB passes forever. Each pass
writes the region, then reads and compares every halfword.

The lesson is amortising command overhead. During a burst this controller moves
one 16-bit halfword every 25 MHz clock. The board, RTL simulation and emulator
on the command line and in the window agree on **44.5 MiB/s writing and
44.8 MiB/s reading**, including ACTIVE, PRECHARGE and refresh.
That is **93–94 % of the 47.7 MiB/s** a 16-bit bus
moves at 25 MHz. The single-halfword controller in
[sdram_test](../../ulx3s/sdram_test/controller.v), including its caller's request
handshake, takes about ten clocks per halfword: at most **4.77 MiB/s** before
refresh stalls. This is an estimate from that state machine, not a new board
measurement; bursts give about 9.4 times that rate.

This is an **original burst controller**, not a reuse of `sdram_test`'s
controller. Only its board pin constraints and the gallery UART/button plumbing
were copied. The SDRAM constraints, including **PULLMODE=UP on every DQ pin**,
match that working reference exactly.

## Controller and timing

[controller.v](controller.v) uses the 25 MHz board clock throughout. It drives
`sdram_clk = ~clk`; command/address/data outputs settle between the board's
rising edge and the SDRAM's rising edge. DQM is always zero. Address units are
halfwords: `{bank[1:0], row[12:0], column[8:0]}`. Thus the 2 MiB region covers
bank 0, rows 0–2047; 32 MiB covers all four banks and all rows.

Initialisation waits 2502 clocks after the 63-clock power-on reset, then issues
PRECHARGE ALL, two AUTO REFRESH commands and LOAD MODE, with the required waits.
Mode register `0x027` selects **full-page sequential bursts, CL2, burst writes**.

A whole 512-word page would occupy 20.48 µs and miss the refresh deadline. Instead,
each burst contains **128 words**. It starts at column 0, 128, 256 or 384, never
crosses a row, and ends with BURST TERMINATE. Each chunk closes its bank and
performs a refresh before opening the next chunk:

```text
REFRESH → RFC wait → ACTIVE → RCD wait → WRITE 128 beats → TERMINATE
        → write recovery → PRECHARGE → RP wait → next chunk
REFRESH → RFC wait → ACTIVE → RCD wait → READ → capture 128 beats
        (TERMINATE alongside the last pipelined beat) → PRECHARGE → RP wait → next chunk
```

READ data uses the CL2 pipeline: READ at SDRAM edge n, DQ published after n+1,
and captured on the following board rising edge. BURST TERMINATE at n+128
preserves the last word of the read pipeline. The independent model implements
this pin/capture convention described in [PERIPHERALS_SDRAM](../../../docs/PERIPHERALS_SDRAM.md).

At 40 ns/clock, all datasheet/model minimums round to two clocks:

| Rule | Minimum | Controller spacing |
|---|---:|---|
| Init | 100 µs | >100 µs of NOP before PRECHARGE ALL |
| tRP | 2 clocks | 2 in init; 3 from chunk PRECHARGE to REFRESH |
| tRCD | 2 | ACTIVE to READ/WRITE: 2 |
| tRC | 2 | successive ACTIVE in one bank: at least 136 |
| tRAS | 2 | ACTIVE to PRECHARGE: 131 read / 132 write |
| tWR | 2 | final write data to PRECHARGE: 3 |
| tRFC | 2 | REFRESH to next command: at least 2 |
| tMRD | 2 | LOAD MODE to next command: at least 2 |
| Refresh | ≤7.8 µs; 8192 per rolling 64 ms | 136/137 clocks while streaming; ≤155 while idle/starting |

Refresh is part of the schedule, rather than a request that a burst can starve.
Idle and UART printing also refresh the SDRAM. Each phase starts with its own
refresh barrier, making its cycle count independent of UART arrival time and
idle refresh position. `r` resets the benchmark state and seed; the physical
SDRAM stays initialised and refreshing throughout the restart marker.

A write chunk takes 137 clocks, a read chunk 136. The counters include all chunk
commands, waits, refreshes and final bank closure; they exclude initialisation,
UART reporting and the gap between phases. There are 8192 chunks in 2 MiB.

## Pattern, output and controls

The pattern is `addr[15:0] XOR {addr[7:0],addr[23:16]} XOR seed`, so bank, row,
column and pass all influence the data. A nonzero 16-bit maximal LFSR, seeded
with `0xace1`, advances once per pass. Adjacent seeds always differ; a stale
preceding pass cannot pass the comparison. The sequence repeats after 65535
passes. Errors are counted per halfword; the first mismatch latches its address,
expected value and received value. LED 7 stays lit after any error until `r`.

UART is 115200 baud, 8N1. All numbers are decimal, including error addresses and
halfword values. `SIZE` and the bandwidth fields use **2^20-byte MB (MiB)**.
Bandwidth is `bytes × 25e6 / clocks / 2^20`, truncated to one decimal place:

```text
RESTART
PASS 1 SIZE 2 WRITE 1122304 44.5 READ 1114112 44.8 ERRORS 0
```

On a mismatch, another line follows, for example the simulation's injected fault:

```text
FIRST ERROR 123 55194 55195
```

The address above is a halfword address; multiply by two for its byte address.
The seven low LEDs form a progress bar, one LED per completed eighth of the
current write/read phase. The bar starts again for the read phase.

| Button | UART | Action at the next pass boundary |
|---|---|---|
| FIRE1 | `r` | exact power-on benchmark state: pass 1, seed, counters, sticky error and size defaults; prints `RESTART` |
| UP | `f` | 32 MiB passes from now on |
| DOWN | `s` | 2 MiB passes from now on |

UART bytes and debounced button commands share a 256-entry FIFO and act in order
at pass boundaries. Restart preserves queued commands and finishes any current
TX byte before its marker. Simultaneous events use UART, FIRE1, UP, DOWN order.
Buttons debounce for 10 ms; held buttons produce one event. A full FIFO drops new
UART bytes rather than overwriting queued ones. Other keys are ignored.

## Resources

Mac build, Yosys 0.70+post, nextpnr LFE5U-85F/CABGA381, seed 1:

- 1276 logic LUT4; 1794 total LUT4 including carry and distributed-memory LUTs.
- 718 TRELLIS_FF, 32 TRELLIS_RAMW (128 RAM LUTs), 57 PIO.
- 1900 occupied TRELLIS_COMB sites (2% of the device).
- No EBR, DSP, or PLL. The receive FIFO uses distributed RAM.
- Routed Fmax **86.84 MHz**, passing the 25 MHz constraint (placement estimate 67.61 MHz).

The Mac bitstream rebuilds identically. `top.bit` SHA-256:
`3bd5038b40c85393c52d387187c5ca47d58950323f9014cbacd52c14331fb1bd`.

[ecp5.toml](ecp5.toml) lists all four synthesised Verilog files and the constraints.
The wrapper and behavioural model under `sim/` are simulation-only.

## Build and check

```sh
sh examples/gallery/sdram_bench/sim/run.sh
python3 examples/gallery/sdram_bench/sim/checks.py
examples/ulx3s/build.sh examples/gallery/sdram_bench target/gallery/sdram_bench
```

[sim/run.sh](sim/run.sh) compiles Verilator directly with quoted paths, so it works
in a workspace containing spaces. It regenerates [expected/uart.txt](expected/uart.txt)
from the actual UART pins, starting at the restart marker. It also runs negative
tests proving that the independent model rejects init and every timing rule,
refresh starvation, an open-bank violation and reserved mode settings.

The independent C++ model [sim/sdram_model.h](sim/sdram_model.h) samples rising
SDRAM clock edges, stores a full 32 MiB array, handles sequential full-page/BL8
read/write bursts and BURST TERMINATE/PRECHARGE, and checks the timing table.
Auto precharge is unused. Alongside the SDRAM model, the testbench audits ordered
physical addresses, complete region coverage, pass-dependent write patterns,
UART framing and error counts. [sim/checks.py](sim/checks.py) also checks delayed
restart determinism, repeated small passes, queued restarts and size commands,
an injected read error with its sticky LED, and debounced DOWN/FIRE1 controls.
These checks leave their transcripts under `target/gallery/sdram_bench/sim/`.

[check.json](check.json) sends `r`, uses marker `RESTART`, and allows 6000000
clocks (0.24 s). This covers the initial 2 MiB pass, a restart, and its complete
2 MiB report, while the following 32 MiB pass is still in progress. Only the
marker and the post-restart 2 MiB line belong to the expected transcript.
Simulation reruns regenerate it byte-identically. In 6 M clocks the independent
model saw **3 261 161 writes, 2 097 152 reads and 43 698 refreshes**, with no timing
violation.

The full-size board pass takes **17956864 write clocks and
17825792 read clocks** for 32 MiB: **1.43130624 s** per complete write/check pass,
plus roughly 6 ms of reporting. `board_seconds=3.3` allows 0.24 + 1.4313 + 1.5 s,
rounded up; every board capture also includes the first full-size pass line,
outside the expected prefix. Both phase counts are exactly 16 times the 2 MiB
pass's clocks.

## On the board

**Board:** on 2026-11-17 at 07:54–07:55 UTC, **3 of 3 loads** printed the expected
68 bytes from `RESTART` on. Every load also wrote and checked the whole 32 MiB
with no error:

```text
PASS 2 SIZE 32 WRITE 17956864 44.5 READ 17825792 44.8 ERRORS 0
```

The log is `target/gallery/sdram_bench/board_runs.log`; the transcripts are
`target/gallery/sdram_bench/board_uart_1.txt` through `board_uart_3.txt`.
The stored configuration, first-load transcript and run log are in
[HR03](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).

## In the command-line emulator

**Emulator:** `python3 examples/gallery/check.py sdram_bench --emulator` now
passes from the command line using `emulate --fast --peripherals`. All nine
gallery demos passed on the remote machine, with sdram_bench included rather
than skipped. Its result was **PASS: 68 bytes from the marker** (`RESTART`),
in **6.8 M clocks at 0.65 MHz** on the remote machine. The final stderr report was:

```text
peripheral: sdram init=yes reads=2097152 writes=4008601 timing_violations=0 unknown_samples=0 refresh_shortfalls=0
```

The UART bytes match the RTL simulation. The gallery checker also requires
the SDRAM report to confirm initialisation and zero timing violations, unknown
samples and refresh shortfalls.

## In the emulator's window

The window check ran with the live runner's attached SDRAM peripheral model.
Its pass lines and cycle counts equal the board's and the RTL simulation's own
SDRAM model.

[gui.script](gui.script) is a 60-second window run: run at maximum speed, show the
3D board, then hold UP for one second after the initial small pass to request
full-size passes. At normal hardware speed the small pass takes about 0.095 s;
the window's wall-clock schedule allows 12 seconds before UP.

The window's record of one run:

- 60 s at 60 frames per second;
- 1.1 MHz median, 1.66 MHz maximum, 80.5 million clocks;
- the same three pass lines as the board: one 2 MiB pass and two 32 MiB passes;
- 53 drawings of the 3D board read back, none with an LED drawn wrong;
- all 8 levels of the progress bar shown; the error LED was never lit;
- the UP press is recorded.

The console printed:

```text
PASS 1 SIZE 2 WRITE 1122304 44.5 READ 1114112 44.8 ERRORS 0
PASS 2 SIZE 32 WRITE 17956864 44.5 READ 17825792 44.8 ERRORS 0
PASS 3 SIZE 32 WRITE 17956864 44.5 READ 17825792 44.8 ERRORS 0
```
