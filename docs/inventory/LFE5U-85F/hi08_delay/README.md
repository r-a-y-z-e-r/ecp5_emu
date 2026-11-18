# HI08 slice: the dynamic input delay DELAYF (I06)

What the IOLOGIC input delay does with its controls. DELAYF has three: LOADN (load DEL_VALUE),
MOVE (one step) and DIRECTION. Its CFLAG output flags the end of the range. The delay steps
themselves (tens of ps each, up to 127) cannot be seen at 25 MHz; the control logic can. The
emulator refused every non-zero DEL_VALUE.

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Designs:** [hi08.v](hi08.v), [hi08b.v](hi08b.v) and [hi08c.v](hi08c.v), all with
  [hi08.lpf](hi08.lpf).
  - **Delays.** Two DELAYF on the button inputs FIRE1 (R1) and FIRE2 (T1), bottom-bank
    SIOLOGIC. DEL_VALUE is 10 and 120 (HI08), or 0 and 127 (HI08b, HI08c).
  - **Controls.** LOADN, MOVE and DIRECTION come from fabric registers and change only
    between 25 MHz cycles. A program of 2 048 steps runs them.
  - **Sample.** Each step records `{loadn, move, direction, cflag1, cflag0}`.
- **Programs:**
  - **HI08:**
    - load;
    - 140 MOVE pulses up, then load and 140 down;
    - MOVE held high for 32 cycles each way;
    - 2-cycle pulses, then LOADN low while pulses go on.
  - **HI08b:**
    - CFLAG at both ends with DIRECTION switched;
    - DIRECTION changed while MOVE is high, both ways;
    - pulses off and back onto an end.
  - **HI08c:**
    - pulses down from the values after configuration, with no load first;
    - DIRECTION switched there and back while MOVE is high;
    - a 12-cycle MOVE that reaches an end;
    - a 12-cycle MOVE at an end.
- **Runner:** [run.py](run.py) `BIT D= 3 1.5`: each design gave one line, the same in all
  three loads ([pass.txt](pass.txt), [pass_b.txt](pass_b.txt), [pass_c.txt](pass_c.txt);
  logs `observations*.log`).
- **Model check:** [hi08_model.py](hi08_model.py), output in [model_check.txt](model_check.txt).

## Reading

The model below gives every compared sample of the three records, 6 003 of them. The 138
samples after DIRECTION changed while MOVE was high, up to the next load, are left out (item
5).

1. **Counter.** The delay is a counter from 0 to 127, saturating at both ends. DIRECTION=0
   counts up, 1 counts down.
   - **Check.** From DEL_VALUE 10, CFLAG rose after 117 steps up and 10 down; from 120,
     after 7 up and 120 down.
   - **Saturation.** Further steps leave it at the end: CFLAG stays high through about 130 more
     pulses.
2. **MOVE.** Each MOVE pulse is one step, taken at its falling edge. Holding MOVE high for 32
   cycles is one step, and reaching an end shows on CFLAG when MOVE falls (1, 2 and 12-cycle
   pulses).
3. **CFLAG** is `DIRECTION ? counter == 0 : counter == 127`, at once. Switching DIRECTION with
   no MOVE switches CFLAG between the two ends.
4. **LOADN.**
   - **While low.** It loads DEL_VALUE at once, clears or sets CFLAG to match, and holds the
     counter: MOVE pulses during it do nothing.
   - **After configuration.** The counter starts at **0**, not at DEL_VALUE. With DIRECTION=1
     both CFLAGs were high from the first sample. A DELAYF whose LOADN starts high therefore
     has zero delay until its first load. nextpnr ties an unused LOADN low, which keeps
     DEL_VALUE loaded, as DELAYG needs.
5. **DIRECTION changing while MOVE is high** is outside the documented protocol, and the
   board shows no simple rule:
   - HI08b B: up at the rise, then DIRECTION 1 while high — the counter went up.
   - HI08b C: down at the rise, then DIRECTION 0 — it went down.
   - HI08c F: DIRECTION 0 at both edges and 1 in between — it went down.

**Not covered:**
- the delay itself (the step size; the timing of the delayed data);
- WAIT_FOR_EDGE;
- the output delay (OUTDEL, DELAYF/DELAYG on an output);
- the side banks (only the bottom-bank SIOLOGIC was run).

## Emulator check

The emulator now executes the delayed data path with zero delay, as it does every propagation
delay: a delay of up to 127 taps changes timing, not data. It accepts any DEL_VALUE and a
LOADN/MOVE/DIRECTION-driven DELAYF, and plans INDD as the pad
([compile/iologic.rs](../../../../src/compile/iologic.rs)).
[tests/io_delay.rs](../../../../tests/io_delay.rs) builds DELAYG, a moving DELAYF and a DELAYF
into an input register with Yosys/nextpnr. Every LED agrees in both executors, and the data
arrives on the cycle a zero delay gives.

Since 2026-11-11 DELAYF's counter and CFLAG run too, in the event kernel and in fast mode
([IOLOGIC_EXECUTION.md](../../../IOLOGIC_EXECUTION.md#input-delay)). The model is the one
above, with one choice for item 5: DIRECTION is taken at MOVE's rise and the step made at its
fall. Inside the protocol that is item 2. Outside it, it gives the board's direction in cases B
and C, though not in F. [tests/io_delay_native.rs](../../../../tests/io_delay_native.rs) runs
the three configurations here in fast mode:
- **hi08.config:** the UART line is the board's line ([pass.txt](pass.txt)), all 2 048 samples.
- **hi08b.config:** the line differs from [pass_b.txt](pass_b.txt) in 8 samples. They are steps
  75–78 and 139–142, where CFLAG falls on the board while MOVE is still high.
- **hi08c.config:** the line differs from [pass_c.txt](pass_c.txt) in 30 samples, steps 611–640
  (case F).
All 38 lie after a DIRECTION change while MOVE was high and before the next load. Taking
DIRECTION at the fall instead would differ in 108 samples of HI08b.

WAIT_FOR_EDGE and OUTDEL are refused.
