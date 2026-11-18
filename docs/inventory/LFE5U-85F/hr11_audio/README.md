# HR11: the audio sample stream on the board, against the emulator

[audio_tone](../../../../examples/ulx3s/audio_tone/top.v) sends nominal 440 Hz and
660 Hz four-bit sine codes to the ULX3S jack's L/R resistor DACs, sampled at 25 kHz;
V stays at code 8. **The tone is limited to codes 4..12, not full scale.** Only the
twelve DAC pads, LEDs and UART are outputs. [PERIPHERALS_AUDIO.md](../../../PERIPHERALS_AUDIO.md)
describes the model, its open-circuit assumption and the digital comparison method.

## Setup

- **Board:** ULX3S 85K v3.0.8, SRAM loads under the shared board lock on 2026-11-17.
- **Runner:** [../hr04_hdmi_oled/run.py](../hr04_hdmi_oled/run.py), prefix `AUDIO`,
  three loads of 4 s at `2026-11-17T16:24:00Z`, `2026-11-17T16:24:10Z` and
  `2026-11-17T16:24:20Z`.
- **Configuration:** [audio_tone.config](audio_tone.config), copied from
  `target/reference/audio_tone/top.config`, built on the Mac with
  `examples/ulx3s/build.sh`, nextpnr seed 1. Maximum frequency was **141.70 MHz**
  against the **25 MHz** constraint. The loaded `top.bit` is not stored; its hash
  is in [SHA256SUMS](SHA256SUMS). The copied configuration's SHA-256 is
  `3f29418b80c0901ee65197c598cb34951711e4a57d6db978e0ffc089cb36fd06`.
- **Records:** [observations.log](observations.log), copied verbatim from
  `out/audio/board_audio.log`, including the runner's load headers and UART lines;
  [TOOLS.txt](TOOLS.txt); [SHA256SUMS](SHA256SUMS). The UART data is not private.
- **Load on the jack:** whether headphones or speakers were plugged in during
  the run **was not recorded**. No unplugged-jack condition is inferred.

## Observations

Each load captured **four complete `AUDIO` lines, twelve in all**. Every load
printed the same sequence:

```text
AUDIO 25000 F1BFDEEF
AUDIO 25000 2FB7D9FB
AUDIO 25000 2FB7D9FB
AUDIO 25000 2FB7D9FB
```

| Load | UTC start, 2026-11-17 | Lines | First CRC | Following three CRCs |
|---|---|---:|---|---|
| 1 | 16:24:00Z | 4 | F1BFDEEF | 2FB7D9FB |
| 2 | 16:24:10Z | 4 | F1BFDEEF | 2FB7D9FB |
| 3 | 16:24:20Z | 4 | F1BFDEEF | 2FB7D9FB |

The count is **25,000 L/R sample pairs per second**. CRC-32/ISO-HDLC covers two
zero-extended code bytes per sample, L then R, at the design's own sample clock.
The nominal 440 Hz and 660 Hz tones complete integer cycle counts per second;
the steady-state second's CRC repeats throughout these captures.

## The comparison

The operator ran the CLI on rserv at branch HEAD
`912ae9a8c13788bab9ee8f076905d691776db528`:

```sh
ecp5_emu emulate target/reference/audio_tone/top.config \
  --names target/reference/audio_tone/top_routed.json \
  --fast --peripherals --startup configured --scope observed \
  --clock net:clk_25mhz --period-ns 40 --cycles 100000000 \
  --console-out net:ftdi_rxd
```

It printed three complete lines in **4 s of emulated time**:

```text
AUDIO 25000 F1BFDEEF
AUDIO 25000 2FB7D9FB
AUDIO 25000 2FB7D9FB
```

**PASS: all three emulator lines are byte-identical to the first three board
lines in each load (9 of 9 compared board lines).** The fourth board line has
no complete counterpart in this emulator window and is not counted as compared.
The design ran on the board, and its reported L/R code sample stream matches the
emulator through the per-second sample counts and CRCs. This is digital evidence;
the jack signals themselves were not observed.

The same CLI run reported:

```text
peripheral: audio l_hz=440.003 l_vpp=1.796938 r_hz=660.003 r_vpp=1.796938 v_hz=-- v_vpp=0.000000 transitions=70398
```

These are **emulator estimates and modelled voltages**, not measurements from the
board. They assume ideal 3.3 V outputs, nominal ladder resistors and an open circuit,
regardless of the unrecorded load connection during the physical run.

## Emulator regression check

[tests/audio.rs](../../../../tests/audio.rs)
`audio_tone_matches_its_pad_crc_and_unloaded_ladder` now checks the first **two**
UART lines, including `AUDIO 25000 2FB7D9FB`, and independently recomputes both
per-second CRCs from 50,000 traced L/R sample pairs. Its analog assertions still
check only the schematic-derived model. The other test checks snapshot replay.

```sh
cargo fmt --all -- --check
cargo test --release -p ecp5_emu --test audio -- --ignored --nocapture
```

Checked on 2026-11-17 on rserv, at the same HEAD with the record and test changes
in the working tree: formatting passed; the ignored audio suite at default
parallelism passed **2 tests, 0 failed**. The second UART line and its pad-derived
CRC were both `2FB7D9FB`.

## What this check does not establish

**The analog voltages, waveform and frequency at the jack remain unmeasured.**
There was no instrument, and nothing on the board reads the jack back in this
design. Matching UART bytes establish only that the design ran and reported the
same digital stream; they do not verify the analog DAC levels or an acoustic output.

All voltage, Vpp and waveform predictions come from
[assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../../../../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb),
the PCB file used by the 3D board, whose **title block says v3.0.7**. They are
modelled from that connectivity, not fitted to this UART record or measured on
this board. A real headphone or speaker load can change the jack levels; its
presence was not recorded here.

**Next step:** a cable from a jack channel to an ADC header input would allow a
coarse analog check, with the load, reference and acquisition path recorded and
accounted for. An instrument could measure voltage, waveform and frequency directly.
Neither check was part of this run.
