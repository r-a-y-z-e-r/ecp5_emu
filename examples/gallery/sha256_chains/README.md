# sha256_chains: parallel hash chains in plain RTL

Ten SHA-256 datapaths advance independent chains in lockstep. A link consumes
the preceding 32-byte digest and produces another 32-byte digest. The circuit
prints periodic values and computes a final digest over all cores' values in
core order. There is no CPU or instruction stream: rotations are wires,
Boolean functions use LUTs, and 32-bit additions use carry chains.

This demonstrates parallel scaling, area per core, serial dependencies within
a chain, and comparison against both a standard and independent software.
The design occupies about 39% of an LFE5U-85F's LUTs as a sizeable fast-mode
workload. All RTL and test code are original; nothing was downloaded.

## Definition and byte order

For core k and pass p:

```text
h(0)   = SHA-256(ASCII("Parallel chain seed / ") || BE32(k) || BE32(p))
h(i+1) = SHA-256(h(i))
```

The original seed text is **22 bytes**, including its trailing space, and has
no names. The two binary fields are 32-bit big endian, giving a 30-byte seed
message. Padding is `80`, 25 zero bytes, then the 64-bit big-endian bit length
240. Each later link hashes the **raw 32 digest bytes**, not their hexadecimal
representation: digest, `80`, 23 zero bytes, then bit length 256.

`CORES=10`, `STEP_EVERY=1024`, and `STEPS=3072` are the defaults. A pass computes
h(0) through h(3072) on every core: 3,073 hashes per core and 30,730 in parallel
over the pass. Every 1,024 transitions, STEP prints the current value on all
cores in order 00 through 09. There are three complete STEP rounds per pass.
STEPS must be a positive multiple of STEP_EVERY; CORES supports 1 through 16.

After the final STEP round, the CHAIN digest is:

```text
SHA-256(h_core0(3072) || h_core1(3072) || ... || h_core9(3072))
```

The 320 raw bytes are processed by **core 0**, using five 64-byte data blocks
and a sixth padding block with bit length 2,560. Other cores hold their final
values. Core 0 consumes its own value in the first block before overwriting
its output with the compression result. Subsequent blocks use only other
cores' held values. This avoids a duplicate register array or EBR.
Odd core counts put the last 32-byte value and padding in one final block.

The pass number then increments and all cores begin new seeds. STEP and CHAIN
fields are uppercase hex: steps and passes use 8 digits, core indices 2 digits,
and CYCLES 16 digits. CHAIN's STEPS is the number of transitions per chain.
CYCLES counts all board clocks since restart, including KATs and UART waits,
and is captured before transmitting its CHAIN line.

## Compression structure and timing

