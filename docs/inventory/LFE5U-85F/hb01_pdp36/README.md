# HB01: the two halves of a 36-bit PDPW16KD read

Question **B01** of [EBR_ADDRESSING.md](../../../EBR_ADDRESSING.md): which pins carry which half
of a 36-bit read in pseudo dual port mode. The two tools disagree:

| Conversion | Bits 17:0 of the read | Bits 35:18 |
|---|---|---|
| yosys PDP wrapper `$__PDPW16KD_` → DP16KD (`brams_map_16kd.v`) | DOA[17:0] | DOB[17:0] |
| nextpnr `pack_ebr` of a public PDPW16KD (`ecp5/pack.cc:1115`) | DOB[17:0] | DOA[17:0] |

Both write bits 17:0 through DIA and 35:18 through DIB, and lay INIT out the same way. Every
32-bit memory that yosys infers up to 512 words deep is such a read, so the answer decides
whether the emulator runs those designs as the chip does.

The bench also records which controls act on the A half of the read, because nextpnr ties the
public `RST` to both RSTA and RSTB while yosys ties RSTA to 0, and only OCEB carries the public
`OCER`.

## Bench

[gen.py](gen.py) writes [hb01.v](hb01.v) and [program.json](program.json); [hb01.lpf](hb01.lpf)
is the HC05 constraint file. Three EBRs share one write port and one read address. Each is a
DP16KD with `DATA_WIDTH_A=36`, which nextpnr writes as `MODE=PDPW16KD`, so every DOA/DOB pin
is named in the HDL and no tool picks an order:

| EBR | Read | REGMODE | Controls driven by the bench |
|---|---|---|---|
| E1 | 36 bits | NOREG | RSTA and RSTB separately (RESETMODE=SYNC) |
| E2 | 36 bits | OUTREG | OCEA and OCEB separately |
| E3 | 18 bits (`DATA_WIDTH_B=18`) | NOREG | none: the existing narrow view |

- **Clocks.** CLKA and CLKB are flip-flops of the 25 MHz observer. A step program pulses them,
  so the bench decides every edge of every port.
- **Step timing.** The controls are set at t=0, the port clocks rise at t=4 and fall at t=8,
  and the outputs are sampled at t=12 (25 MHz cycles).
- **Observation.** After each step the observer prints one UART line (115200 8N1, `ftdi_rxd` =
  L4). Each 18-bit value is printed as five hex digits:

  ```text
  S<step> A<E1 DOA> B<E1 DOB> C<E2 DOA> D<E2 DOB> E<E3 DOB>
  ```

- **Repetition.** The program repeats forever. The first pass is not printed: it brings every
  latch and output register to a value the program set.

INIT words 0–3 and the five write patterns come from a fixed LCG (`program.json`). The 46 steps:

| Steps | What they do | What they tell apart |
|---|---|---|
| 00–07 | Read INIT words 0–3; E3 reads the 18-bit words 2w, then 2w+1 | The order of INIT on DOA/DOB; E3 checks the narrow view |
| 08–0D | Write words 8 and 9 through DIA/DIB, read them back | The order of live writes |
| 0E–17 | Write words 10 and 11 with group enables 0001, 0100, 0010, 1000 over a full word, read them back | Which ADA0–3 enables which 9-bit group |
| 18–21 | E1: an edge with only RSTB, or only RSTA (on CLKB, then on CLKA), with CEB=0. Then the same with CEB=1 reading word 1 | Which reset clears which half, and whether a SYNC reset acts with CE=0 |
| 22–2D | E2: output W2, latch W3 with OCE=0, then one edge: CLKA alone with both OCE, CLKB with only OCEB, CLKB with only OCEA | Which clock and which OCE move each half of the output register |

[expect.py](expect.py) computes the lines from `program.json` alone, for each combination of
three hypotheses:
- `low=A|B`: the read path that carries bits 17:0;
- `ctl=B|own`: whether the A half follows the read port's OCEB/RSTB, or its own OCEA/RSTA;
- `ce0=reset|hold`: whether a SYNC reset at an edge with CE=0 clears the latch.

All eight combinations give different passes. For example, step 00 (INIT word 0):

