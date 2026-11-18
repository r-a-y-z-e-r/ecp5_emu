# Audio jack: three 4-bit resistor DACs

Added on 2026-11-17. [audio.rs](../src/peripherals/audio.rs) observes the FPGA's DAC
pads; it never answers or drives an input. The window opens **Audio (jack)** when a
design drives any bit of L, R or V. The same model attaches with `emulate --fast
--peripherals`, independently of the UART and LEDs.

## Evidence and the electrical model

**Every voltage, Vpp and waveform here is modelled from the schematic connectivity,
not measured.** The source is
[assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb), the PCB
file the 3D board uses. Its filename and board silkscreen say **v3.0.8**; its title
block says **rev v3.0.7**. These are nominal predictions for ideal 3.3 V LVCMOS33
outputs and nominal resistors, not measured rail voltages or resistor tolerances.

| Bit | L ball / resistor | R ball / resistor | V ball / resistor | Ohms |
|---|---|---|---|---|
| 0 | E4 / R14 | A3 / R18 | H5 / R57 | 1100 |
| 1 | D3 / R15 | B5 / R19 | F2 / R58 | 549 |
| 2 | C3 / R16 | D5 / R20 | F5 / R59 | 270 |
| 3 | B3 / R17 | C5 / R21 | E5 / R60 | 130 |

These resistors meet at `/analog/AUDIO_L`, `/analog/AUDIO_R`, `/analog/AUDIO_V`,
respectively AUDIO1 (`JACK_TRS_6PINS`) pads **2, 3, 4**. There is **no coupling
capacitor, ESD part or other component on these nets: only ladder resistors and
the jack**. Jack pads **5 and 6** are on their own nets with nothing else attached.
The constants and PCB evidence are in [board.rs](../src/board.rs).

The model assumes **nothing is plugged into the jack (open circuit)**. For each
channel, over the bits currently driven to a known 0 or 1:

```text
G_i = 1 / R_i
V_open = 3.3 * sum(bit_i * G_i) / sum(G_i)
R_source = 1 / sum(G_i)
```

A Z pad disconnects its resistor; it contributes to neither sum. An entirely
undriven channel has no defined voltage. An X makes the channel voltage and source
impedance unknown. A configured input is not a driven DAC bit and is left out;
resolved pad levels are the model's input, with no separate weak-pull or FPGA output
resistance model. Partly driven channels use the same conductance formula, not a
four-bit code divided by 15.

With all four bits driven, the model gives:

| Code | Open-circuit volts |
|---|---|
| 0 | 0.000000 |
| 4 | 0.865192 |
| 8 | 1.796938 |
| 12 | 2.662130 |
| 15 | 3.300000 |

The source impedance is **70.788462 Ohm**, about 71 Ohm. The code-4-to-12 Vpp is
**1.796938 V**. These are schematic-derived, unloaded values. As an example only,
a purely resistive 16–32 Ohm headphone load would multiply the open-circuit level
by `R_load / (R_load + 70.788462)`, about **0.184 to 0.311**. That load division is
**not a modelled feature**. Actual headphones, output drivers and cables need
measurement; no load, coupling/filtering, settling, binary-code glitches, noise,
playback or audio device is simulated.

## Emulated time, status and the window

`Attached` traces only the DAC bits the design can drive, mapping the remaining
bits to Z. It adds no answering inputs or response horizon. Voltage changes retain
their emulated picosecond timestamps. Per channel, `View.audio` supplies the current
code (with driven/X masks), voltage, source impedance, minimum, maximum, Vpp,
frequency estimate and transition count. Transitions count changes of a settled
channel pad word, excluding the initial observation, rather than individual bits.

Extrema and the time-weighted mean cover the last **one emulated second**, or the
observed history since startup. They use voltage-change intervals, including the
interval covering the window's left edge, so narrow pulses are not lost to scope
resampling. Undefined intervals are omitted from extrema and mean. The history's
memory and work scale with the number of voltage changes in that second.

Frequency uses **rising crossings of the time-weighted mean, with 5% of Vpp
hysteresis**. It averages the interval between the first and last complete crossing
in the window. This avoids treating each step or the uneven MSB weight as a tone
cycle. At least two rising crossings are needed; DC and insufficient/undefined
history have no estimate. An unknown interval resets crossing history. An estimate
expires after the longer of 50 ms or three estimated periods without a crossing.

