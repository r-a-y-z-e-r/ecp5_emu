# Execution of LUTRAM distributed memory

The continuation of this milestone is [global controls and explicit wake phases](SYSTEM_EXECUTION.md). In it, the user/startup GSR is tied to the FFs, GOE to the PIO, and GWDIS to LUTRAM writing. The 73 checks and the ready-unknown boundaries given below belong to the preceding milestone.

Continuation of the [automatic execution assembly](AUTOMATIC_EXECUTION.md), October 22, 2026. `emulate` now automatically executes the standard group A/B DPRAM + C RAMW from CRAM. Asynchronous 16×4 read, write on a selected edge, WREMUX and the shared address/data paths work. Source HDL or netlist is not read for execution.

This is a software model in the earlier profile `ready-unknown`: user mode has already been reached, GWDIS is released, PUR/GSR are inactive. Physical startup, user GSR, PRLD and the link between LUTRAM and SED are not implemented yet. All L01–L09 and the future checks HL01–HL06 remain open.

## One storage and real paths

[kernel/lutram.rs](../src/kernel/lutram.rs) adds `Builder::lut_ram(LutRam)`. It uses the existing sixteen cells `DeviceState::lut(node)`. The LUT read and the RAM write access one array; no separate copy of the RAM is created. The configuration image and the source INIT are kept unchanged. A write marks the read as needing recomputation even when the address does not move.

[compile/lutram.rs](../src/compile/lutram.rs) checks the three MODEs jointly: SLICEA=DPRAM, SLICEB=DPRAM, SLICEC=RAMW. The four columns and the RAMW are created as one group. The set shared bit is not turned into three independent modes. Ambiguous MODE, combinations with CCU2 and nonzero/unknown INITs of C are rejected.

In RAMW, eight directed transfers are recovered: WADO0/1/2/3 from D0/B0/C0/A0 and WDO0/1/2/3 from C1/A1/D1/B1. The input muxes of C are taken into account. After that, the source fixed connections to WAD, WD, WCK and WRE of the columns are used; they are kept as separate paths of the plan.

The table is read in the physical order A+2B+4C+8D. On write, WAD is converted once into the same order: `[WAD3, WAD1, WAD2, WAD0]`. The INIT already permuted by the packer is not permuted again. The basis and the limitations of the sources are kept in [PLC_LUTRAM.md](PLC_LUTRAM.md).

Both column pairs use CLK1 and LSR1. CLK1.CLKMUX selects normal/inverted polarity. WREMUX supports WRE/INV/0/1; the bit shared with LSR1.LSRMUX is checked and applied to the write enable exactly once. The shared WRE is not replaced by independent enables of the columns.

## Event order and X/Z

At one timestamp, the inputs and the directed paths stabilize first. All FFs and write ports get one snapshot before the new states are published. Then the FFs and the cell changes are published simultaneously, the asynchronous read is recomputed, and the following delta phases run.

If an FF captures the RAM output at the same time as the RAM writes this address, the chosen digital policy gives the FF the value before the write, and the asynchronous output, after it stabilizes, the new value. This is a reproducible rule of the kernel; a hardware collision and setup/hold are not proven by it. L03/HL03 remain open. Coinciding changes of the external address/data and the clock first go through stabilization, as in the previous kernel.

- The initial clock level does not create an artificial write. A change of address, data or WRE while the clock is held does not write the array.
- With a defined address, edge and enable=1, one cell of each column changes. WRE=0 keeps the array.
- An undefined address affects only the cells compatible with it. Each gets the union of the variants "hold" and "write".
- An undefined enable or a possible edge also merge the write with the hold. Matching values stay known.
- Z in the address/control means unknown; Z in the data turns into X. Correlations between alternative cells are not kept; excess X is possible.
- Local FF reset does not clear LUTRAM. This is not an implementation of the global PUR/GSR/GWDIS.

`Statistics` counts separately the write-port checks, the permitted defined/possible column edges and the cell changes. `lut_write_edges` is the number of column events, not the number of four-bit words. The trace so far shows wires; there is no separate log of each changed cell and no full save/resume.

## Execution scope and the boundaries of C

In `observed`, reaching one DPRAM or RAMW output creates the whole group of four columns with the shared controls. Unreachable groups keep the storage but do not execute the write. In `all-plc`, the roots are now all **83 640 FFs and all configured RAMW groups**, including memory with no observable read.

