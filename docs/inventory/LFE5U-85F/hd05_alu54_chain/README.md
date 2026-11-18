# HD05 slice: an ALU54B chain that repeats exactly (D04)

The eight ALU54B of [HD04](../hd04_alu54_cascade/README.md) whose outputs are all registered,
alone in a design. HD04 itself has ALUs whose unregistered R loops through their own operand,
so its lines differ between loads and the emulator's fast mode cannot run it. This record
repeats exactly and can be replayed line by line.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hd05.v](hd05.v), written by [hd05_gen.py](hd05_gen.py), with
  [hd05.lpf](hd05.lpf); configuration [hd05.config](hd05.config). It is HD04 without the
  blocks t and u:

  | ALU | X | Set | OP | CE0 | Patterns | Shows |
  |---|---|---|---|---|---|---|
  | pa | 13 | 1 | opa | pulse | A | the opcode sweep |
  | pb | 13 | 2 | read | pulse | A | CIN >> 18 of pa |
  | qa | 22 | 2 | opb | pulse | A | the sweep in another order, with CIN from pb |
  | qb | 22 | 1 | read | pulse | A | CIN >> 18 of qa |
  | ra | 31 | 1 | read | pulse | A | CIN >> 18 of qb |
  | rb | 31 | 2 | opa | high | A | a second ALU that nextpnr puts on the first ALU's CE0 |
  | sa | 49 | 1 | opa | pulse | Z | flags with all-zero patterns |
  | sb | 49 | 2 | opa | pulse | O | flags with all-one patterns, CIN from sa |

  Yosys also turns the `opb` multiplication into a MULT18X18D of its own, which nextpnr puts
  in the unused block at X 40.
- **Lines.** `I...` before any pulse, then `T<n:4>`, the two operand sets and `<r:14><f:2>`
  for each ALU.
- **Runner:** [run.py](run.py) `hd05.bit TI 3 165`.
- **Model check:** [hd05_model.py](hd05_model.py), output in
  [model_check.txt](model_check.txt).

## Observations

[observations.log](observations.log): three loads with the same 8 195 distinct lines: the `I`
line, the 8 192 steps, and a second variant of the first two steps. That variant is the start
of the second pass, which follows the end of a pass instead of the configured state.

## Reading

The model of HD02, HD03 and HD04 gives every R and every flag of every step, for all eight
ALUs in all three loads, and the `I` line: 8 192 of 8 192 each, 0 differ.

## Emulator check

[tests/alu54_native.rs](../../../../tests/alu54_native.rs) runs [hd05.config](hd05.config):
- **Fast mode** runs the `I` line, the whole pass and the first steps of the next. Every line
  it prints is a board line.
- **The event kernel** runs the `I` line and the first two steps. Its lines are the board's.
