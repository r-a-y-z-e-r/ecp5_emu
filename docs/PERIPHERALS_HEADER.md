# J1/J2 header logic probes

Added on 2026-11-17. [header.rs](../src/peripherals/header.rs) observes every
header ball the design uses. Outputs show resolved 0/1/X/Z, frequency, duty
and transitions; inputs show the pad level and the external drive setting.
The same pad-fed model attaches in the live runner and `emulate --fast
--peripherals`. It supplies no answering inputs or edge horizon.

## Wiring and shared pins

The source is [assets/ulx3s/ulx3s_v3.0.8.kicad_pcb](../assets/ulx3s/ulx3s_v3.0.8.kicad_pcb),
the file used by the 3D board: filename/silkscreen **v3.0.8**, title block
**v3.0.7**. `board::HEADER` lists name, ball, connector and physical pad;
its order is J1 then J2, GN then GP at each signal row.

| Connector | Odd pad | Net / U1 ball | Even pad | Net / U1 ball |
|---|---:|---|---:|---|
| J1 | 5 | GN0 / C11 | 6 | GP0 / B11 |
| J1 | 7 | GN1 / A11 | 8 | GP1 / A10 |
| J1 | 9 | GN2 / B10 | 10 | GP2 / A9 |
| J1 | 11 | GN3 / C10 | 12 | GP3 / B9 |
| J1 | 13 | GN4 / A8 | 14 | GP4 / A7 |
| J1 | 15 | GN5 / B8 | 16 | GP5 / C8 |
| J1 | 17 | GN6 / C7 | 18 | GP6 / C6 |
| J1 | 23 | GN7 / B6 | 24 | GP7 / A6 |
| J1 | 25 | GN8 / A5 | 26 | GP8 / A4 |
| J1 | 27 | GN9 / B1 | 28 | GP9 / A2 |
| J1 | 29 | GN10 / B4 | 30 | GP10 / C4 |
| J1 | 31 | GN11 / E3 | 32 | GP11 / F4 |
| J1 | 33 | GN12 / F3 | 34 | GP12 / G3 |
| J1 | 35 | GN13 / G5 | 36 | GP13 / H4 |
| J2 | 5 | GN14 / U17 | 6 | GP14 / U18 |
| J2 | 7 | GN15 / P16 | 8 | GP15 / N17 |
| J2 | 9 | GN16 / M17 | 10 | GP16 / N16 |
| J2 | 11 | GN17 / L17 | 12 | GP17 / L16 |
| J2 | 13 | GN18 / H17 | 14 | GP18 / H18 |
| J2 | 15 | GN19 / G18 | 16 | GP19 / F17 |
| J2 | 17 | GN20 / E17 | 18 | GP20 / D18 |
| J2 | 23 | GN21 / D17 | 24 | GP21 / C18 |
| J2 | 25 | GN22 / C15 | 26 | GP22 / B15 |
| J2 | 27 | GN23 / C17 | 28 | GP23 / B17 |
| J2 | 29 | GN24 / D16 | 30 | GP24 / C16 |
| J2 | 31 | GN25 / E14 | 32 | GP25 / D14 |
| J2 | 33 | GN26 / C13 | 34 | GP26 / B13 |
| J2 | 35 | GN27 / E13 | 36 | GP27 / D13 |

J1 pads **1,2,19,20,39,40** are `2V5_3V3`; **3,4,21,22,37,38** are GND.
J2 pads **1,2,19,20** are +3V3; **3,4,21,22,37,38** are GND; pad **39** is
`/gpio/IN5V` and pad **40** `/gpio/OUT5V`. Power pins are greyed in the
window and have no input controls. **The 5 V pins are not modelled.**

GP/GN11..13 also reach ESP32 **U9 pins 6..11**. The ESP32 is not modelled;
the model assumes it does not drive those shared nets. GN14, GP14, GN15,
GP15, GN16, GP16, GN17, GP17 are ADC **AIN0..7**, in that order. Header and
ADC reuse the same pad trace columns, rather than duplicating signals.

