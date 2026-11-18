# R03 differential IO tool mapping

This is a reproducible software test fixture, not a board series or hardware
measurement. It was initially placed under the board inventory during R03
research, then moved here because it contains no board evidence.

`build.py` synthesizes three small designs (ordinary input/output FF, and an
inout with a fabric tristate), applies plain LOCATE/IOBUF constraints, routes
with nextpnr, and packs each successful route with ecppack. `mapping.json`
records all 80 cases: 51 route and pack. Routing success alone does not establish
an available Trellis encoding. Logs, LPFs, JSON, configurations and bitstreams
stay in `target/r03-map/`; only the compact result matrix is retained here.

Run from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](../../../docs/TOOLCHAIN.md) installed. Set `ECP5_IMAGE_READER` to the native image reader and
`ECP5_DB` to the Trellis database:

```sh
python3 tests/fixtures/r03_differential/build.py
cargo test --release -p ecp5_emu --test r03_differential -- --include-ignored
```

The recorded sweep ran on rserv on 2026-11-17 with the pinned tools
(Yosys 0.70, nextpnr-ecp5 0.11.1-77 / 6b438a1c, local Trellis).
It uses CABGA381/85k, seed 1, clock G2, LVCMOS25 on the ordinary pins. The matrix
covers all 46 differential catalogue BASE_TYPE values, unsupported direction
spellings, LVDS in every bonded bank (0/1/2/3/6/7), side C input/output and the
top-only LVCMOS12D/15D outputs. No bottom-bank GPIO is bonded in this package.
The CSV catalogue uses an aggregate BASE_TYPE row; its 46 pair values are 17
OUTPUT, 16 INPUT and 13 BIDIR. The former blanket refusal covered 45 of them,
excluding OUTPUT_LVCMOS33D.

The writer spells BASE_TYPE as `<direction>_<IO_TYPE>` without renaming.
OUTPUT_LVDS and OUTPUT_LVCMOS18D subsequently decode as bit-identical aliases.
BLVDS25E output/bidir currently fails nextpnr's get_vccio assertion. INPUT_*E
has no matching input encoding; outputs spelled BLVDS25/MLVDS25/LVPECL33/SLVS/
SUBLVDS have no matching side output encoding. These tool boundaries are
distinct from the emulator's ability to decode the catalogue. nextpnr treats E standards as single-ended
when deciding to write HYSTERESIS; the decoder preserves this writer rule so
BIDIR_MLVDS25E receives a specific bidirectional refusal.

`tests/r03_differential.rs` rebuilds X05 and the LVDS input variant itself,
compares kernel/fast traces for all 32 packable input/output spellings, and tests
bidir/open-drain diagnostics. The input receiver reads P. N and invalid-pair
behaviour are not modelled; the test varies N between complement, equal and
undriven. Outputs use P=data, N=complement and shared tristate/GOE.

This test never programs a board. Native LVDS output needs a 2.5 V bank; ULX3S
side banks run at 3.3 V, so that physical use would be out of specification.
The deterministic digital model ignores those electrical properties.
