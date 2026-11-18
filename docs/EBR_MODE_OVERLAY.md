# EBR: MODE, write inversion, and CIB constants

A continuation of [EBR execution](EBR_EXECUTION.md), 22 October 2026. The load failure of DP with WEAMUX=INV was removed, and the interpretation of the CIB control line constants was fixed. The board was not programmed; this is a check of the configuration encoding and of software execution. Hardware B01–B08 remain open.

## Composite MODE

In the pinned catalog, all **208 EBRs** have exactly one overlap of MODE with another field: one WEAMUX bit. For 104 resources, MODE occupies two fragments; for 104, three. DP16KD and NONE set the shared bit to zero, PDPW16KD to one. The other bits, including the fragments of the neighboring MIB/EBR_SPINE, are needed to tell the mode and the array activity apart.

nextpnr `write_bram` writes MODE, then, in DP mode, WEAMUX. So DP with WEAMUX=INV gets a one on top of the zero bit of the MODE pattern. The full ordinary comparison of MODE gives `unmatched`, although the configuration was emitted by the standard writer. Because of this, the EBR plan used to be rejected, and INIT/WID stayed with an undefined binding.

The [EBR decoder](../src/decode/ebr.rs) now keeps both results:

- The raw MODE field, all observed bits, the sources, and `unmatched` stay unchanged.
- `ResourceResult.ebr_mode` stores the effective mode, the status `qualified`, the overlap coordinate, the base/observed values, and the WEAMUX name. For a full strict match, the status stays `resolved`.

The rule admits only the known single overlap. The following are checked: the full NONE/DP/PDP variants in all existing fragments, the identical coordinate coverage, the exact WEA/INV encoding, their shared tile, and the absence of another owner of the overlapping bits. All the other MODE bits must match. Empty/incomplete variants, additional selector encodings, conflicting fields, and indeterminable modes get no permission to execute. If DP/NONE remain indistinguishable, the result stays ambiguous.

DP/NONE allow replacing the shared zero with the one of INV. PDP requires a full original match: the nextpnr writer does not write WEAMUX in this mode. The unused WEAMUX/WEBMUX in PDP are marked `applicable=false`; an INV read from the shared bit does not become an input inversion of PDP.

The effective mode is used **before** determining the activity and the INIT/WID binding. An active DP with inversion gets its INIT; a disabled NONE with the same selector stays disabled. Active WID owners hidden by the filter still prevent an ambiguous binding. The planner applies the same result, including unobserved EBRs in all-plc. In the plan report it is available as `ebr[].mode_decode`, in the load summary as `ebr_mode_statuses`.

## Control line constants

A fixture with `.WEA(1'b0)` exposed an additional error. nextpnr removes the constant net and writes the CIB.JLSR* MUX with the value `0` and WEAMUX=INV. The previous emulator created a zero on the raw CIB wire, then inverted it into a one: the port ended up enabled for writing.

In the original Trellis fuzzer `043-cib_constmux`, the constant is set through an NCL parameter of the **EBR pin**, after the consuming mux. For JCLK/JLSR, the fuzzer offers only the variants of the signal name and `0`; for JCE, the name and `1`; for data, the name, `0`, `1`. The nextpnr source `tie_cib_signal` separately shows the required interpretation: raw CLK/LSR is tied to one, and the zero is obtained by the subsequent inversion of the input. This explains why the name of a CIB setting cannot be passed off directly as the level of the raw wire.

The [planner](../src/compile/mod.rs) now applies this rule only to JCLK0/JCLK1/JLSR0/JLSR1 with a verified encoding shape: a non-empty variant `0`, an empty variant of the source signal, and no other variants. The raw wire gets **1**, then the consuming inversion is applied once. DATA/CE keep the direct interpretation of constants. An unknown shape of the CLK/LSR encoding is rejected.

The `constants` report stores the tile, the field, the wire, the source name of the value `0`, the actual level, and the rule applied; `fields` also keeps the original result. This is an interpretation of the contract of the pinned tools. There are no measurements here of the electrical level or of analog transitions on the board.

## Checks

Four new tests of the [decoder](../src/decode/ebr.rs) check strict/composite modes, preservation of the raw error, activity, INIT/WID, hidden owners, and rejection for unknown overlaps. For all 208 resources, five valid combinations and the corruption of each remaining MODE bit were checked: **1040 valid combinations and 1872 rejected corruptions**. A separate test of the [compiler](../tests/automatic_compile.rs) distinguishes the four CIB CLK/LSR lines from an ordinary data=0, and also checks the rejection for a new unknown encoding.

The [native fixtures](../tests/ebr_native.rs) now have three variants:

| Design | MODE raw / effective | Reads A/B | Write attempts A/B |
|---|---|---:|---:|
| Controlled WE, no inversion | resolved / resolved | 78 / 35 | 39 / 13 |
| Controlled WE, WEAMUX=INV | unmatched / qualified DP | 78 / 35 | 39 / 13 |
| Constant WEA=0, inversion inserted by nextpnr | unmatched / qualified DP | 117 / 35 | 0 / 13 |

Each variant goes through four runs of 513 samples: `.config`/`.bit`, ready-unknown/wake-phases, observed/all-plc. All 83 640 FFs take part in all-plc. All A/B output bits, the FF after the memory, and the final 18 432 bits of the array are checked. For the two new variants, INIT is actually bound, while the raw MODE still has `unmatched`. The CLI repeats each variant with seven probes: 514 samples, including the initial state.

Audit and artifacts: [208 resources and sources](inventory/LFE5U-85F/ebr_mode_overlay_audit.json), [inverted WE](inventory/LFE5U-85F/ebr_mode_overlay_inverted_native.json), [read-only port](inventory/LFE5U-85F/ebr_mode_overlay_readonly_native.json), [CLI with inversion](inventory/LFE5U-85F/ebr_mode_overlay_inverted_cli.json), [read CLI](inventory/LFE5U-85F/ebr_mode_overlay_readonly_cli.json), [milestone check](inventory/LFE5U-85F/ebr_mode_overlay_validation.json). The HDL, LPF, and stimuli of the two variants are listed in the check report.

```sh
python3 tools/audit_ebr_mode_overlay.py
cargo test --locked
cargo test --release --locked --test ebr_native -- --ignored --test-threads=1

# After the native test, pins qa[0], qb[0], captured:
cargo run --release --locked -- emulate target/ebr-inverted-wea/top.bit \
  --startup ready-unknown --scope observed \
  --stimulus target/ebr-inverted-wea/events.json \
  --probe pin:C10 --probe pin:E8 --probe pin:G5 \
  --output target/ebr-inverted-wea/user-run.json
```

In total, **101 tests** passed: 90 ordinary and 11 explicitly launched native integration ones, including the earlier counters, LUTRAM, system controls, the full architecture, and CRAM/INIT. Clippy passed with no warnings, and formatting passed. The external blink with ECP5_TEST_BIT was not run separately.

The EBR family stays partial: PDP36 read, an active ASYNC reset with SYNC release, hardware collisions, and control priorities keep the [earlier limits](EBR_EXECUTION.md). The next step of the main code is the shared storages and the DSP arithmetic according to the [implementation plan](IMPLEMENTATION_PLAN.md).
