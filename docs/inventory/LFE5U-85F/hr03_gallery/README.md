# HR03: the gallery demos on the board, against the emulator

The demo designs of [examples/gallery](../../../../examples/gallery/README.md), run on the board
on 2026-11-16–18 in the HR series: whole designs compared with the emulator, as in
[HR01](../hr01_picorv32_console/README.md) and [HR02](../hr02_ram32_mac/README.md).

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:**
  - tool versions in [TOOLS.txt](TOOLS.txt);
  - checksums in [SHA256SUMS](SHA256SUMS);
  - board-configuration hashes, all fifteen emulator PASS reports
    and fast-mode rates, including the 2026-11-18 `d377f3e` rerun in
    [emulator_board_configs.log](emulator_board_configs.log);
  - the configurations that ran, as `<demo>.config.gz`, built by `examples/ulx3s/build.sh` with
    nextpnr seed 1 (the stock SERV register-file configuration is
    [serv_shell_stock_rf.config.gz](serv_shell_stock_rf.config.gz));
  - the transcript of each demo's first load, as `<demo>.board_uart_1.txt.gz`;
  - the HDMI frames the window decoded for video_life, as
    [video_life.window_monitor.txt](video_life.window_monitor.txt) (frame, size, CRC), and
    for riscv_terminal as [riscv_terminal.window_monitor.txt](riscv_terminal.window_monitor.txt);
  - the OLED pictures with completed frames painted by the window for oled_bricks, as
    [oled_bricks.window_oled.txt](oled_bricks.window_oled.txt)
    (serial, frame count, data CRC, printed/unprinted), retained as original-build provenance;
  - the 2026-11-18 window evidence:
    [riscv_oled.window_oled.txt](riscv_oled.window_oled.txt),
    [oled_bricks_p28.window_oled.txt](oled_bricks_p28.window_oled.txt),
    [chip_synth.window_oled.txt](chip_synth.window_oled.txt),
    [chip_synth.window_audio.txt](chip_synth.window_audio.txt) and
    [hdmi_plasma.window_monitor.txt](hdmi_plasma.window_monitor.txt).
- **Runner:** `check.py DEMO --board` (a copy is in [check.py](check.py)). It loads the
  bitstream three times. After each load it does the following:
  1. opens the serial port, which takes about 0.3 s after the load;
  2. sends the demo's input from `check.json`;
  3. reads for `board_seconds`;
  4. compares the text from the marker on with the demo's `expected/uart.txt`.
  The marker is the design's first answer to that input.
  Each load gives one line in [observations.log](observations.log).
  The record's check.py copy uses per-demo required peripheral models: `oled` for
  oled_bricks and riscv_oled, defaulting to `sdram` when the list is absent.
  The board-configuration emulator rerun also passed sdram_bench: 68 bytes, init=yes and zero timing,
  unknown-sample and refresh counters.
- **Prediction:**
  - `expected/uart.txt` is the output of a Verilator simulation of the demo's RTL. It comes from
    neither the emulator nor the board.
  - For all fifteen demos, the emulator ran the board's own
    configuration (hash in
    [emulator_board_configs.log](emulator_board_configs.log)) and printed the RTL
    simulation's bytes from the marker on. SDRAM and OLED designs use `--peripherals`.
    On 2026-11-17–18 the Mac-built `top.config` and `top_routed.json` were copied to
    rserv, hashes verified equal, and checked without rebuilding. On 2026-11-18,
    fast mode `d377f3e` (captures grouped by enable/reset) reran **all fifteen**
    on those board configurations: all passed. The closing section of
    [emulator_board_configs.log](emulator_board_configs.log) supplies the current
    CLI rates; window rates below belong to their recorded runs. Each logged
    configuration SHA-256 equals its decompressed record; serv_shell uses
    `serv_shell_stock_rf.config.gz`, current oled_bricks `oled_bricks_p28.config.gz`.
  - Earlier `check.py DEMO --build --emulator` checks built rserv's own configurations:
    x86_64, with a Yosys 0.70 commit slightly different from the Mac's 0.70+post.
    Those configurations were not bit-identical to the Mac builds loaded on the board,
    so those earlier checks did not test the board's configurations.
  - sdram_bench's pass lines and cycle counts equal the board's, the RTL simulation's
    own SDRAM model's and the emulator's SDRAM model's, in both the window and the
    command-line check.