RAMW occupies the combinational positions of C. Their ordinary F/OFX are not executed as a LUT with zero INIT; direct reaching of an unsupported output is rejected. The FFs of C keep their own clock/reset/CE and the M path when SD=0. For SD=1 the DI function in RAMW is not established yet: the FF input explicitly gets X, and each such case is listed in `report.lutram[].unmodelled_c_ff_di` and in the CLI summary. This is an explicit gap L02, not evidence of a physically floating DI. It makes it possible to account for the unused FFs of C at full PLC scope without inventing their combinational function. In the native all-plc example there are four such FFs; they do not take part in the checked outputs.

The standard path of four columns is supported. A direct OFX in DPRAM, mixed MODEs, non-standard INIT of C and other unestablished combinations require further work. The family is not declared fully closed.

## End-to-end check

The [native test](../tests/lutram_native.rs) synthesizes two independent 16×4 memories with different INITs. The first writes DI on posedge clk_a, the second writes inverted DI on negedge clk_b. Four FFs capture the read of the first memory; they have a separate local reset. Yosys produces two TRELLIS_DPR16X4, nextpnr places and routes them on the 85F/CABGA381 with 28 GPIOs, and ecppack creates a compressed `.bit`.

| Input and scope | FF | Ordinary LUT4 | Columns / RAMW | Directed paths |
|---|---:|---:|---:|---:|
| `.config`, observed | 4 | 4 | 8 / 2 | 561 |
| `.bit`, observed | 4 | 4 | 8 / 2 | 561 |
| `.bit`, all-plc | 83 640 | 83 628 | 8 / 2 | 711 395 |

Each variant cross-checks **2048 timestamps** with independent arrays of expected words and the FF expectation. Checked: all sixteen initial addresses, writes to all addresses of both memories, 436/289 permitted word writes, different/coinciding edges, local reset and 108 read/write coincidences on capture. The three result sequences are identical. The CLI separately runs seven samples: a write to both memories, an address change and an FF reset with the RAM data preserved.

[Seven portable checks](../tests/lutram.rs) additionally verify: that the source INIT is unchanged, a single owner of the cells, the refresh of the read when the address does not move, both edges, X/Z, the four WREMUX, standalone write in all-plc, the shared mode, addressing of all columns, M/DI of the FFs of C, and rejections for unsupported combinations.

In total, **73 tests** passed: 66 ordinary and seven explicitly run integration tests. The earlier automatic counters/CLI/GSR rejections and the PIO overlay, the native LUT/FF kernel, reading of configuration/service data, the full architecture with 197 408 clock relations, EBR/DSP decode and the old counter with 65 538 observations also passed again. Clippy passed with no warnings, as did formatting. The external blink with ECP5_TEST_BIT was not run separately; there are no board results.

Saved: [native results](inventory/LFE5U-85F/lutram_execution_native.json), [HDL](inventory/LFE5U-85F/lutram_execution_fixture.v), [package constraints](inventory/LFE5U-85F/lutram_execution_fixture.lpf), [CLI](inventory/LFE5U-85F/lutram_execution_cli.json) and [summary of the checks](inventory/LFE5U-85F/lutram_execution_validation.json). CABGA381 here is a profile for the software check; no physical board was programmed.

```sh
cargo test --release --locked --test lutram
cargo test --release --locked --test lutram_native -- --ignored
```

Reproducible `.config`, `.bit`, events, observed plans and logs appear in `target/lutram/`. GPIO names are given in `validation.json` and `top.lpf`; `cli-events.json` is suitable for the ordinary `emulate` command.

Next, per the [plan](IMPLEMENTATION_PLAN.md): the system controls GSR/PUR/GOE/GWDIS and startup, EBR ports; PRLD, PIO overlays and disputed combinations of C need their own contracts and distinguishing hardware checks. The GUI remains after the kernel.

## Fast mode

Since 2026-10-30, [fast mode](FAST_MODE.md) also runs LUTRAM, under the rules of the event order
above but with two values: the read follows the cells and the read address, and a write stores the
data from the state before the edge, so a flip-flop capturing the read on that edge gets the old
cell. A cell that is X when fast mode starts is refused as unknown. The PicoRV32 register file in
[examples/ulx3s/picorv32_console](../examples/ulx3s/picorv32_console/top.v), 128 columns, runs
cycle for cycle as in the event kernel, and `tests/lutram.rs` compares random traffic on both clock
polarities.