A separate ring holds **2400 samples at exactly 48 kHz**, about the last 50 ms.
Resampling uses zero-order hold on an integer picosecond grid with rational clock
rounding; a sample exactly at a pad change uses the new level. The scope is an ideal
step waveform, with no analog filtering or antialiasing; it cannot display signals
above its 24 kHz Nyquist limit faithfully. The one-second extrema are independent
of this sampling grid. Cloning/checkpoints preserve the voltage history, counters,
ring and sample phase; replay after going back reproduces them exactly.

The **Audio (jack)** window shows three traces in volts versus emulated time, plus
frequency, one-second Vpp and the four-bit code for each attached channel. Its
label explicitly says **modelled, not measured, open circuit**. Recording adds
`# audio modelled open-circuit ...` lines when estimates change notably: at least
the larger of 1 Hz and 1% in frequency, or 10 mV in Vpp, including transitions to/from unavailable.
As with the other monitor records, emission is limited to once per wall-clock
second; code changes alone do not generate record lines.

The CLI's fixed summary, after the HDMI line when present, is:

```text
peripheral: audio l_hz=F l_vpp=F r_hz=F r_vpp=F v_hz=F v_vpp=F transitions=N
```

Hz fields have three decimal places; Vpp fields have six. `--` means no estimate
or no defined voltage range. DC has `hz=--` and `vpp=0.000000`. `transitions` is the
sum of the three channel counts. All analog fields remain modelled predictions.

## Reference design and checks

[audio_tone/top.v](../examples/ulx3s/audio_tone/top.v) divides the 25 MHz clock by
1000 for **25,000 sample pairs per second**. A 32-entry sine table contains only
**codes 4..12**, never the full 0..15 range: L is approximately 440 Hz and R 660 Hz.
Two continuous 32-bit phase accumulators use rounded increments 75591424 and
113387137, giving nominal digital rates 439.999997616 and 660.000002244 Hz. V stays
at code 8. The only outputs are the twelve DAC pads, LEDs and UART L4 (115200 8N1).

Every second it prints `AUDIO <count> <crc>`: decimal sample-pair count, then eight
uppercase hexadecimal CRC digits. CRC-32/ISO-HDLC uses reflected polynomial
EDB88320, initial/final XOR FFFFFFFF, over two zero-extended code bytes per sample,
**L then R**, including the first sample. The first complete line is:

```text
AUDIO 25000 F1BFDEEF
```

Phases continue across seconds; later blocks can have different CRCs even though
the displayed sample count remains 25000. This is a digital stream checksum, not a
voltage checksum or an acoustic measurement.

Run from the repository root, with the toolchain from
[docs/TOOLCHAIN.md](TOOLCHAIN.md) installed. Set `ECP5_IMAGE_READER` to the native
image reader and `ECP5_DB` to the Trellis database:

```sh
cargo test --release -p ecp5_emu --lib
cargo test --release -p ecp5_emu --test audio -- --ignored --nocapture
cargo test --release -p ecp5_emu --test live_peripherals -- --ignored --test-threads=1
cargo test --release -p ecp5_emu --test cli_peripherals -- --ignored
```

Unit tests check numerical ladder voltages, partial/Z/X channels, synthetic
quantized sines, DC/window aging, scope spacing, partial attachment and snapshot
replay. [tests/audio.rs](../tests/audio.rs) builds through the shared reference
helper, checks the allowed outputs and amplitude bound, recomputes the first two CRCs
from the actual traced code bytes, checks the UART lines, V's constant code-8
voltage and the modelled extrema. It requires L/R within **1 Hz** of 440/660:
40 us sample quantization moves individual crossings, while averaging almost a
second makes this a conservative digital estimator tolerance, not an analog
accuracy claim. A second test replays with different chunk sizes and compares the
whole status and scope. Live and CLI tests check the same stream and status; the
live test rewinds before the first report and reproduces UART, scope and statistics.

The routed check on rserv (2026-11-17) estimated **L 440.003207 Hz** and
**R 659.976766 Hz** from the emulated pad trace, with modelled **1.796937883 Vpp**.
Its first UART line and the independent CRC over 25,000 observed pad-code pairs
were both `AUDIO 25000 F1BFDEEF`. These results validate the digital stream and
model implementation; they are not analog measurements of a board.

## Board check: HR11

[HR11](inventory/LFE5U-85F/hr11_audio/README.md) records three board loads on
2026-11-17 whose first three UART lines match the emulator's, validating the
reported digital sample stream while the jack's analog signals remain unmeasured.