```text
low=A: S00 A00D61 B02AD6 C12F7D D0AD94 E00D61
low=B: S00 A02AD6 B00D61 C0AD94 D12F7D E00D61
```

The emulator, with the order and controls it uses now (`low=A ctl=B ce0=reset`), prints the
same 46 lines as `expect.py` for that combination
([tests/ebr_wide_read_native.rs](../../../../tests/ebr_wide_read_native.rs)).

## Running it

```sh
python3 gen.py            # only after changing the bench
yosys -p "read_verilog hb01.v; synth_ecp5 -top top -json top.json"
nextpnr-ecp5 --85k --package CABGA381 --json top.json --lpf hb01.lpf --textcfg hb01.config --seed 1
ecppack --compress hb01.config hb01.bit
python3 run.py hb01.bit 3 2 | tee observations.log
```

[run.py](run.py) loads SRAM with `openFPGALoader -b ulx3s`; it never writes the flash. It then
reads the serial port for the given number of seconds (one pass takes about 0.17 s) and prints
every distinct line per step. A stable bench shows one line per step.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS). The
  bitstream is `ecppack --compress hb01.config` and is not stored.

## Observations

Made on 2026-11-11 in two runs of three loads each.
- **Stability.** Every load printed 561 or 562 lines, about 12 passes of the program. Every
  step printed one line in all of them.
- **First run.** Its log was cut after load 2 by a closed pipe (`| head`). Load 3 still reported
  46 steps and no unstable one.
- **Second run.** [observations.log](observations.log) is the full log. Its three loads printed
  the same 46 lines, kept in [pass.txt](pass.txt).

Against `expect.py`:

| Hypothesis | Steps that differ from the board |
|---|---:|
| `low=A ctl=B ce0=reset` | **0** |
| `low=A ctl=B ce0=hold` | 1 |
| `low=A ctl=own ce0=hold` | 5 |
| `low=A ctl=own ce0=reset` | 6 |
| every `low=B` | 46 |

## Reading

1. **B01: bits 17:0 of a 36-bit read come out on DOA and 35:18 on DOB.** The DOA half holds what
   DIA wrote and what INIT puts in the chunk 2w; the DOB half holds DIB and chunk 2w+1 (steps
   00–17). This is the order of the yosys PDP wrapper. nextpnr's packing of a public PDPW16KD
   (`DO[17:0]` → DOB) is the reverse. Reading its source rather than measuring it: a design that
   instantiates PDPW16KD and is placed by nextpnr gets the two halves of `DO` swapped.
2. **Group enables.** ADA0–ADA3 enable the 9-bit groups 0–3 of the 36-bit write, in that order
   (steps 0E–17).
3. **Controls of the A half.** RSTB clears both halves and RSTA neither, whether RSTA comes with a
   CLKB or a CLKA edge (steps 18–21). OCEB moves both halves of the OUTREG output register and
   OCEA neither, and a CLKA edge moves nothing (steps 22–2D). The read port's controls act on the
   whole 36-bit read.
4. **SYNC reset with CE=0.** A SYNC reset at a CLKB edge with CEB=0 clears the NOREG latch
   (step 19, the only step where `ce0=hold` differs; with CEB=1, step 1F, the reset wins over
   the read under both rules). This is the kernel's rule EC02, now observed for this
   case. Not covered: OUTREG, CS, and reset against a write.
5. **The narrow view.** In the 18-bit read of E3, word 2w is bits 17:0 and word 2w+1 is bits 35:18
   of the 36-bit word w, on DOB.

## Emulator check

The emulator's order and controls (`PDP36_LOW_PATH = 0`, `PDP36_A_PATH_OWN_CONTROLS = false` in
[kernel/ebr.rs](../../../../src/kernel/ebr.rs)) are the measured ones.
[tests/ebr_wide_read_native.rs](../../../../tests/ebr_wide_read_native.rs) runs `hb01.config` in
the event kernel, decodes the UART and gets the 46 lines of `pass.txt`. Fast mode prints the same
lines (`emulate --fast --console-out pin:L4`).

**Not covered:**
- ASYNC reset and GSR on a 36-bit read;
- REGMODE_A ≠ REGMODE_B, which the plan refuses;
- CS decoding of the read port;
- collisions between a write and a read of the same word (B05/B06).
