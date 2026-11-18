# Tests

Times below were measured on 2026-11-18 on a 32-core Linux server, release build already compiled.
Opt-in measurements ran one binary at a time with `--test-threads=1`. Allow about 1.5–2× as long
on a Mac.

## Prerequisites (once)

Have Yosys, nextpnr-ecp5, ecppack and ecpbram on PATH, clang for fast mode, and the Trellis database
selected with `ECP5_DB`. HDL slang/VHDL checks also need `ECP5_OSS_CAD_SUITE` set to an OSS CAD Suite
root. Build the native reader and CI fixtures:

```sh
bash packaging/build_trellis.sh
bash packaging/build_test_fixtures.sh   # image reader + blinky_cycle and uart_hello
```

## Default suite: about 4 min

```sh
cargo test --release --locked
```

The recorded run had 451 passed, 0 failed and about 290 ignored for `ecp5_emu`, and about 250 passed
for `ecp5_gui`. Three of the default tests run `uart_hello` and `blinky_cycle`, which CI builds with
its own tools; placement differs from machine to machine, so those tests check properties that
hold for any placement, never a stored one. How the tests, the cross-check of the two executors
and the board runs fit together is on the [verification page](verification.md).

## Opt-in tests by area

Define this helper once to apply the common skips to each binary; an optional test-name filter
follows the binary name:

```sh
ignored_tests() {
  test_binary="$1"
  shift
  cargo test --release --locked --test "$test_binary" -- --ignored --test-threads=1 \
    --skip real_bitstream_matches_text_config --skip life_full --skip window_compile \
    --skip source_index_after_ready_timing --skip watching_64_nets_costs_little \
    --skip window_open_sequence "$@"
}
```

`--skip` matches substrings. The external-bitstream check needs `ECP5_TEST_BIT`. The `life_full`
checks need `target/reference/life_full`, about 40 min of place and route; prepare that reference
and omit `--skip life_full` when deliberately selecting them. The compilation, source-index and
window-load timing checks run separately and also default to the full-chip reference; they use
compilation or the headless runner rather than opening a native window. The 64-net watching check
also uses `life_full`.

Most `fast_mode` references build on demand the first time, but its compiler-override check still
needs a prepared `target/reference/life`. For example, prepare it and run that area:

```sh
examples/ulx3s/build.sh examples/ulx3s/life target/reference/life
ignored_tests fast_mode
```

Run the listed binaries separately with `ignored_tests NAME`. For the HDL frontend subset use
`ignored_tests flow real_suite`. Globs below name groups of binaries, not one Cargo target.

| Area | Test binaries | Measured time |
|---|---|---|
| Projects and build flow (real Yosys/nextpnr builds) | `flow`, `new_project`, `demo_copy`, `examples` | about 2–3 min |
| HDL frontends (`ECP5_OSS_CAD_SUITE`) | `flow`, filtered to `real_suite` | about 10 s |
| Fast mode vs the event kernel | `fast_mode` | 2.6 min warm, 5 min fresh |
| Fast mode: pads, traces and reset order | `fast_bidir`, `fast_trace`, `fast_tristate_external`, `fast_reset_order`, `kernel_speed` | under 30 s together |
| Live runner, window back end | `live_board` | 1.7 min warm, 4.6 min fresh |
| Live peripherals | `live_peripherals` | 3.7 min |
| Live switches and CLI | `live_switches`, `cli_board_inputs`, `cli_peripherals` | about 2.5 min together |
| SDRAM | `sdram` | 4.8 min |
| SPI flash | `spiflash` | 2.0 min |
| Other peripherals | `hdmi`, `oled`, `sdcard`, `adc`, `rtc`, `usrmclk` | about 1.3 min together |
| Overlay and geometry caches | `fabric_overlay`, `overlay_cache`, `geometry_cache` | about 1–2 min |
| Board-record replays, quick (each at most 2.5 min) | `dsp*`, `ebr`, `ebr_collision_native`, `ebr_wide_read_native`, `ebr_native`, `gearbox*`, `io_delay*`, `io_register_native`, `lutram*`, `names_native`, `oscillator_native`, `pll_native`, `system_native`, `dcs_native`, `clock_gate_native`, `configured_startup_native`, `differential_native`, `r03_differential`, `reference_designs`, `resource_decode`, `dtr_native`, `eclk_bridge_native`, `ebr_r02` | about 20 min together |
| EBR startup replay | `ebr_start_native` | 4.0 min |
| IOLOGIC replay | `iologic_native` | 4.4 min |
| Edge-clock replay | `edge_clock_native` | 5.2 min |
| DDR reset replay | `ddr_reset_native` | 11.5 min |
| IO latch replay | `io_latch_native` | 12.0 min |
| DSP capture and slice preload replays | `hd09_hs02_native` | 24 min |
| Narrow EBR read-before-write replay | `ebr_narrow_rbw_native` | 30 min |
| PLL reset replay | `pll_reset_native` | 38.5 min |

The held check in `ebr_r02` executes its body only with `ECP5_HELD_TESTS=1`; otherwise it returns
without exercising the pending behaviour. Read each selected test's `#[ignore]` prerequisites.

## Long run: ALU54B

Run `alu54_native` on its own: about 6–7 h single-threaded for exhaustive board-record replays.
Its individual `#[ignore]` reasons give prerequisites, and most include time estimates.

```sh
ignored_tests alu54_native
```

Excluding `alu54_native` and `life_full`, the measured sequence took about 2.8 h of test time,
or about 40 min wall time with binaries in parallel, one per core and `--test-threads=1` each,
bounded by `pll_reset_native`.
