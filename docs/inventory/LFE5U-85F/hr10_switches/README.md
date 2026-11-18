# HR10: the DIP switches on the board, against the emulator

[switches](../../../../examples/ulx3s/switches/top.v) reads the ULX3S's four DIP inputs,
as [PERIPHERALS.md](../../../PERIPHERALS.md#dip-switches) describes. **Every switch pad
is an input; only the LEDs and UART are outputs.** The two designs share identical HDL
and differ only in PULLMODE on the four switch pads: DOWN and UP. Neither drives a switch
net. UART order is sw[0] first, with 10 ms debounce and a one-second repeat.

## Setup

- **Board:** ULX3S 85K v3.0.8, SRAM loads under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `SW`, two loads
  of 3 s per design. DOWN loads were at `2026-11-17T09:36:13Z` and
  `2026-11-17T09:36:22Z`; UP loads were at `2026-11-17T09:36:30Z` and
  `2026-11-17T09:36:39Z`.
- **Configurations:** [switches.config](switches.config) and
  [switches_pullup.config](switches_pullup.config), copies of the Mac builds in
  `target/reference/switches/top.config` and `target/reference/switches_pullup/top.config`.
  Both used `examples/ulx3s/build.sh`, nextpnr seed 1 and a 25 MHz constraint. Maximum
  frequencies were **184.64 MHz (DOWN)** and **203.46 MHz (UP)**. The loaded bits are not
  stored; their hashes are in [SHA256SUMS](SHA256SUMS).
- **Records:** [observations.log](observations.log) contains `out/switches/board_down.log`
  followed by `out/switches/board_up.log`, each verbatim under a `# design …` header.
  They hold only switch positions and may be stored. [TOOLS.txt](TOOLS.txt) and
  [SHA256SUMS](SHA256SUMS) identify the tools and artifacts.

## Observations

Each load captured three complete lines: **six per design, twelve in all**. Every line
was identical:

```text
SW 1111
```

| Design | Switch pull | Loads | Lines | Board result |
|---|---|---:|---:|---|
| `switches` | DOWN | 2 | 6 | `SW 1111` throughout |
| `switches/pullup` | UP | 2 | 6 | `SW 1111` throughout |

## The comparison

The model is **ON = 1 through R51 (130 Ohm), OFF = the pad's configured pull**. With DOWN,
an OFF switch would read zero. The DOWN result therefore identifies **all four levers as
ON**, by this model and the checked wiring. The supply path through R51 overrides the
internal pull-down, as the model assumes. UP reading `1111` is consistent with those
positions.

[tests/live_switches.rs](../../../../tests/live_switches.rs)
`the_board_all_on_read_reproduces_for_both_pulls` sets **ECP5_SWITCHES=1111**, builds both
examples on rserv with the shared reference builder, and runs them through the live runner.
It checks `SW 1111`, all four LEDs high and all four recorded positions ON. Its expected UART line is the board's line for each design.

```sh
cargo fmt --all -- --check
cargo test --release -p ecp5_emu --test live_switches -- --ignored --test-threads=1
```

Checked on 2026-11-17 on rserv: `cargo fmt --all -- --check` passed, and the serial
ignored switch suite passed **4 tests, 0 failed**, including the all-ON comparison:
`HR10 DOWN: SW 1111` and `HR10 UP: SW 1111`.

## What this check does not establish

**The OFF level and the sw[n]-to-physical-lever mapping remain untested on the board**:
every lever was ON, so these observations cannot distinguish a permutation of the four
inputs or show the response of an open contact. The emulator's OFF/pull and replay tests
are separate from this board evidence.

**Next step:** a person flips one identified lever, for example SW2 OFF with the other
three ON, then captures both designs. The predictions are **DOWN: `SW 1011`** and
**UP: `SW 1111`**. That would check both the OFF level and SW2's mapping to `sw[1]`;
repeating for the remaining levers would check the full mapping. No such flip was part
of this run.
