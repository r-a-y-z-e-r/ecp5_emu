# repro/ — the ALU54B accumulator, alone

This is the demo's serial-filter MAC (`../dsp_mac.v`, hand-instantiated
MULT18X18D + ALU54B) wrapped in a tiny top that drives it through seven
scripted phases and prints, over the UART, the ALU54B accumulator R and
a fabric `*`-plus-adder reference computed from the same operands, as
`R-ref` hex pairs. It exists to isolate the DSP block from the rest of
the design: if the emulator and the RTL sim disagree here, the bug is in
the DSP handling, not in the filter.

- `top.v` — the driver: constant and alternating operands, accumulator
  clears, negative products, an enable-gap phase. One line per phase.
- `sim/tb.v` — iverilog testbench; prints the same lines.
- `dsp_mac.v`, `top.lpf` — symlinks to the demo's own files, so the
  repro always tests what the demo uses.
- `sim/dsp_models.v` — symlink to the behavioural primitive models the
  RTL sim uses.
- `yosys_signed_ports/` — a separate, unrelated reproducer for the
  yosys failure the demo's build first hit; see its own README.

Run the RTL simulation:

```sh
cd sim && sh run.sh        # iverilog; prints the 7 phase lines
```

Build and run in the emulator (from the repository root):

```sh
bash examples/ulx3s/build.sh examples/gallery/fir_scope/repro target/gallery/fir_scope_repro
printf '' | target/release/ecp5_emu emulate target/gallery/fir_scope_repro/top.config \
  --names target/gallery/fir_scope_repro/top_routed.json --fast --startup configured \
  --scope observed --clock net:clk_25mhz --period-ns 40 --cycles 500000 \
  --console-out net:ftdi_rxd
```

Expected: both outputs list seven identical `R-ref` pairs (each R equals
its reference). That is what the current design produces, in iverilog,
in the event kernel and in fast mode.

## What it caught

With the accumulator's clear wired only to the ALU54B's RST0 pin (the
two MULT18X18Ds had their own RST0 tied to 0), the RTL sim passed but
the emulator — event kernel and fast mode alike — never cleared R.
Per-cycle VCD probes showed the RST0 pulse never reached the ALU.
nextpnr shares each CE/RST line between the cells of one DSP block and
keeps only one net per line (the same behaviour
docs/inventory/LFE5U-85F/hd03 describes for CE0); the multipliers'
constant RST0 won, and the ALU's own RST0 pin was ignored. The fix,
matching hd05: every cell of the block gets the same controls, RST0
included; the MULT18s ignore theirs (no register stages selected).
