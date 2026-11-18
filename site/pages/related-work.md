# Related work

The 2026-11-16 survey looked for tools that start from an FPGA
bitstream, for tools that show the inside of a chip or a virtual board, and for reference
points on the speed of compiled simulation. Each entry links to its primary source. Where the
survey could not confirm something, the entry says so.

## Tools that start from a bitstream

All the open tools found work the same way. They turn the configuration back into a structural
Verilog netlist of the logic in use, and an external simulator (Icarus Verilog, Verilator,
Vivado's xsim) runs that netlist with the vendor's or Yosys's cell models.

| Tool | Family | Input → output | Unused fabric | Runs the design itself |
|---|---|---|---|---|
| [IceStorm](https://github.com/YosysHQ/icestorm) `icebox_vlog` | Lattice iCE40 | `.asc` → Verilog netlist ([manual](https://manpages.debian.org/testing/fpga-icestorm/icebox_vlog.1.en.html)) | dropped | no, external simulator |
| [Project Trellis](https://github.com/YosysHQ/prjtrellis) `ecpunpack` | Lattice ECP5 | `.bit` → `.config` text | listed only where not default | no |
| [ECP5 bitstream decompiler](https://github.com/VoidMercy/Lattice-ECP5-Bitstream-Decompiler) (VoidMercy) | Lattice ECP5 | `.config` → Verilog netlist plus cell models, used with Yosys/Icarus/Verilator in CTF challenges ([writeup](https://github.com/berndoJ/revenge-of-lllattice-writeup)) | pruned | no, external simulator |
| [F4PGA fasm2bels](https://github.com/chipsalliance/f4pga-xc-fasm2bels) | Xilinx 7-series | `.bit` → FASM → Verilog netlist and constraints | dropped | no, Vivado or Verilator |
| [Project Apicula](https://github.com/YosysHQ/apicula) `gowin_unpack` | Gowin | `.fs` → Verilog netlist | dropped | no, external simulator |
| [Project Mistral](https://github.com/Ravenslofty/mistral) | Intel Cyclone V | `.rbf` ↔ configuration database, used by nextpnr | — | no |
| [Debit](https://dl.acm.org/doi/10.1145/1344671.1344729), [BIL](https://github.com/florianbenz/bil) | older Xilinx | `.bit` → netlist (reverse engineering) | — | no |
| [BitMan](https://github.com/khoapham/bitman), [byteman](https://github.com/fpgasystems/byteman) | Xilinx 7-series, UltraScale | bitstream manipulation for partial reconfiguration | — | no |

The survey found no tool that executes an ECP5 bitstream directly, and none that keeps the
unused part of the die in the model. Closed vendor and fault-injection tools may work on
configuration frames directly; that could not be checked from public sources.

ecp5_emu does not produce HDL. The configuration is decoded with the Trellis database into the
settings of every resource of the LFE5U-85F, used or not, and executed from there
([Architecture](architecture.md)). That keeps every tile, site, wire and programmable
connection addressable while the design runs, which a netlist simulator cannot show, and it
makes the emulator answer for the decoding itself: a misread bit gives a wrong result instead of
being absorbed by a cell model. The price is that the emulator must know what each setting does,
which is why unmeasured settings are refused or modelled as `x`
([How it is verified](verification.md)).

## Tools that show the chip or a board

| Tool | What it shows | Live values on the routed die |
|---|---|---|
| Vendor floorplanners (Quartus Chip Planner, [Vivado](https://www.amd.com/en/products/software/adaptive-socs-and-fpgas/vivado.html) device view) | placement and routing after implementation | no: a static view |
| [nextpnr](https://github.com/YosysHQ/nextpnr) GUI, [nextpnr-viewer](https://github.com/EDAcation/nextpnr-viewer) | the die, including the ECP5, with placement and routing | no: shows the place-and-route steps, not the circuit running |
| [Digital](https://github.com/hneemann/Digital), [DigitalJS](https://github.com/tilk/digitaljs) | live values on a schematic | no: a schematic, not the chip |
| [Makerchip Virtual FPGA Lab](https://github.com/os-fpga/Virtual-FPGA-Lab) | virtual boards (LEDs, switches, displays) driven by RTL simulation | no |
| [softfpga](https://github.com/senolgulgonul/softfpga) | an iCE40-style board in the browser, from a Yosys netlist | no: no place and route |
| [8bitworkshop](https://8bitworkshop.com/), EDA Playground | RTL simulation with video or waveforms | no |

ecp5_emu's window combines what these tools show separately: the die as nextpnr draws it, live
values as Digital draws them on a schematic, and a virtual board, all from one run of the real
bitstream.

## Speed of compiled simulation

For scale, published numbers for RTL simulators on processor designs (they simulate the RTL, not a
placed and routed FPGA, so they are not a like-for-like comparison):

- Verilator is reported to run RocketChip/BOOM class SoCs at roughly 10–300 kHz on one thread
  ([ESSENT](https://github.com/ucsc-vama/essent), [RepCut](https://escholarship.org/uc/item/57f2222p)).
- ESSENT (Beamer and Donofrio, DAC 2020) is 1.5–11.5 times faster than Verilator by skipping
  inactive parts of the design. Fast mode tried the same grouping and found it slower on
  `life_full`, where nearly every cell's inputs change each cycle ([Fast mode](../../docs/FAST_MODE.md#the-full-chip-life-life_full)).
- RepCut (Wang and Beamer, ASPLOS 2023) reaches up to 27 times on 24 threads by replicating
  shared logic between partitions, the direction fast mode lists as next for large designs.
- Arcilator (CIRCT) reports 1.9–4.3 times Verilator on Rocket and BOOM
  ([CIRCT](https://github.com/llvm/circt/tree/main/tools/arcilator)).

The comparison measured in this project is on the same design: PicoRV32 from its ECP5 netlist
runs at 1.16–1.20 MHz under Verilator 5.054 and at 1.93–1.94 MHz in fast mode from the bitstream, in the same session
([Fast mode](../../docs/FAST_MODE.md#speed)).
