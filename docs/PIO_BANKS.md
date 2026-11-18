# PIO, pin pairs, and banks

The runtime now has a [basic digital PIO binding](AUTOMATIC_EXECUTION.md) for an unambiguous mode. A native example revealed a specific BASE_TYPE/PULLMODE overlap: the default DOWN clears a bit of OUTPUT_LVCMOS33, after which the strict decoder sees NONE. The [evidence](inventory/LFE5U-85F/automatic_pio_overlay.json) keeps the original observations; such an image is rejected for now. The full mode interpretation and the physical check of P01/P04 remain open.

On October 22, 2026, all named PIO/BANK settings of the LFE5U-85F were additionally collected. These extend the settings in [IOLOGIC_RESOURCES.md](IOLOGIC_RESOURCES.md) and [IO_ROUTING.md](IO_ROUTING.md): the earlier 1 316 PIO data-select/tristate fields did not include the whole buffer configuration.

## Scope and origin

| Objects/data | Result |
|---|---:|
| PIO | 365 |
| Banks from iodb/BANKREF | 0,1,2,3,4,6,7,8 |
| PIO per these banks | 56,48,48,64,24,64,48,13 |
| Composite fields / fragments | 4 451 / 4 883 |
| Bit references / unique CRAM positions | 15 435 / 8 928 |
| Positions with several owner resources | 1 204 |
| Overlaps with other families from earlier catalogs | 448 |
| Groups of identical full enum encodings | 373 |
| Composite fields with differing fragment defaults | 328 |
| Distinguishable setting descriptions checked against the native API | 439, 4 064 encoding groups in total |

All named PIO/BANK fields of the used tiles received an owner. At the level of each value, no contradictions in bit polarity were found. This is a check of encodings and bindings, not of the validity of all combinations or of the buffer behavior.

BASE_TYPE is composed from several tiles, including neighboring configuration fragments. For 328 PIO the fragment defaults diverge: NONE, INPUT_LVCMOS12, and BIDIR_LVCMOS12 occur in one field. One cannot take the last default and declare the pin active. The joint set of CRAM constraints must be decoded, the alternatives kept, and the modes/routes taken into account. The voltage of the external bank is not measured from BASE_TYPE.

Collected: BASE_TYPE, PULLMODE, OPENDRAIN, HYSTERESIS, DRIVE, SLEWRATE, CLAMP, DIFFRESISTOR, TERMINATION_1V35/1V5/1V8, the four DATAMUX/TRIMUX, and BANK.VCCIO/DIFF_REF/LVDSO/VREF. In all eight BANK.VCCIO the labels NONE/1V2/1V5 have the same encoding. nextpnr additionally writes 1V2 for a required 1.35 V. The input profile of the board must set the real power supply separately.

## What is a separate digital node

A PIO keeps a separate input receiver, output driver, tristate enable, IOLOGIC data select, pull, and physical pad. In a bidir, a change of the external level must pass back to the input regardless of the value that its own driver tries to output. Open-drain defines the allowed driver states; Z, a weak pull, and a conflict of an external source are not replaced by an unconditional zero.

The driver strength is needed for the digital resolution of strong/weak and of conflicts. The DRIVE numbers and the slew/termination parameters are kept as configuration; by themselves they do not define a delay, a current, a voltage, or a SPICE model. Accepting an unknown/unsupported electrical mode must come with an explicit diagnostic or an environment profile.

