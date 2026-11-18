# HC01 slice: when DCCA's CE stops and restarts the clock (C02)

Question **C02** of [CLOCK_NETWORK.md](../../../CLOCK_NETWORK.md): what a DCCA (a clock buffer
with a dynamic clock enable) does with CE. The sysCLOCK guide says it stops the output at zero
without a shortened pulse, and its waveform shows a dependence on phase, but it gives no rule
to execute. The emulator refused `DCC MODE=DCCA` ("dynamic DCC is not implemented").

## Setup

- **Board:** ULX3S "85K v3.0.8", JTAG IDCODE `0x41113043` (LFE5U-85), CABGA381. Loaded over JTAG
  into SRAM with `openFPGALoader -b ulx3s`; the flash was not written.
- **Records:** tool versions in [TOOLS.txt](TOOLS.txt), checksums in [SHA256SUMS](SHA256SUMS).
- **Design:** [hc01.v](hc01.v) with [hc01.lpf](hc01.lpf); nextpnr places the DCCA as
  `DCC_T0` with `MODE=DCCA` (see [hc01.config](hc01.config)).
  - **CLKI.** A slow clock made in the fabric: a period of 8 cycles of the 25 MHz clock, high
    in phases 0–3. It enters the DCC from the CIB.
  - **CE.** Driven by a fabric register.
  - **Experiments.** Twelve of them: CE stops (1→0) or starts (0→1) at phase K = 1, 2, 3
    (CLKI high) or 5, 6, 7 (CLKI low). CE never changes together with an edge of CLKI.
  - **Observation.**
    - Two flip-flops toggle on the rising and on the falling edges of CLKO.
    - The 25 MHz clock samples them, so each mask bit is one 25 MHz cycle, from the cycle CE
      changed in. Bit 47 comes first.
    - Each line `E<n> K<phase> D<0 stop, 1 start> R<rising mask> F<falling mask>` goes out over
      the UART.
- **Runner:** [run.py](run.py) `hc01.bit E 3 1.5`: three loads, 1.5 s of lines each.

## Observations

[observations.log](observations.log): three loads, 485–486 lines each, the same twelve lines
in every load and every pass ([pass.txt](pass.txt)):

```text
E0 K1 D0 R000000000000 F100000000000
E1 K2 D0 R000000000000 F200000000000
E2 K3 D0 R000000000000 F400000000000
E3 K5 D0 R000000000000 F000000000000
E4 K6 D0 R000000000000 F000000000000
E5 K7 D0 R000000000000 F000000000000
E6 K1 D1 R010101010101 F001010101010
E7 K2 D1 R020202020202 F002020202020
E8 K3 D1 R040404040404 F004040404040
E9 K5 D1 R101010101010 F010101010101
EA K6 D1 R202020202020 F020202020202
EB K7 D1 R404040404040 F040404040404
```

## Reading

1. **Stop, CE falling while CLKI is high (K1–K3).** The pulse that is high runs to its normal
   falling edge at phase 4: one falling edge, no shortened pulse. Then CLKO stays low.
2. **Stop, CE falling while CLKI is low (K5–K7).** The very next rising edge does not pass;
   there is no edge at all.
3. **Start, CE rising while CLKI is high (K1–K3).** The rest of that high phase does not pass
   (no runt pulse); CLKO rises with the next rising edge of CLKI.
4. **Start, CE rising while CLKI is low (K5–K7).** CLKO rises with the very next rising edge.
5. So CE is taken at the rising edge of CLKI, and that high phase passes whole or not at all,
   as a latch open while CLKI is low would give. A flip-flop on the falling edge would have let
   one more pulse through in experiments 3–5, and the board shows none.

**Not covered:**
- CE changing at the same instant as an edge of CLKI (a race on the board);
- the gate's state at the end of configuration;
- the centre DCCs and clocks from a pin or a PLL.

## Emulator check

The emulator executes `MODE=DCCA` as a clock gate (`IoPath::ClockGate` in
[kernel/iologic.rs](../../../../src/kernel/iologic.rs)): CLKO takes CE at each rising edge of
CLKI and falls with CLKI.
- A CE change in the same instant as the rising edge is read after the change, as a register's
  data input is.
- It starts low until the first rising edge, an assumption.

[tests/clock_gate_native.rs](../../../../tests/clock_gate_native.rs) runs `hc01.config` in the
event kernel and in fast mode; both print the twelve lines of the board.