## Observations

| Demo | Input | Loads | Bytes compared | Result |
|---|---|---:|---:|---|
| riscv_shell | `reboot`, then 16 shell commands | 3 | 3 214 | all identical, cycle counts of `primes 10000` (1 305 904), `mandel` (1 025 262) and `memtest` (902 055) included |
| ebr_life (first build) | `rfpnns gp` | 3 | 64 462 | all identical |
| ebr_life | `rfpnns gp` | 3 | 80 523 | all identical |
| led_tennis | `r` | 3 | 401 | all identical: one whole attract-mode game, 5:1 |
| fir_scope (first scope) | `rnuuuf` | 3 | 6 961 | all identical, 7 frames with `MISMATCH 0`; the first load's 20 frames all read `MISMATCH 0` (the other transcripts were not kept) |
| fir_scope | `rnuuuuu2f` | 3 | 13 547 | all identical; 25, 26 and 26 frames arrived, every one `MISMATCH 0x00000000` |
| riscv_terminal | `reboot`, then 10 shell commands with two `screen` | 3 | 2 837 | all identical, the two screen CRCs `c44adea5` and `2cd64349` included |
| serv_shell (width-8 register file) | `reboot`, then 11 shell commands | 3 | 1 304 | all identical, `primes below 2000: 303, 3957776 cycles` included |
| serv_shell (stock register file) | `reboot`, then 11 shell commands | 6 | 1 304 | all identical, the same cycle counts |
| video_life | `r` | 3 | 1 748 | all identical: 60 frame lines (frame, generation, population, CRC-32 of the frame's pixels) |
| sdram_bench | `r` | 3 | 68 | all identical, and each load's 32 MiB pass `WRITE 17956864 44.5 READ 17825792 44.8 ERRORS 0` |
| mandelbrot | `r+d++wit` | 3 | 24 567 | all identical: 16 views, each with its iteration sum, the 8-view tour included |
| oled_bricks (original, 769ebb4) | `r` | 3 | 4 434 | all identical; 305 frame lines in 6 s, 50.8 fps; all three transcripts identical over 8 921 bytes from RESTART |
| oled_bricks (P18) | `r` | 3 | 4 434 | all PASS; provenance configuration `764c57af` |
| oled_bricks (P27/P28, current) | `r` | 3 | 4 434 | all PASS; transcripts identical over 8 888 bytes; window: all 392 completed-frame picture CRCs printed |
| hdmi_plasma | `r` | 3 | 1 389 | all PASS; Python recomputes all 239 complete board frame CRCs; window: all 185 decoded CRCs printed and all 182 printed CRCs decoded |
| chip_synth | `r` | 3 | 836 | all PASS; 24 AUDIO CRCs (3 s) equal the model-checked loop simulation; window: all 1 752 completed-frame picture CRCs printed, 110 audio records |
| clock_domains | `r` | 3 | 356 | all identical over 4 979 bytes from RESTART; 42–43 passes, zero errors; every pass line equals the RTL simulation's line for that pass number |
| sha256_chains | `r` | 3 | 2 780 | all identical over 33 375 bytes from a single RESTART per load; independent hashlib confirms load 1's 381 STEP values and 12 CHAIN digests |
| riscv_oled | `r` | 3 | 1 534 | all identical over 12 684 bytes from a single RESTART per load; 507 frame lines in 8 s, 63.4 fps against the predicted 63.54 |

The first ebr_life build had no reseeding of a settled world; its configuration is not kept. Its
transcript prefix is the same as the second build's up to the moment the world settles.

## Reading

1. **riscv_shell.** A PicoRV32 RV32IMC exercises the chip as one system:
   - 16 EBR of firmware RAM, read and written by byte lane;
   - a 511-byte receive FIFO in one more EBR;
   - a 32 × 32 multiplier over four MULT18X18D, and a divider in the fabric;
   - a LUTRAM register file.
   On all of it, the board runs cycle for cycle what the emulator runs: the cycle counts the
   firmware measures with `rdcycle` are equal. This extends HR01 (RV32I, 4 EBR) to the M and C
   extensions and the DSP multiplier.
2. **ebr_life.** About 300 generations of two 64 × 32 worlds in PDPW16KD, each row read 36 bits
   at a time and written back, give the same frames on the chip and in the emulator. That covers
   a settled world, its reseeding, and the LFSR soups.
3. **mandelbrot.** Three MULT18X18D inferred from `*` iterate z ← z² + c on signed Q3.14 values
   every clock. The art, and the sum of every pixel's iteration count, agree on the chip, in the
   emulator and in the simulation. A single wrong product would change the sums.
4. **fir_scope.** One 31-tap FIR filter is built twice:
   - 31 MULT18X18D inferred from `*`, with an adder tree in the fabric;
   - one MULT18X18D and a hand-instantiated ALU54B accumulating 31 products.
   On the chip they gave the same output for every sample of every frame. That is the first
   board result for an ALU54B used the way a designer would use it, as a multiply-accumulate.
   The emulator agrees, and so does the simulation, whose ALU54B is a behavioural model of the
   two opcodes used.
5. **video_life.** Life on a 640 × 480 HDMI picture: a PLL, four ODDRX1F lanes and 13 EBR. The
   board's frame lines equal the emulator's and the simulation's. The board's video itself was
   not captured. In the window, the emulator's HDMI receiver decoded frames 1 to 14 from the
   emulated pads, and their CRCs are the ones the board printed for those frames. Frames 15 on
   differ because the window's script then pressed FIRE1. So the pads carry the picture the logic
   computes, and the board computes the same frames.
6. **riscv_terminal.** riscv_shell's computer drives an 80 × 30 text screen on HDMI. The
   board's own CRC hardware fingerprinted two screens, and gave the CRCs that an independent
   Python renderer of the expected text gives. The emulator printed them too. In the window, the
   screen whose CRC the design printed was among the frames decoded from the emulated pads.
7. **serv_shell.** SERV, a bit-serial RV32I core with a third of PicoRV32's logic, runs riscv_shell's C shell. On the chip it counted 3 957 776 cycles for `primes 2000`, as the emulator and the simulation did: 16 times PicoRV32's 243 162. The stock register file (a 2-bit READBEFOREWRITE DP16KD whose ports collide in the same clock) gives the same output on the chip, and its runs led to HB11 and HB09.
8. **sdram_bench.** The board's SDRAM is written and read back in full in bursts, at 93–94 %
   of the bus bandwidth. The cycle counts are equal on the chip, in the RTL simulation's own
   SDRAM model and in the emulator's SDRAM model in the window.
9. **led_tennis.** The game's timers, LFSR and log over 18 s of the board (450 million clocks)
   agree, down to the length of every rally.
10. **oled_bricks.** Orchard computes SPI pixels from fixed-point game state,
    without a frame buffer. The current checks below use the P27/P28 build.

    **Original-build provenance (769ebb4, 2026-11-17):** 3 of 3 loads matched
    4 434 bytes, identical over 8 921 bytes: 305 frames in 6 s, 50.8 fps.
    The board configuration `7022a3a6` passed the same bytes on rserv at 0.84 MHz;
    OLED: 151 frames, CRC `0F3CECC2`, zero errors. The original 90 s window
    painted 459 OLED pictures (7 startup, all 452 others with printed CRCs),
    with 462 LED drawings, none wrong, at 2.14 MHz. Records:
    [oled_bricks.config.gz](oled_bricks.config.gz),
    [oled_bricks.board_uart_1.txt.gz](oled_bricks.board_uart_1.txt.gz),
    [oled_bricks.window_oled.txt](oled_bricks.window_oled.txt).
    Original `top.bit` SHA-256:
    `990f23674325b2c1057766552c79c257092ed1a1940d14e81e079636c7765ee3`.

    **P18 provenance (2026-11-18):** configuration `764c57af` passed 4 434 bytes
    on rserv: 75 000 000 cycles in 88.247 s, 0.85 MHz; OLED: 151 frames,
    CRC `0F3CECC2`, zero errors. All three board loads passed at 00:47:19Z,
    00:47:32Z and 00:47:44Z. Records:
    [oled_bricks_p18.config.gz](oled_bricks_p18.config.gz),
    [oled_bricks_p18.board_uart_1.txt.gz](oled_bricks_p18.board_uart_1.txt.gz),
    [observations.log](observations.log), [emulator_board_configs.log](emulator_board_configs.log).
    P18 `top.bit` SHA-256:
    `1e9c4db7c520e28a99308408acf0810d63b31f716de1d0acec35d88b54feaff0`.

    These checks are of the **P27/P28 build**, commit **“the paddle reaches both
    walls; a visible serve prompt”**, on **2026-11-18**. P18 remembers short movement
    clicks, repeats holds and keeps human game-over still until FIRE1 starts a new
    game; P27 makes both paddle widths reach both walls; P28 adds the FIRE1 serve
    prompt and makes the waiting ball follow the paddle without a frame of lag.

    **RTL:** PASS: SPI/UART timing and CRCs, **151 frames, 4 HITs, 1 MISS,
    4 434 bytes**, identical all-switches-ON attract output, restart and P18/P27/P28
    pin/SPI/control checks, plus paddle catch boundaries;
    [sim/run.sh](../../../../examples/gallery/oled_bricks/sim/run.sh), [expected/uart.txt](../../../../examples/gallery/oled_bricks/expected/uart.txt).

    **Emulator:** the board's own configuration (`630b5018`),
    [oled_bricks_p28.config.gz](oled_bricks_p28.config.gz),
    passed **4 434 bytes from RESTART** on rserv without rebuilding. Fast mode
    `d377f3e`: **75 000 000 cycles in 68.023 s, 1.10 MHz**
    (earlier on this build: 88.536 s, 0.85 MHz). OLED: 151 frames, zero errors,
    last completed CRC `89DC07BB` printed by the design;
    [emulator_board_configs.log](emulator_board_configs.log).

    **Board:** **3 of 3 SRAM loads PASS** on 4 434 bytes; transcripts identical over
    **8 888 bytes from RESTART**. `top.bit` SHA-256:
    `d402a6128bb5fb840c0ea9ea9e1f82256d319022efbc32c29fea4b2b48c988ef`.
    Records: [observations.log](observations.log),
    [SHA256SUMS](SHA256SUMS),
    [oled_bricks_p28.board_uart_1.txt.gz](oled_bricks_p28.board_uart_1.txt.gz).
    This checks UART output; the physical panel was not observed.

    **Window:** 90 s under `out/screen.lock`, display awake and unlocked at both ends:

    - **5 389 window frames**, median 16.7 ms, p95 17.3 ms;
    - **193 943 236 cycles** (7.758 s of board time), **2.16 MHz**;
    - **413 LED drawings, 0 wrong**; FIRE1 ×5, LEFT ×4, RIGHT ×4;
    - console matched **2 581 bytes** until scripted FIRE1 printed `PLAYER` before
      F0058; control lines: `PLAYER`, `SW 0100`, `SW 1100`, `PLAYER`.
      The second `PLAYER` is FIRE1 starting a fresh game after game-over (P28);
    - **398 OLED pictures**: 6 before the first completed frame; **392 of 392**
      others with printed data CRCs.

    Picture evidence: [oled_bricks_p28.window_oled.txt](oled_bricks_p28.window_oled.txt).
11. **clock_domains.** Three concurrent CDC circuits use four domains, without
    a CPU: a dual-clock DP16KD FIFO, a four-phase word handshake and an
    acknowledged pulse toggle. Board and window checks ran on **2026-11-17, commit 9d6db85**.

    **Emulator:** the earlier `--build --emulator` check used an rserv build.
    The 2026-11-17 rerun used the board's Mac-built configuration on rserv,
    without rebuilding, and passed the RTL simulation's 356 bytes from RESTART.
    The 2026-11-18 `d377f3e` rerun passed the same bytes:
    2 400 000 cycles in 4.031 s, **0.60 MHz** (earlier: 17.444 s, 0.14 MHz).

    A separate command-line run on rserv sent 30 spaces followed by `f` and
    ran 30 000 000 cycles in 232.7 s. It printed
    `FREQ 02625A00 01FCA056 024F4730`: 40 000 000, 33 333 334 and
    38 750 000 Hz. The PLL ratios are exact; the 33 1/3 MHz count varies by
    ±1 with the window phase (25 000 000 × 4/3 = 33 333 333.3). OSCG is
    exactly nominal, 310 MHz / 8.

    **Board:** 3 of 3 SRAM loads passed at 15:57:49Z, 15:57:57Z and 15:58:05Z,
    each matching 356 expected bytes. The transcripts are identical over
    4 979 bytes from RESTART: 42 passes (43 in two loads), zero errors.
    Every pass line equals the simulation's line for the same pass number.
    The loaded top.bit SHA256 is
    `6beb96be32d0c8af5b4b83df84552a0ed7978bedb1b4fb1503b879ede81c1c81`.
    One extra load with `f`, outside the byte comparison, printed frequencies
    of 40 000 000, 33 333 333 and 39 005 279 Hz: OSCG base 312.04 MHz,
    +0.66%, compared with HC06's earlier 311.8–311.97 MHz on the HC06 board.

    **Window:** `gui_run.py clock_domains`, 60 s with the display awake and
    unlocked at the start and end; 3 588 window frames, median 16.7 ms and
    95th percentile 17.2 ms. It reached 6 966 496 cycles (0.279 s of board
    time), showing 0.13 MHz. All 9 LED drawings read back were correct,
    with 9 distinct heartbeat/PLL-lock patterns. The console passed on
    356 bytes from RESTART; all 11 passes received equal the simulation's
    lines. FREQ was not reached: its 25-million-board-cycle window takes
    about three minutes at this rate, longer than the script. The emulator
    FREQ reading above therefore comes from the command line.

    These results check the protocols and content on silicon as well as in
    simulation. The emulator samples ideally: it models no metastability,
    setup/hold window or routing skew between bus bits, and cannot demonstrate
    the failure of an unsafe crossing.

    Records: [clock_domains.config.gz](clock_domains.config.gz),
    [clock_domains.board_uart_1.txt.gz](clock_domains.board_uart_1.txt.gz) and
    [clock_domains.board_freq.txt.gz](clock_domains.board_freq.txt.gz), the
    extra `f` load's transcript. The run entries are in
    [observations.log](observations.log).
12. **sha256_chains.** Ten iterative SHA-256 cores advance independent chains in
    lockstep, with FIPS 180-4 known-answer tests at power-on and restart, and
    a final digest over all cores' values computed by the same hardware.
    Board and window checks ran on **2026-11-17, commit 889ba8c**. The final build uses
    **32 460 LUTs (38.8%)**, **13 478 FF**, with **Fmax 44.59 MHz**.

    **Emulator:** the board's own configuration, SHA-256
    `e960285d6ef004177ecc394cda41fb31e10af3e108f920a92b80b551e7bee27e`,
    passed on rserv without rebuilding: **2 780 bytes from RESTART**.
    The 2026-11-18 `d377f3e` rerun ran 7 086 000 cycles in 8.966 s,
    **0.79 MHz** (earlier: 75.850 s, 0.09 MHz). The configuration hash and PASS/rate lines are in
    [emulator_board_configs.log](emulator_board_configs.log).

    **Board:** **3 of 3 SRAM loads passed**, at 18:07:30Z, 18:07:47Z and
    18:08:04Z. Each transcript has exactly one RESTART; all three are identical
    over **33 375 bytes** from it. Independent Python hashlib recomputation of
    load 1 confirms all **381 STEP values** and **12 CHAIN digests**.
    The loaded bitstream SHA-256 is
    `565fc42c7a9d6025a00925f00261b886c2e8d5edcc5abb0149fa481781872807`.

    **Window:** `gui_run.py sha256_chains`, 150 s with the display awake and
    unlocked at the start and end; **8 967 window frames**, median **16.6 ms**
    and 95th percentile **19.0 ms**. It reached **9 717 418 cycles** (0.389 s
    of board time), showing **0.07 MHz**. All **7 LED drawings** read back
    were correct. The console passed on **2 780 bytes** from its single
    RESTART. Independent hashlib recomputation confirms all **47 STEP values**
    and **1 CHAIN digest** in the window's console.

    Records: [sha256_chains.config.gz](sha256_chains.config.gz) and
    [sha256_chains.board_uart_1.txt.gz](sha256_chains.board_uart_1.txt.gz).
    The run entries are in [observations.log](observations.log).
13. **riscv_oled.** PicoRV32 draws a fixed-point 3D lantern in C while an EBR
    peripheral sends the other RGB565 frame buffer over SPI and computes CRC32.
    The 2026-11-17 Mac build (e6141fb, seed 1) uses 4 904 TRELLIS_COMB (5%),
    1 379 TRELLIS_FF, 29 DP16KD (13%), 4 MULT18X18D, 32 TRELLIS_RAMW and
    23 TRELLIS_IO, with routed Fmax 52.81 MHz (PASS at 25 MHz).

    **RTL:** PASS: firmware and single-job Verilator, late/early UART restart,
    FIRE1, SPI timing, every frame CRC and byte-identical rerun;
    [sim/run.sh](../../../../examples/gallery/riscv_oled/sim/run.sh), [expected/uart.txt](../../../../examples/gallery/riscv_oled/expected/uart.txt).

    **Emulator:** the board's own configuration (`54f65b7f`),
    [riscv_oled.config.gz](riscv_oled.config.gz),
    passed on rserv without rebuilding: **1 534 bytes from RESTART**, draw cycle counts
    included. Fast mode `d377f3e`: **26 000 000 cycles in 230.385 s, 0.11 MHz**
    (earlier: 269.054 s, 0.10 MHz). OLED: 64 frames, zero errors, last completed CRC
    `36E1F3DE` among the printed CRCs;
    [emulator_board_configs.log](emulator_board_configs.log).

    **Board:** **3 of 3 SRAM loads PASS** on 2026-11-17 at 19:36:36Z,
    19:36:51Z and 19:37:07Z, each on 1 534 bytes with one RESTART;
    identical over **12 684 bytes**, **507 frames in 8 s, 63.4 fps**.
    `top.bit` SHA-256:
    `c9793c55ad76e9ee1159e4c0477baa702af9f6c3b296aca8b099ac3a05015266`.
    Records: [observations.log](observations.log),
    [SHA256SUMS](SHA256SUMS),
    [riscv_oled.board_uart_1.txt.gz](riscv_oled.board_uart_1.txt.gz).
    This checks the board's UART; the physical panel was not observed.

    **Window:** 90 s under `out/screen.lock`, display awake and unlocked at both ends:

    - **5 383 window frames**, median 16.7 ms, p95 17.4 ms;
    - **65 218 465 cycles** (2.609 s of board time), **0.74 MHz**;
    - **2 059 LED drawings, 0 wrong**; LEFT ×3, RIGHT ×3;
    - console matched **1 066 bytes from RESTART**, F0000–F0029, until the first
      scripted LEFT at cycle 17 811 361 changed rotation from F002A;
    - **2 017 OLED pictures**: 18 before the first completed frame; all **1 999**
      others carry data CRCs printed by the design.

    Picture evidence: [riscv_oled.window_oled.txt](riscv_oled.window_oled.txt).
14. **hdmi_plasma.** Two integer effects generate HDMI pixels without a frame
    buffer: plasma and a rotating cut-glass lattice. Checks: 2026-11-18.

    **RTL:** PASS: **212 frame CRCs** equal the independent Python model
    (A: 173, B: 39), including controls, raster timing and byte-identical golden
    reruns; [sim/run.sh](../../../../examples/gallery/hdmi_plasma/sim/run.sh), [model.py](../../../../examples/gallery/hdmi_plasma/model.py),
    [expected/uart.txt](../../../../examples/gallery/hdmi_plasma/expected/uart.txt). Yosys syntax/elaboration passed.

    **Emulator:** the board's own configuration (`ac678206`),
    [hdmi_plasma.config.gz](hdmi_plasma.config.gz),
    passed **1 389 bytes from RESTART** on rserv without rebuilding. Fast mode
    `d377f3e`: **28 690 000 cycles in 35.332 s, 0.81 MHz**
    (earlier: 39.637 s, 0.72 MHz);
    [emulator_board_configs.log](emulator_board_configs.log).

    **Board:** **3 of 3 SRAM loads PASS** on 1 389 bytes. The Python model recomputes
    all **239 complete board frames' CRCs**. `top.bit` SHA-256:
    `2b91d6d64b138a9bd781a5c058709f00dc87e9f3c1320cd198559cde6eb5904e`.
    Records: [observations.log](observations.log),
    [SHA256SUMS](SHA256SUMS),
    [hdmi_plasma.board_uart_1.txt.gz](hdmi_plasma.board_uart_1.txt.gz).
    The board's video was not captured; the frame check uses UART CRCs.

    **Window:** 120 s under `out/screen.lock`, display awake and unlocked at both ends:

    - **7 175 window frames**, median 16.7 ms, p95 17.2 ms;
    - **78 467 555 cycles** (3.139 s of board time), **0.65 MHz**;
    - **13 LED drawings, 0 wrong**; FIRE1 ×1, LEFT ×1;
    - console **PASS on 1 389 bytes from RESTART**;
    - **185 decoded 640 × 480 HDMI frames painted**: all 185 decoded CRCs occur
      among the design's printed CRCs, and all **182 printed CRCs** were decoded
      from the pads. The receiver numbers from power-on and the design from RESTART,
      so matching is by CRC, not frame number.

    Receiver evidence: [hdmi_plasma.window_monitor.txt](hdmi_plasma.window_monitor.txt).
15. **chip_synth.** Glass Orchard uses four DDS/ADSR voices, stereo 4-bit jack
    DACs and a live OLED scope. Checks: 2026-11-18.

    **RTL:** PASS: **40 OLED frame CRCs and 7 AUDIO blocks** in the one-second check;
    **204 977 sample pairs and 65 AUDIO blocks** through the tune wrap, independently
    modelled; controls, restarts and byte-identical reruns passed. Records:
    [sim/run.sh](../../../../examples/gallery/chip_synth/sim/run.sh),
    [expected/uart.txt](../../../../examples/gallery/chip_synth/expected/uart.txt).
    Yosys read/hierarchy checking passed.

    **Emulator:** the board's own configuration (`3dabd197`),
    [chip_synth.config.gz](chip_synth.config.gz),
    passed **836 bytes** on rserv without rebuilding. Fast mode `d377f3e`:
    **25 000 000 cycles in 39.790 s, 0.63 MHz** (earlier: 99.202 s, 0.25 MHz).
    OLED: 40 frames, zero errors, last completed CRC `F00BD767` printed by the design;
    [emulator_board_configs.log](emulator_board_configs.log).

    **Board:** **3 of 3 SRAM loads PASS** on 836 bytes; all **24 AUDIO CRCs (3 s)**
    equal the model-checked loop simulation. `top.bit` SHA-256:
    `a848efb0bc0e2a42f608d274b17d983ba446fb3c756114dfc68c481b0b98158f`.
    Records: [observations.log](observations.log),
    [SHA256SUMS](SHA256SUMS),
    [chip_synth.board_uart_1.txt.gz](chip_synth.board_uart_1.txt.gz).

    **Window:** 120 s under `out/screen.lock`, display awake and unlocked at both ends:

    - **7 178 window frames**, median 16.7 ms, p95 17.6 ms;
    - **140 988 417 cycles** (5.640 s of board time), **1.18 MHz**;
    - **3 362 LED drawings, 0 wrong**; all six buttons once;
    - console **PASS on 836 bytes**;
    - **1 764 OLED pictures**: 12 before the first completed frame; **1 752 of
      1 752** others with printed data CRCs;
    - **110 `# audio` lines** from Audio (jack), e.g. L ≈ 320–341 Hz,
      R ≈ 259–262 Hz, both **1.797 V peak to peak** (codes 4..12), V **0 V**.
      These are modelled open-circuit voltages, not measurements.

    Evidence: [chip_synth.window_oled.txt](chip_synth.window_oled.txt),
    [chip_synth.window_audio.txt](chip_synth.window_audio.txt).
16. **Window.** All fifteen demos ran from their board configurations; their
    README records check the 3D board's LED drawings, console output and available
    peripheral views. The four final runs above were measured on **2026-11-18**
    under `out/screen.lock`, with the display awake and unlocked at the start
    and end of every run. OLED startup pictures are counted separately from
    completed-frame CRC matches; audio voltages are open-circuit model predictions.