[sha256_core.v](sha256_core.v) follows
[FIPS 180-4](https://nvlpubs.nist.gov/nistpubs/FIPS/NIST.FIPS.180-4.pdf), sections
4.1.2, 4.2.2 and 6.2.2. It retains the same iterative core and its comments:
eight working words a–h, an IV copy for feed-forward, a 16-word sliding
message schedule, and a 6-bit round counter.

Σ0 and Σ1 XOR rotations; σ0 and σ1 also use logical shifts. Ch selects between
two words, Maj takes the bitwise majority, and K[t] is a combinational constant
case. Each round updates a–h and appends one schedule word. All additions wrap
to 32 bits. The schedule, working words and output live in flip-flops.

A compression uses **66 rising edges**: one load, 64 rounds, and feed-forward.
Done appears 65 intervals after the accepted load edge. The controller permits
an ordinary **67-clock start-to-start interval**. Independent chains therefore
sustain **cores × clock / 67** before reporting. The core-only upper bound is
cores × clock / 66. The simulation checks both the latency and controller period.

Each core's next input depends on its own previous output, so independent
cores supply the parallelism. At a reporting boundary every core waits while
all lines drain through UART. Every value is retained; no report is omitted.
During the final digest only core 0 runs, chaining each block's output into
the next block's IV.

There is one 25 MHz clock on G2, no PLL, and a six-bit power-on counter that
holds reset for 63 rising edges. UART RX and buttons use two synchronizer
stages in that domain. Configuration initializes registers and the RX FIFO.

## Known-answer tests

Power-on, UART `r` and FIRE1 initialize pass 0. Before any chain begins,
**every core** hashes the public standard examples `abc` and the 448-bit message
`abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq`.
The latter needs two padded blocks; the first result supplies the second IV.
Power-on begins directly with the KATs, without a RESTART marker. Only UART
`r` or FIRE1 prints that marker. No intermediate value is precomputed. These
use the same compression hardware as the chains and final digest. The output shows core 0's digest; OK requires
all cores to match the standard. Any mismatch prints FAIL and prevents chains
from starting after both tests have been reported.

```text
KAT abc BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD OK
KAT 448 248D6A61D20638B8E5C026930C3E6039A33CE45964FF2167F6ECEDD419DB06C1 OK
```

Python `hashlib` independently computes both expected digests as well as all
chain values and the final concatenation digest.

## Resources

Final build for LFE5U-85F CABGA381, Yosys/nextpnr-ecp5, seed 1, on this Mac:

- **32,460 total LUTs (38.8%)**: 27,992 logic, 4,276 carry, 128 RAM, 64 RAMW;
- **13,478 flip-flops**, **33,130 packed TRELLIS_COMB (39.6%)**;
- **Fmax 44.59 MHz**, passing the 25 MHz constraint;
- **3,246 LUTs per core**, including shared control and IO;
- 32 TRELLIS_RAMW cells for the 256-byte distributed RX FIFO;
- 18 IO pins and one DCCA clock buffer;
- no DSP, EBR, PLL, CPU or IOLOGIC.

[ecp5.toml](ecp5.toml) lists every synthesised `.v` file and [top.lpf](top.lpf).
The small RX FIFO uses LUT RAM; the hash datapaths use LUTs, carry chains and
flip-flops throughout.

**Separate sizing builds** measure the complete chain workload at three core
counts. LUT totals include logic, carry, distributed RAM and RAMW LUTs; the
part has 83,640 LUTs. Per-core costs include the shared controller and IO.
[sim/scaling.sh](sim/scaling.sh) reproduces the parameter builds.

| Cores | Total LUTs | LUTs/core including control | Routed Fmax | Hashes/s at 25 MHz before reports |
|---:|---:|---:|---:|---:|
| 1 | 4,344 (5.2%) | 4,344 | 66.01 MHz | 373,134 |
| 4 | 14,281 (17.1%) | 3,570 | 53.97 MHz | 1,492,537 |
| 10 | 32,581 (39.0%) | 3,258 | 45.34 MHz | 3,731,343 |

In these sizing builds, adding nine cores costs an average **3,137 LUTs per
additional core**. Ten cores keep the final design in the 25–40% LUT range.
The rate before reporting is cores × 25 MHz / 67.

UART output and the serial final digest reduce end-to-end throughput. Each
pass sends 30 STEP lines (83 bytes each) and one CHAIN line (127 bytes):
2,617 bytes at 2,172 board clocks per byte, including its handshake. That is
about 5.68 million clocks of reporting versus about 206,000 clocks advancing
chains. Including six final compressions and small control overheads gives
about **0.236 s/pass**, roughly **130,000 chain hashes/s** at 25 MHz with all
reports enabled.

## Output and controls

An excerpt after `r` from [expected/uart.txt](expected/uart.txt), with CRLF line endings:

```text
RESTART
KAT abc BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD OK
KAT 448 248D6A61D20638B8E5C026930C3E6039A33CE45964FF2167F6ECEDD419DB06C1 OK
STEP 00000400 00 66FC7FBE311297A73DE0764DDA71316722DCCE14C3812C17D9810B0181C108C7
STEP 00000400 01 D1D968DDB0FC77149433B2016ACC4A334AB0670356DB1B52EDB27DCE148593B1
...
CHAIN 00000000 STEPS 00000C00 DIGEST 8A1009FF581A93FA302F26D70BC36C2D2355F667F78DD4A0A5B69CD654EEA221 CYCLES 00000000005B13E8
```

| Control or indicator | Action or meaning |
|---|---|
| UART `r` or FIRE1 / btn[1] | restart to pass 0, rerun both KATs, reset counters |
| Other keys and buttons | ignored |
| DIP switches | absent; never read |
| LED0–LED6 | current pass number modulo 128 |
| LED7 | chain step bit 10; toggles every 1,024 steps |

UART is 115200 8N1, L4 out (`ftdi_rxd`) and M1 in (`ftdi_txd`), with 217 clocks
per bit. Buttons debounce for 250,000 clocks (10 ms). Restart finishes the
transmitter's current byte, cancels the remainder of an interrupted line,
then prints RESTART. It preserves the RX FIFO so queued keys keep their order.
Power-on KAT output may precede the requested marker. The gallery checker
compares from the first RESTART, which is emitted only for `r` or FIRE1.

[gui.script](gui.script) runs at maximum speed, types `r` at two wall seconds,
lets the chains run, and quits at 150 seconds, with the 3D board and its LEDs.
This allows the full expected window plus margin at 0.07 MHz.

## Optimisations not taken

No rounds are unrolled, no seed prefix state or schedule words are cached,
and no intermediate state is precomputed. Every link runs all 64 rounds.

- Two rounds per clock could reduce the ordinary interval from 67 to about 35
  clocks, about **1.91×** at an unchanged clock, at roughly twice the round
  logic and a longer combinational path. Timing would need a new measurement.
- A fully unrolled pipeline can accept independent messages frequently, but
  each chain must wait for its own result. Many interleaved chains would be
  needed to fill such a pipeline; its area would be much larger.
- Seed prefix precomputation affects only one of 3,073 hashes per core/pass,
  so its possible throughput gain is below 0.04% here.
- More infrequent reports or a faster link would reduce UART waiting. The
  default retains complete, ordered output for straightforward checking.

## Checks

All three checks below ran on **2026-11-17, commit 889ba8c**. Records are in
[HR03](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).

```sh
sh examples/gallery/sha256_chains/sim/run.sh
examples/ulx3s/build.sh examples/gallery/sha256_chains target/gallery/sha256_chains
python3 examples/gallery/check.py sha256_chains --emulator
```

The RTL simulation uses independent Python `hashlib` for every computed seed
and link, every STEP, both KATs and the final concatenation digest. It checks
ordering, clock counts, UART framing, controls and odd-core padding. Its
6,260,000-cycle window produces **2,780 bytes**: both KATs, 30 STEP lines and
one CHAIN line. Repeated runs are byte-identical. A late `r` at cycle 200,000,
after both power-on KAT lines begin, produces exactly one RESTART and the same
expected bytes from it.

**Emulator:** the **board's own Mac-built configuration** passed on rserv,
without rebuilding, in `check.py all --emulator`.
All **2,780 bytes from RESTART** matched. Fast mode chose **one thread** and ran
**7,086,000 cycles in 75.9 s, 0.09 MHz**. The configuration SHA-256 is
`e960285d6ef004177ecc394cda41fb31e10af3e108f920a92b80b551e7bee27e`;
its verified hash and PASS/rate lines are in
[emulator_board_configs.log](../../../docs/inventory/LFE5U-85F/hr03_gallery/emulator_board_configs.log).

**Board:** **3 of 3 SRAM loads passed**, at 18:07:30Z, 18:07:47Z and 18:08:04Z.
Each transcript has exactly one RESTART; the three transcripts are identical
over **33,375 bytes** from it. Independent `hashlib` recomputation of load 1
confirms all **381 STEP values** and **12 CHAIN digests**. The loaded bitstream
SHA-256 is
`565fc42c7a9d6025a00925f00261b886c2e8d5edcc5abb0149fa481781872807`.

**Window:** `gui_run.py sha256_chains` ran for **150 s**, with the display awake
and unlocked at the start and end:

- **8,967 window frames**, median **16.6 ms**, 95th percentile **19.0 ms**;
- **9,717,418 cycles** (0.389 s of board time), **0.07 MHz** shown;
- **7 LED drawings** read back, **0 wrong**;
- console **PASS on 2,780 bytes** from its single RESTART;
- independent `hashlib` recomputation confirms **47 STEP values** and
  **1 CHAIN digest** in the console.

HR03 records: [sha256_chains.config.gz](../../../docs/inventory/LFE5U-85F/hr03_gallery/sha256_chains.config.gz)
and [sha256_chains.board_uart_1.txt.gz](../../../docs/inventory/LFE5U-85F/hr03_gallery/sha256_chains.board_uart_1.txt.gz).
Load entries are in [observations.log](../../../docs/inventory/LFE5U-85F/hr03_gallery/observations.log).
