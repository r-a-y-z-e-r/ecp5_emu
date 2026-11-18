# HI13 start-state implementation validation

The implementation changes only the clock-source qualification in
`src/compile/eclk.rs`, the four StopSync initial values in
`src/kernel/iologic.rs`, tests and the clock/IOLOGIC records. Runtime STOP
shifting is unchanged. No commit or board operation was performed.

The planner follows the normal selected routes and plain buffers to the native
source. Primary input pins qualify through static DCC bypasses and DCCA with
constant CE=1; a LOGIC LUT with all sixteen entries 1 also qualifies a tied CE.
Fabric state and nonconstant enables do not qualify. PLL and OSCG outputs keep
stages at 1 because their configuration-time activity is unmeasured.

`tests/eclksync_start.rs` builds packed HI13 and HI12 sync_2 designs, compiles
only the observed edge-clock cone, and compares every watched transition in the
event kernel and fast mode through 1,048,576 pin cycles. It checks midpoint and
final counts against every preserved raw load, including HI13's independent
G_in=I-64 qualification. The complete-design UART CLI replays additionally
exercise the full serializer and snapshots.

| Check | Result |
|---|---|
| HI13 / HI12 sync_2 packed-image event/fast traces and counts | 2 passed |
| NONE qualification and enabled source-path tests | 5 passed; pin, constant/chained/dynamic/disabled DCCA, fabric, PLL, OSCG |
| Edge-clock controls, including initially cleared runtime STOP | 6 passed |
| HI04 / HI04b native board lines | 2 passed |
| HI03 / HI07 gearbox native event and fast replays | 4 passed |
| HI06 bridge native replays | 2 passed |
| Cross-domain fast tests | 3 passed |
| Fast-mode regression tests | 21 passed |
| Complete HI13 UART CLI, original board image and rserv rebuild | Both P=0/G=4; one complete record and 29 heartbeats each |
| HI13 support checks | 9 passed |
| Raw board analysis | Reproduces run_1/analysis.json byte-for-byte |
| Whole default crate suite, once | Passed; native tests outside the targeted list retain their ignore attributes |
| Format | Passed, all workspace Rust files |
| Clippy, all targets and features, `-D warnings` | Passed, zero warnings |

The targeted Rust run totals **45 passed, zero failed, zero ignored**. It used
`tools/rserv.sh` and its exported `TMPDIR=$HOME/work/tmp`:

```sh
tools/rserv.sh cargo test --release -p ecp5_emu \
  --test edge_clock --test edge_clock_native \
  --test gearbox_native --test gearbox71_native --test eclk_bridge_native \
  --test eclksync_none --test eclksync_start \
  --test cross_domain_fast --test fast_mode -- --include-ignored
```

[emulator_checks_corrected.json](emulator_checks_corrected.json) records the
original board-bitstream CLI replay, whose image hash matches
[SHA256SUMS.build](SHA256SUMS.build).
[emulator_checks_corrected_rserv.json](emulator_checks_corrected_rserv.json)
records the separately rebuilt and validated rserv image. Both are actual
working-tree binary results, including concurrent planner changes; neither
claims a clean-commit baseline. The historical emulator result and raw board
captures remain unchanged.

The complete default crate suite and linters also completed successfully on
rserv (exit status 0), in this sequence:

```sh
tools/rserv.sh bash -c 'cargo test --release -p ecp5_emu && cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings'
```

Both test builders remove their temporary directories on drop; the CLI rserv
build used an EXIT cleanup trap. A final remote check found no remaining
`ecp5-start-*`, `ecp5-r09-*` or `hi13-cli.*` scratch directories under the remote
TMPDIR. The shared rserv checkout, fixtures and fast-program cache were retained.
`src/kernel/fast.rs` and `src/fast.rs` have no diff. The separate changes to
`src/compile/index.rs`, `src/compile/mod.rs` and the route decoder were
not part of this implementation.
