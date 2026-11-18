# HS01 slice: do flip-flops take their init value at configuration?

This is the first hardware observation of the project, made on 2026-10-29. It covers part of open
question **S01**: startup, PUR, user GSR, REGSET/PRLD, disabled GSR and the global synchroniser. It
is a slice of the planned series **HS01**; see [FF_CCU2.md](../../../FF_CCU2.md) and
[HARDWARE_VALIDATION.md](../../../HARDWARE_VALIDATION.md).

## Why it matters

nextpnr gives flip-flops inferred from `reg x = init;` the setting `SLICE.GSR=DISABLED`. The
emulator therefore left such flip-flops at `x` in every startup profile, so a typical student
design without an explicit reset (a blinky, for example) never got a defined value.

## Setup

- **Board:** ULX3S, USB product string "ULX3S FPGA 85K v3.0.8", FTDI serial `<FTDI-serial>`.
- **Device:** JTAG IDCODE `0x41113043` (LFE5U-85), package CABGA381.
- **Loading:** JTAG SRAM configuration with `openFPGALoader -b ulx3s`. The board's flash was not
  written. Tool versions are in [TOOLS.txt](TOOLS.txt) and checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** six flip-flops, each driving one LED directly from Q (no inverter, checked in the
  routed netlist). D = ~Q, so a flip-flop toggles on an enabled clock edge.
  - `led[5:3]`: Lattice primitives `FD1P3AY/AX/AY` with `SLICE.GSR=ENABLED`, REGSET = SET, RESET,
    SET (init `101`).
  - `led[2:0]`: inferred `reg [2:0] r = 3'b110` with `SLICE.GSR=DISABLED`, REGSET = SET, SET, RESET
    (init `110`).
  - `led[7]` is constant 1 and `led[6]` constant 0. They are references for polarity and reading
    direction.
- **Observation:** by eye on the board. The LEDs are combinational from the flip-flops under
  test, so the observer is outside the reset being tested.
- **Pins:** silkscreen D0..D7 = `led[0..7]` = balls B2, C2, C1, D2, D1, E2, E1, H3, active-high.
  FIRE1 = R1 and FIRE2 = T1, both with PULLMODE=DOWN.

| Run | HDL | Clock | Clock enable | Bitstream |
|---|---|---|---|---|
| A | [hs01_init.v](hs01_init.v) | 25 MHz oscillator on G2 | FIRE1 | `hs01.bit` from [hs01.config](hs01.config) |
| B | [hs01_noclk.v](hs01_noclk.v) | FIRE2 button, so no edges after configuration | FIRE1 | `hs01b.bit` from [hs01b.config](hs01b.config) |

Both runs use [ulx3s.lpf](ulx3s.lpf), and both have the same REGSET and SLICE GSR settings.

The repeat series uses [hs01_repeat.v](hs01_repeat.v), built into `hs01r.bit` from
[hs01r.config](hs01r.config). It puts both runs into one bitstream. All eight flip-flops are
inferred `reg = init` with `SLICE.GSR=DISABLED`:
- `led[7:4]` are wired like run A: 25 MHz clock, enable FIRE1, init `1001`;
- `led[3:0]` are wired like run B: clock FIRE2, enable FIRE1, init `1010`.

## Observations

| Run | Right after configuration, no button touched | Meaning |
|---|---|---|
| A | D7, D4, D0 lit | All six flip-flops at the inverse of REGSET, in both groups |
| B | D7, D5, D3, D2, D1 lit | All six flip-flops at REGSET, in both groups |

Follow-up actions:
- In run A, holding FIRE1 makes LEDs 0–5 glow dimly (toggling at 25 MHz). After a release the state
  is either the REGSET pattern or its inverse, at random.
- In run B, holding FIRE1 and pressing FIRE2 several times gave D7, D4, D0: the inverse pattern.
  The flip-flops are live.

### Repeat series (`hs01r.bit`, loaded three times)

The bitstream was reloaded over JTAG each time, and nothing was pressed before observing.

| Repeat | Loaded (UTC) | Lit LEDs | `led[7:4]` (A-type, init `1001`) | `led[3:0]` (B-type, init `1010`) |
|---|---|---|---|---|
| 1 | 2026-10-29T15:44:50Z | D6 D5 D3 D1 | `0110` (inverse) | `1010` (= REGSET) |
| 2 | 2026-10-29T15:46:04Z | D7 D4 D3 D1 | `1001` (= REGSET) | `1010` (= REGSET) |
| 3 | 2026-10-29T15:46:36Z | D6 D5 D3 D1 | `0110` (inverse) | `1010` (= REGSET) |

## Reading

1. **Flip-flops with `SLICE.GSR=DISABLED` take their REGSET value at configuration**, exactly like
   GSR-enabled ones. The B-type flip-flops, which get no clock edge after configuration, equalled
   REGSET in 4 loads out of 4: run B plus three repeats, 18 flip-flop observations. Status:
   *repeated on the board* for JTAG SRAM configuration of one board. Power-up from flash was not
   checked, because it would overwrite the board's flash.
2. **With a free-running clock, flip-flops see a variable number of enabled clock edges around
   wake-up.** The A-type flip-flops came up inverted in 3 loads out of 4 and at REGSET once, and
   the whole group always flipped together. So their initial state is REGSET, after which the clock
   enable (the FIRE1 input, PULLMODE=DOWN) reads 1 for some number of 25 MHz edges whose parity
   varies from load to load. The mechanism is not established. A plausible candidate is an
   input that is still high from the configuration-time state and settles to its user-mode
   pull-down only after the flip-flops have started capturing. That would be an electrical
   settling time, not a fixed digital order. For the emulator this is an open point of the
   startup and environment model. Inputs sampled right after wake-up should not be assumed to
   be at their pull value.

## Emulator prediction for the same flip-flop logic

The emulator rejected the LED pins as built here:
- without PULLMODE, BASE_TYPE decodes as `NONE` (overlay, P01);
- with `DRIVE=4`, BASE_TYPE is ambiguous between `OUTPUT_SSTL135_II` and `OUTPUT_SSTL15_II`.

So the prediction was taken on [twin2.config](twin2.config). It is built from the same netlist
with [ulx3s_plain.lpf](ulx3s_plain.lpf) (LEDs `LVCMOS33 PULLMODE=NONE`), and its GSR and REGSET
settings are identical to the board runs.

| Profile | `led[7:0]` |
|---|---|
| `ready-unknown` | `1 0 x x x x x x` |
| `wake-phases` (4 `sys:step` pulses) | `1 0 1 0 1 x x x` |

The emulator matches run B for the GSR-enabled group. It is more conservative than the board for
the GSR-disabled group, which the board repeatedly shows initialised to REGSET.

**Used by the emulator.** The `--startup configured` profile initialises every executed PLC
flip-flop to REGSET on the strength of this result. With it, the emulator reproduces run B
exactly: `10101110` right after configuration, and inverting on enabled FIRE2 edges. The native
test `tests/configured_startup_native.rs` checks this for both `.config` and `.bit`.