## External levels and replay

`live::Static::header` lists `(HEADER index, direction)` for used header
balls. `Shared::header_levels` holds the 56 external settings in table order
once ready. `Command::HeaderLevel(index, Option<bool>)` queues an input at
now + **1 ns**, branches history and records the physical setting with its
resolved driver level. Snapshots restore the header history and positions;
going back undoes later changes, and stepping/running forward replays them.
Output-only balls have no external control and ignore this command.

**Some(false)/Some(true)** drives 0/1 from outside. **None releases**:

- A pure input uses `board::input_pull`: DOWN → 0, UP → 1; NONE, ambiguous
  or missing → 0, because fast mode is two-valued and refuses a pure input at Z.
  The real pin floats with NONE; zero is a model fallback, not a measured level.
- A bidirectional pad releases the external driver to **Z**, through
  `set_at_levels`, leaving the FPGA drive or its configured pull to resolve it.

**ECP5_HEADER="GP0=1,GN3=0"** supplies start settings in both the CLI and
window; values are exactly **0, 1 or z**. Absent/empty means all released.
Unknown names, duplicates and malformed entries fail clearly. Names are the
GP/GN net names, not ball aliases; whitespace around entries is accepted.
Valid settings for unused or output-only balls do not create a driver.
The CLI uses `board::header_inputs` in both executors: `--set` and time-0
stimulus win over these defaults, and later stimulus changes them at its time.

Tests and embedders use **RunOptions::default().header(levels)**, passed to
`spawn_with`, without mutating process environment. This builder returns
`HeaderRunOptions { switches, header }`; `header: Some(levels)` overrides the
whole ECP5_HEADER setting, even when every entry is None. A None header uses
the environment fallback. The builder preserves existing DIP-only
`RunOptions { switches: … }` literals and spawn callers.

On an AIN-shared pin, an external header drive takes priority: **0 → 0 V,
1 → 3.3 V**, ahead of AdcVolts/ECP5_ADC_VOLTS and pad/default sources. Releasing
restores the previous source. AdcVolts changes made while driven are retained
and become effective on release; they do not override the header drive. This
priority and the underlying source are checkpointed together. AdcVolts itself
does not drive the digital FPGA pad.

## Measurements, window and CLI

The model keeps change intervals over the last **one emulated second**, with
the interval spanning the window's start. Frequency is the reciprocal mean
period between complete **rising edges**; at least two are required. Duty is
high-time divided by elapsed window time, including partial periods at either
end. Before a full second, it uses observed history. DC is 0 Hz and 0/100%;
unknown/Z intervals make measurements unavailable, rather than inventing bits.
Transitions count every resolved level change since the first observation.
The header state and its measurement intervals clone into peripheral snapshots.