According to [sysI/O FPGA-TN-02032-1.4](https://www.latticesemi.com/view_document?document_id=50464), January 2023, pp. 10–12, the side pins are grouped into A/B and C/D. Both pairs have referenced/differential inputs, but a true LVDS output exists only on A/B. The top/bottom pairs use a different buffer design; pseudo-differential outputs cannot be identified with a true LVDS driver. The positive/negative pad and the shared differential receiver must remain linked objects, and the availability in a package is checked separately.

The document speaks of disabling the pulls in output mode. The stored PULLMODE field and the fact that nextpnr writes it do not yet prove that the pull acts under the given BASE_TYPE. This is a separate mode condition.

## Dynamic bank controllers

The [power guide FPGA-TN-02210-1.4](https://www.latticesemi.com/view_document?document_id=50468), pp. 5–6, directly describes BCINRD/BCLVDSO for banks 2/3/6/7. BCINRD disables the differential/referenced receivers and the related reference circuits; BCLVDSO disables the LVDS outputs. These functions do not apply to all LVCMOS/LVTTL inputs of a bank. The document describes the power-down control as active high; the exact polarity of the wire named enable must be checked during binding, not derived from the name.

Eight source references of controllers and 32 templates of fixed BNK_INRD/BNK_LVDS links to PIO are kept. The exact binding of the endpoints and the settings is not yet established. A site reference with a negative vendor coordinate is not turned into the physical position of the controller. A disabled receiver was not given an unconfirmed 0/1 output. A programmatic search by name does not substitute for the absence of this function.

Bank 8 includes the configuration functions and the JTAG power. The special SPICB0/WRITEN keeps its PIO/SIOLOGIC and its link to the pin ownership; the absence of named IOLOGIC fields does not mean the absence of a pad or a function. The pins of all packages are analyzed in [PACKAGE_JTAG.md](PACKAGE_JTAG.md).

## Open questions and reproduction

P01 — joint BASE_TYPE/defaults; P02 — data/tristate/readback and shared bits; P03 — differential pairs; P04 — driver resolution and the external environment; P05 — bank controllers; P06 — power/VREF and ambiguous codes; P07 — configuration pads; P08 — electrical settings and tool limitations. All eight questions and the four series HP01–HP04 are kept in the [machine contract](inventory/LFE5U-85F/pio_bank_research/pio_bank_contract.json); there are no hardware results.

```sh
python3 tools/collect_pio_banks.py
```

[pio_bank_research](inventory/LFE5U-85F/pio_bank_research/evidence_report.json) contains the tables of all fields/bits/defaults/overlaps, banks, PIO, and the remaining bank-control references. Next, the catalog is connected to `load` and to the basic part of the new `emulate`; the boundaries of the physical interpretation are kept above and in the description of automatic execution.

## Joint BASE_TYPE decoding (P01, added 2026-10-29)

Since this date the emulator decodes `BASE_TYPE` together with the settings that share its CRAM
bits, instead of matching each field on its own. The implementation is
[decode/pio.rs](../src/decode/pio.rs), reached through `ConfiguredDevice::pio_modes`.

**Why independent matching fails.** In the Trellis database the `BASE_TYPE` patterns carry bits
that other settings write too. In `PICL1_DQS3`, for example:
- `DRIVE 4` clears `F6B8`/`F7B8` of `OUTPUT_LVCMOS33`, so the bits look like `OUTPUT_SSTL135_II`
  or `OUTPUT_SSTL15_II`;
- the `PULLMODE` default `DOWN` clears `F9B7`, so an output left without `PULLMODE` reads as `NONE`;
- `OPENDRAIN` shares bits with `DRIVE`;
- `PIOC.BASE_TYPE` reaches into the drive and pull bits of `PIOD`, so the two PIOs of a pair overlap.

Across all 365 PIOs, the group overlapping `BASE_TYPE` is `DRIVE`, `HYSTERESIS`, `OPENDRAIN`,
`PULLMODE`, plus `CLAMP` on 141 of them.

**Writer contract.** nextpnr's `write_io` writes, per IO cell:
1. `BASE_TYPE` (PIO and PIC tile);
2. `HYSTERESIS`, `SLEWRATE`, `PULLMODE`, `DIFFRESISTOR`, `CLAMP`, `DRIVE`, `TERMINATION_*` and
   `OPENDRAIN`, in that order and only when needed.

libtrellis `config_to_tile_cram` applies the enums in file order and then the default of every
unspecified enum, in enum-name order, starting from zero. Its "BASE_ first" pass never fires for
`PIOx.BASE_TYPE` because of the name prefix, but nextpnr's order has the same effect.

**Decoding.** The decoder inverts this writer for the PIO and every PIO sharing bits with it. It
tries each field's options plus "unspecified" and both cell orders of a pair, and keeps every
assignment that reproduces the observed bits.

Only writes nextpnr can produce are considered:
- `BASE_TYPE` is always written, and never as `NONE`;
- nothing else is written without `BASE_TYPE`, except on the complement PIO of a differential
  pair (see below);
- `HYSTERESIS` is written exactly for single-ended, non-referenced inputs and bidirs;
- `DRIVE` is written only for `LVCMOS33`/`LVCMOS33D`.

Search cost is kept low in two exact steps:
1. Choices are pre-filtered on bits only one field touches.
2. Each PIO's combinations are filtered on bits only that PIO touches, before pairs are checked.

The search is capped at 20M assignments. With 60 PIOs in a fixture, this adds about 0.3 s to
loading.

The per-PIO result (`report.pio_modes` in an emulate plan) contains:
- the direction;
- every consistent `BASE_TYPE`;
- the effective overlaid settings. An unspecified enum takes its default. Without a default, it
  takes the option its leftover bits match, or `unwritten` if none matches.

Rules the planner applies:
- The direction need not be unique. Identical bits can stand for several writer inputs; for
  example, on the `SPICB0` PIO, `BIDIR_LVCMOS15` with explicit `HYSTERESIS=OFF` and
  `OPENDRAIN=OFF` writes the same bits as `OUTPUT_LVCMOS33`.
- A driver or receiver present in every consistent mode is built.
- One present in only some modes is recorded as ambiguous, and fails only if the plan uses it.
- Differential candidates execute when all aliases agree on pair ownership and input/output direction (see R03 below). Bidirectional differential and open-drain differential outputs are refused with the IO_TYPE and reason.
- For single-ended IO, at least one LVCMOS/LVTTL candidate is required. The standards that remain
  ambiguous (LVCMOS12…33, LVTTL33, HSUL12, SSTL) behave the same digitally; the bank decides them
  electrically.
- `OPENDRAIN` `unwritten` is push-pull, because nextpnr writes `OPENDRAIN` only on request.
- Images from other writers can be `unmatched`; they are rejected rather than guessed.

**Evidence:**
- Tools: the writer order is read from nextpnr and libtrellis at the pinned revisions
  ([TOOLCHAIN.md](TOOLCHAIN.md)).
- Board: all six HS01 loads ([hs01_init_slice](inventory/LFE5U-85F/hs01_init_slice/README.md)) used
  LEDs with `LVCMOS33 DRIVE=4` and the default `PULLMODE`. They drove as push-pull outputs
  (constant 1 lit, constant 0 dark, flip-flop values shown). FIRE buttons with `PULLMODE=DOWN` read
  as inputs. The emulator now executes those exact bitstreams.
- Unit tests in `decode/pio.rs` cover a synthetic pair shaped like the real overlaps.
  `tests/configured_startup_native.rs` runs the board LPF and an official-style ULX3S LPF.

**Still open (P01–P08):**
- electrical behaviour (drive strength, slew, hysteresis, clamp, termination);
- differential electrical behaviour and bidirectional differential pairs (P03);
- VREF/bank standards (P06);
- configurations written by tools other than nextpnr.

## Pseudo-differential LVCMOS33D outputs (added 2026-10-29)

ULX3S HDMI (GPDI) designs drive each TMDS pair as `IO_TYPE=LVCMOS33D` on the positive pin only.
The emulator executes this output on the top/bottom and side banks.

**What the chip does.** The sysIO Usage Guide (FPGA-TN-02032, §4.11.5, p. 14) describes it:
- pads A and C are the true pads, B and D the complement pads of a pair;
- the complement pad's single-ended driver can be driven by the complement of the true pad's
  data;
- its tristate is then the true pad's tristate.

**What nextpnr writes** (`write_io` in `ecp5/bitstream.cc`):
- `PIOA.BASE_TYPE OUTPUT_LVCMOS33D` on the true PIO. On the top bank nextpnr requires A and
  `OUTPUT`, and writes the value into the true PIO's tiles and into the `PIOT1`/`PICT1` tiles of
  the next column, where the complement PIO lives. In `PIOT0` the value has the same bits as
  `OUTPUT_LVCMOS33`. In `PIOT1` it sets `F7B0` (present in every `…D` type) and the 3.3 V driver
  bits `F15B0`–`F17B0`, but not the input bits of the B pad.
- On the complement PIO, never a `BASE_TYPE`. On the top bank it writes only `PIOB.DRIVE`. On the
  side banks it writes `PULLMODE=NONE`, and `DRIVE` and `OPENDRAIN` as for the true PIO.

**Decoding.** The joint decoder accepts those complement writes without a `BASE_TYPE` only when
the pair's true PIO has a differential `BASE_TYPE` in the same assignment. Such a PIO gets
`complement_of` in `report.pio_modes`. If some assignments claim it and others do not, its status
is `ambiguous_complement` and the planner rejects it.

Before this rule, the decoder read the complement PIO of a real `LVCMOS33D` design as an ordinary
`OUTPUT_LVCMOS33` without data, and the pin was silently `z`.

**Execution.**
- The true pad is a single-ended LVCMOS33 output: data from the fabric or the IOLOGIC (an ODDR in
  HDMI designs), with its tristate.
- The complement pad is driven by the inverted data under the same tristate and the global output
  enable.
- The complement's receiver is not used; a design reading it is rejected.
- Other differential input/output BASE_TYPE values use the same digital pair model as described below. Open-drain differential outputs remain refused.

**Checks:**
- `decode/pio.rs`, `a_differential_true_pio_claims_its_complement`: a synthetic pair.
- `tests/differential_native.rs` (`--ignored`) runs the HP02 configuration
  ([hp02_lvcmos33d](inventory/LFE5U-85F/hp02_lvcmos33d/README.md)), built by nextpnr from an HDMI-style
  design: an ODDRX1F into an `LVCMOS33D` output with a fabric tristate. The complement pad equals
  the inverse of the true pad on every cycle, and both are `z` while released.
- A left-bank pair (B2/C2) decodes and runs the same way.

**Not yet checked on the board.** The FPGA cannot read the complement pad back by itself, so the
check needs two wires on the J1 header from the pair's pins to input pins. The HP02 design and
its analysis script are ready for that run. Until then, this behaviour rests on the vendor
document and on the bits nextpnr writes.

## Differential digital IO (R03, added 2026-11-17)

`IO_TYPE=LVDS` on an ordinary output FF now executes. The planner builds the same
receiver, inverted data and tristate/GOE muxes used for ordinary IO and LVCMOS33D;
fast mode needs no changes. Pair ownership comes from the joint CRAM decoder,
including bit-identical aliases such as `OUTPUT_LVDS` / `OUTPUT_LVCMOS18D`.

- **Output:** P carries the data; N carries its complement. Both share tristate
  and global output enable. Fabric and IOLOGIC output paths use the same model.
- **Input:** the receiver reads P. The N leg and invalid-pair behaviour are not
  modelled. A stimulus normally drives P/N complementary; driving only P gives
  that value, and changing N (including P == N) does not affect the receiver.
  Ordinary four-state rules still apply if P itself is X or floating.
- **Refusals:** differential BIDIR is not implemented, including pseudo-differential
  bidir. Open-drain differential output has no complementary push-pull model.
  Mixed single-ended/differential ownership or mixed input/output direction is
  ambiguous and refused. Diagnostics include the IO_TYPE alternatives and reason.

The catalogue has 46 differential BASE_TYPE values. All 17 OUTPUT and 16 INPUT
values execute; the 13 BIDIR values remain refused. Before R03, only
`OUTPUT_LVCMOS33D` executed; the old guard refused the other 45.

| Direction | Executed BASE_TYPE suffixes (prefix each with the direction and `_`) |
|---|---|
| OUTPUT | LVCMOS12D, LVCMOS15D, LVCMOS18D, LVCMOS25D, LVCMOS33D, LVDS, LVDS25E, BLVDS25E, MLVDS25E, LVPECL33E, HSUL12D, SSTL135D_I, SSTL135D_II, SSTL15D_I, SSTL15D_II, SSTL18D_I, SSTL18D_II |
| INPUT | LVCMOS18D, LVCMOS25D, LVCMOS33D, LVDS, BLVDS25, MLVDS25, LVPECL33, SLVS, SUBLVDS, HSUL12D, SSTL135D_I, SSTL135D_II, SSTL15D_I, SSTL15D_II, SSTL18D_I, SSTL18D_II |
| BIDIR (refused) | LVCMOS18D, LVCMOS25D, LVCMOS33D, LVDS, BLVDS25E, MLVDS25E, HSUL12D, SSTL135D_I, SSTL135D_II, SSTL15D_I, SSTL15D_II, SSTL18D_I, SSTL18D_II |

**Pinned-tool mapping.** The reproducible routing/packing sweep is a
[test fixture](../tests/fixtures/r03_differential/README.md), not board evidence.
nextpnr writes `<direction>_<IO_TYPE>` verbatim; it does not rename BLVDS25 to
BLVDS25E or LVDS to LVDS25E. Not every spelling/direction has a Trellis encoding.
The sweep routes small designs with Yosys/nextpnr from PATH on rserv and packs
with ecppack. Its recorded matrix distinguishes routing from packing failures.

| Plain IO_TYPE | Input on side A/C | Output on side A | Notes |
|---|---|---|---|
| LVDS | INPUT_LVDS | OUTPUT_LVDS | OUTPUT_LVCMOS18D is a CRAM alias, not an extra writer row |
| BLVDS25, MLVDS25, LVPECL33, SLVS, SUBLVDS | INPUT_ + IO_TYPE | no matching output encoding | Output requires an E spelling where one exists |
| LVDS25E, MLVDS25E, LVPECL33E | no matching input encoding | OUTPUT_ + IO_TYPE | Pseudo-differential electrical implementations share the digital model |
| BLVDS25E | no matching input encoding | tool rejects before writing | nextpnr's get_vccio lacks BLVDS25E; catalogue OUTPUT/BIDIR_BLVDS25E still decode |
| LVCMOS18D/25D/33D, HSUL12D, SSTL135D/15D/18D_I/II | INPUT_ + IO_TYPE | OUTPUT_ + IO_TYPE | Both SSTL drive classes use the same digital pair |
| LVCMOS12D, LVCMOS15D | no matching input encoding | top OUTPUT_ + IO_TYPE | Side A2 output configs do not pack; top B9 does |

LVDS input/output route and pack on positive A pads in side banks 2, 3, 6 and 7
(C18, L16, K2, A2). LVDS input also works on C (B2); LVDS output on C does not
pack. Differential inputs on top banks 0/1 (A10/A12) are rejected by nextpnr's
output-only rule. Top LVDS outputs do not pack. CABGA381 has no bonded bank 4/5
GPIO, so no bottom-bank input route is claimed. Inputs therefore do **not** work
on every bank with this toolchain/package.

**Electrical scope:** termination, VCCIO, common mode, swing, DRIVE, SLEWRATE
and similar settings have no digital effect here. ECP5 true-LVDS outputs require
a 2.5 V bank; the ULX3S side-bank pins run at 3.3 V, so using native LVDS output
there would be out of specification. The digital semantics above are not in
doubt. No board check was performed or is required for R03.

**Regression:** `tests/r03_differential.rs` rebuilds X05 and an LVDS input variant
as `tests/io_delay.rs` builds its designs, including routing and packing. It
compares kernel/fast samples before and after clock edges, checks P/N output
complements, input with complementary/equal N and input driven only on P, the
other packable differential spellings, and clear bidir/open-drain refusals.
`decode/pio.rs` unit tests cover differential classification, input/output aliases
and complement ownership. HP02 continues to check LVCMOS33D tristate release.