The **Header J1/J2** window has two 2×20 grids in physical connector order.
Outputs show level, Hz and duty; input/bidirectional pads have **0 / 1 /
released** controls. Changed readings produce `# header …` record notes at
most once per wall second. The monitor's command list generalizes the ADC
hook: app.rs sends each returned live::Command directly. The Header J1/J2 panel's
real-window check is pending in the single screen-recording batch before publication;
use the [recording kit](../tools/window_record/README.md#header-j1j2-panel).

The CLI summary has fixed output order from `board::HEADER` and fields:

```text
peripheral: header GP0=1000.000Hz/25.000% GP1=2000.000Hz/50.000% ...
```

Only output/bidirectional pins appear; values use three decimal places,
`--` when unavailable. An inputs-only model prints exactly
`peripheral: header`. Levels and input settings remain in View.header.

## References and the board safety condition

[header_probe/top.v](../examples/ulx3s/header_probe/top.v) is the **board-safe
default**. Every one of the 56 header signals is an **input** with a pull,
alternating GN=UP / GP=DOWN, as in adc_read. Four internal period counters
run for **1/2/5/10 kHz**, with **25/50/75/10%** PWM thresholds. UART L4 at
115200 8N1 prints their counts and 56 input bits, first after 1 ms, then
once per second. Only LEDs and UART are outputs. Its first emulator line is:

```text
HEADER 00000001 00000002 00000005 0000000A 55555555555555 E1
```

The 14 hex digits have GN0 at bit 0, GP0 at bit 1, through GP27 at bit 55;
E is the GN0 echo, also on led[0]. This is the emulator's pull-only prediction,
not a board measurement. Actual attachments may change input levels.

[driven/top.v](../examples/ulx3s/header_probe/driven/top.v) enables the shared
HDL's DRIVE_PWM parameter and drives **GP0..GP3 only**, at **DRIVE=4**: J1 pads
**6/8/10/12**, balls **B11/A10/A9/B9**. Those nets connect only J1 and U1 in the
PCB; a plain test reads that file and rejects any additional component endpoint.
Every other header ball is an input with a pull. **Never drive the ESP32-shared
GP/GN11..13 or ADC-shared GP/GN14..17.**

**Empty-header condition, 2026-11-17:** only the power cable was plugged into
the board; J1/J2 were empty. The four outputs are permitted under that
condition, at DRIVE=4, with **SRAM loads only**. If anything
is plugged in later, recheck the condition before loading driven/. The inputs-only
variant remains the default. [HR13](inventory/LFE5U-85F/hr13_header/README.md)
records the statement, date and the completed SRAM checks.

The driven ports are **inout** with explicit ECP5 **BB** buffers. BB.I drives
PWM; BB.O reads the pad and supplies the counters. There is no shortcut from the
PWM wire to the readback counters. In each **25,000,000-clock second**, they count
rising edges and high samples separately for each pad, then latch and reset.
A first sampled high after configuration counts as a rise. Its first UART report
is after a full second, unlike the default's initial 1 ms report:

```text
HEADER 000003E8 000007D0 00001388 00002710 P 000003E8/005F5E10 000007D0/00BEBC20 00001388/011E1A30 00002710/002625A0 55555555555555 E1
```

The first four hex values remain cumulative internal period counters. **P**
introduces four `rising_edges/high_cycles` pairs in GP0..GP3 order, followed by
the same 56-bit level word and GN0 echo. Each value is eight-digit hex. The
per-second predictions are **1000/6250000, 2000/12500000, 5000/18750000 and
10000/2500000**, giving 25/50/75/10% high. GN0 changes also print after the first
report, retaining the last completed pad window and updating internal counts,
levels and echo. Shared logic/UART files remain relative symlinks.

The plain tests in [tests/header.rs](../tests/header.rs) retain the inputs-only
port/pull audit and add the driven variant's four BB buffers, 52 remaining input
ports and all pulls, four DRIVE=4 settings, reviewed source/file-list seals and
the four nets' component endpoints in the PCB. No FPGA toolchain is needed.
Ignored tests check the UART pad counts and the header model's frequency/duty;
live tests include readback, external levels and rewind/replay.

[HR13](inventory/LFE5U-85F/hr13_header/README.md) records the 2026-11-17 checks
of both variants: all twelve captured lines match the emulator (inputs-only
lines 2–4, with the early startup line missed by the serial-port opening;
driven lines 1–3), including GP0..GP3 pad counts and the 56-input pull pattern;
drive strength, behavior under load and the 5 V pins remain unmeasured.

## Limits

These are ideal logic levels, not analog measurements. Drive strength, current,
external circuit contention, loading, voltage thresholds, delays and the 5 V
pins are not modelled. X/Z are reported where the pad trace supplies them;
fast mode's pure-input fallback cannot reproduce a physically floating pin.
The unmodelled ESP32 is assumed silent. No electrical safety is inferred from
a driven output design passing its tests.
