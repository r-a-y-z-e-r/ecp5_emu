# New projects and project files

Status: design only; review required before implementation.
Proposed commands and modules below are new work, not existing APIs.

## 1. What to borrow

Supply a named folder, known target, correct ports/constraints and a recognisable
first result in one dialog. This comparison is from established behaviour,
without a version audit or web research; it is interaction guidance.

| Prior art | What a student needs | What would be ceremony here |
| --- | --- | --- |
| Quartus New Project Wizard | Name, directory, top-level entity, source files and target device. | Separate pages for family/device selection and tool settings when the board and toolchain are fixed. |
| Vivado new project flow | A named RTL project, sources, constraints and a board or part; sources can be added later. | Project-type and target-selection pages, simulation settings and IP-related choices before a first counter. |
| Gowin IDE | A project directory, device selection and adding HDL/constraint files to a project. | Asking beginners to identify the chip/package and assemble an empty project before seeing a result. Exact page sequence and labels are **unverified**. |
| apio (`apio init` / `apio create`) | Board-aware command-line setup and a small folder the student can build or populate from an example. | Installation/package management in the creation dialog. The exact division between `init` and `create`, and their options across releases, is **unverified**; do not copy their syntax. |
| Icestudio | Board selection, visible IO and a small example that can be built quickly. | Requiring a graphical block design or translating its workspace format into this text editor. |

## 2. Creating a project

Put **New project…** beside Open on the start screen and first in Design.
The dialog contains Name and Project folder with Browse. Initially suggest
`~/ecp5-projects/<name>`; folder and name are independently editable after that
suggestion. Remember the last parent chosen; create it only on Create.
The module is always `top`, so display names may contain spaces.

Below that, show **ULX3S — LFE5U-85F / CABGA381, 25 MHz** and the resource checklist.
Clock and LEDs are checked initially. Buttons, DIP switches and UART are available
in the first slice. Other resources appear disabled in an Advanced section,
pointing to existing examples, then Start from a demo once copying is available.
Header pins say “Deferred: pin map and electrical settings need review”.

Create writes `ecp5.toml`, `top.v` and `top.lpf`, opens the project and selects
`top.v` in the editor. **Build and run**, checked initially, starts the normal
background build and then emulation; leaving it unchecked opens the source only.
Never load the board automatically. Missing tools leave an editable project with
the ordinary build diagnostic and a retry control, not a failed creation dialog.
This follows the existing build/emulate handoff in `gui/src/project.rs:468–545`
and `:950–968`; SRAM loading is a separate action (`src/flow/board.rs:17–23`).

Reject blank names, invalid folder leaves, a destination that is a file, and a
nonempty destination directory. An existing empty directory is allowed. Show
validation beside the fields; do not overwrite files or silently choose a suffix.
Cancel writes nothing. Generate and validate in a temporary sibling directory,
then publish without replacing an occupied destination; recheck at publication.
On failure remove only temporary files created by this operation. Build failures
after publication keep the complete project. This is a new creation policy.

Clock is required by every initial recipe. A dependency selected by another
resource is checked with a reason, not silently added: buttons/switches require
LEDs; UART requires clock. Prevent removing a required selection. Require at least
one observable output (LEDs or UART initially), so clock alone cannot create a
project that appears to do nothing. Advanced recipes may require UART as well.

## 3. Library, CLI and generated files

Implement generation in a proposed `src/flow/new_project.rs`, exported through
`src/flow/mod.rs`. It owns validation, dependency resolution, deterministic text
generation and publication. It returns a project that has passed `Project::open`;
it does not start tools, launch windows or depend on egui. CLI and GUI pass the
same options and receive the same diagnostics. Put template text/data beside this
module, embedded in the binary, so creation needs no example checkout at runtime.
Put demo copying in a proposed `src/flow/demo_copy.rs` using the same publication policy.

Proposed CLI:

```sh
ecp5_emu new DIR [--name NAME] [--resources clock,leds,buttons,switches,uart]
ecp5_emu new DIR --from-demo ulx3s/uart_hello [--name NAME]
ecp5_emu new DIR --from-demo gallery/riscv_shell [--name NAME]
```

Omitting resources means `clock,leds`; an explicit list replaces that default,
with required dependencies reported. Unknown or unavailable resources fail before
writing. `--from-demo` and `--resources` are mutually exclusive. Name defaults to
the destination leaf. `new` creates only; print the folder and the existing
`ecp5_emu build DIR` / `ecp5_emu run DIR` next steps. No additional build flag.
Extend dispatch/help in `src/command_flow.rs:10–34` and `src/main.rs:23–28`;
today the folder commands are build, run and load, not new.

Generated manifest, with the actual display name escaped as a TOML string:

```toml
[project]
name = "my_counter"
top = "top"

[sources]
verilog = ["top.v"]
constraints = "top.lpf"

[device]
part = "LFE5U-85F"
package = "CABGA381"
```

Use existing keys only. Omit speed, build and memory settings rather than invent
wizard metadata. All keys are optional; the defaults are sorted root-level `*.v`,
exactly one root-level `*.lpf`, and root-level `*.hex`, `*.mem`, `*.vh`, `*.svh`,
`*.init` excluding memory images. Paths may name subfolders, but cannot be absolute
or contain `..` (`src/flow/project.rs:1–24`, `:127–155`, `:188–224`).
Explicit source names make later additions intentional.

`top.v` has a short comment explaining the selected demonstration and LED layout,
then `module top` with only selected ports and synthesizable behaviour. Later
helpers may be modules in this file; separate helper files need manifest entries.
No tool downloads, network access or regeneration of edited projects.

### Template language

Add a language choice to the dialog, proposed CLI `--language
verilog|systemverilog|vhdl`, and creation-script `language` field. Verilog remains
the default `top.v`; SystemVerilog generates `top.sv` using `logic`/`always_ff`;
VHDL generates `top.vhd` with entity/architecture `top` and `numeric_std`.
These are templates of the same top: selected ports, observable behaviour and
`top.lpf` pins stay equivalent. For non-Verilog choices, replace this note's
`top.v` references with the selected filename and write `sources.hdl` as defined
in [HDL languages](hdl-languages.md). SV depends on that note's slice 1; VHDL
selection stays disabled until its slice 2 toolchain/provenance checks pass.

## 4. Checklist: ports, behaviour and pin provenance

Use `src/board.rs` constants for balls, array order, button names and idle polarity;
use the LPFs below for IO settings. A generator resource catalogue refers to those
constants and emits setting-provenance comments. `examples/ulx3s/README.md:36–37`
records the official `ulx3s_v20.lpf` convention followed by these examples.

The initial behaviour is deliberately composable:

| Checked resource | Ports added to `top` | Smallest demonstration to generate |
| --- | --- | --- |
| Clock | `input clk_25mhz` | A shared initialized counter advances on its rising edge; LPF frequency is 25 MHz. Required for the other initial recipes. |
| LEDs | `output [7:0] led` | A 28-bit counter, initially zero, drives `led = count[27:20]`, slow enough to change visibly on the board. In slow single stepping, inspect `count` in Waves; use Run at max to see the divided LEDs move. |
| Buttons | `input [6:0] btn` | Two-register synchronizers initialized to released levels, then XOR with the `BUTTONS[].idle` mask. Any pressed button lights `led[0]`, including active-low PWR. This is a level display, not a press counter needing debounce. |
| DIP switches | `input [3:0] sw` | Two-register synchronizers, initially zero; `sw[3:0]` appears on `led[4:1]`. OFF reads low with the selected DOWN pulls. |
| UART | `output ftdi_rxd`, `input ftdi_txd` | Send `Hello, ULX3S!\r\n` at 115200 8N1 on startup and periodically, using the framing in `uart_hello/top.v`. A synchronized falling edge on RX requests another greeting when the sender is idle; this is an activity check, not a receive/echo implementation. |

There is one assignment to the LED bus: begin with the counter byte, replace bit
0 only for buttons and bits 4:1 only for switches. Remaining bits keep counting.
The UART uses its own output and never assigns LEDs. Every generated input has
a consumer, so synthesis need not remove the very IO being demonstrated.
Counter, synchronizers and UART state have explicit synthesizable initial values,
as the reference tops do (`examples/ulx3s/{blinky_cycle,debounce,uart_hello}/top.v`).

Initial pin/electrical sources follow. “Omitted” means preserve the omission.

| Resource | Ball source in `src/board.rs` | IO settings copied from example LPF |
| --- | --- | --- |
| Clock | `CLOCK = G2`, `CLOCK_PERIOD_NS = 40`, lines 11–13 | `examples/ulx3s/blinky_cycle/top.lpf:1–3`: LVCMOS33, NONE, DRIVE omitted, 25 MHz. |
| LEDs | `LEDS`, lines 14–15, bit 0 first | Same LPF, lines 4–19: LVCMOS33, NONE, DRIVE=4. |
| Buttons | `BUTTONS`, lines 99–144: PWR D6 idle=1; FIRE1 R1, FIRE2 T1, UP R18, DOWN V1, LEFT U1, RIGHT H16 idle=0 | `examples/ulx3s/debounce/top.lpf:4–17`: LVCMOS33, UP for btn[0], DOWN for btn[1..6], DRIVE=4 throughout. |
| Switches | `SWITCHES`, lines 17–23: E8, D8, D7, E7 | `examples/ulx3s/switches/top.lpf:22–29`: LVCMOS33, DOWN, DRIVE omitted. ON connects to supply; OFF floats, as the board constant's comment explains. |
| UART TX / RX | `UART_TX = L4`, `UART_RX = M1`, `BAUD`, lines 146–149 | TX: `examples/ulx3s/uart_hello/top.lpf:4–5`, LVCMOS33, UP, DRIVE=4. RX: `examples/ulx3s/picorv32_console/top.lpf:22–23`, LVCMOS33, UP, DRIVE omitted. FTDI names are from the FTDI side: the FPGA transmits on `ftdi_rxd`. |

Later checklist entries must also produce a working demonstration, not just ports
tied to constants. The following are the decided recipe boundaries and sources;
they remain disabled until implementation and the tests in section 5 pass.
Reuse/adapt the example protocol engines, with one shared UART reporter to avoid
multiple drivers. Generated helper RTL must carry its source/notice attribution.

| Resource | Ports and smallest useful behaviour | Balls and electrical source |
| --- | --- | --- |
| OLED | Outputs `oled_clk`, `oled_mosi`, `oled_dc`, `oled_resn`, `oled_csn`; initialize SSD1331 and repeatedly fill a solid colour. Adapt `examples/ulx3s/oled_pattern/top.v`; require clock. | `OLED_*`, `src/board.rs:42–47`; `examples/ulx3s/oled_pattern/top.lpf:22–31`: LVCMOS33, NONE, DRIVE=4 on all five. |
| HDMI / GPDI | `output [3:0] gpdi_dp`; the 640×480 colour bars from `examples/ulx3s/hdmi_bars/top.v` with its fixed PLL/TMDS helpers. Require clock; do not claim audio or full HDMI protocol support. | `GPDI_DP` / `GPDI_DN`, `src/board.rs:25–30`; `examples/ulx3s/hdmi_bars/top.lpf:4–11`: positive lanes LVCMOS33D, DRIVE=4, PULLMODE omitted. The negative pins are complementary outputs, not independent `gpdi_dn` RTL ports or separately driven LPF entries. |
| SD | Outputs `sd_clk`, `sd_cmd`, output escaped `\sd_d[3]`, input escaped `\sd_d[0]`, as `examples/ulx3s/sd_read/top.v`; initialize SPI mode, read block 0 and print its first bytes or a timeout on UART. Require clock/UART; no writes. | `SD_CLK`, `SD_CMD`, `SD_D`, `src/board.rs:48–53`; `examples/ulx3s/sd_read/top.lpf:22–29`: LVCMOS33, NONE/DRIVE=4 for clock, CMD/MOSI and D3/CS; UP/DRIVE omitted for D0/MISO. D1/D2, `SD_CDN`, `SD_WP` are not emitted: no settings for them are established by this recipe. |
| Audio | `output [3:0] audio_l, audio_r, audio_v`; adapt the bounded four-bit tones of `examples/ulx3s/audio_tone/top.v`, L/R tones and V=8. Require clock. | `AUDIO_L`, `AUDIO_R`, `AUDIO_V`, `src/board.rs:31–41`; `examples/ulx3s/audio_tone/top.lpf:6–29`: LVCMOS33, NONE, DRIVE=4 throughout. |
| SDRAM | Outputs `sdram_clk/cke/csn/wen/rasn/casn`, `[1:0] sdram_ba/dqm`, `[12:0] sdram_a`, `inout [15:0] sdram_d`; adapt `sdram_test/top.v` and `controller.v` to write/read one small region and report pass/errors. Require clock/UART. | All `SDRAM_*`, `src/board.rs:81–97`; `examples/ulx3s/sdram_test/top.lpf:22–99`: LVCMOS33 and DRIVE=4; NONE except data D[15:0] uses UP. |
| ADC | Input `adc_miso`, kept input `[7:0] adc_ain`, outputs `adc_csn/sclk/mosi`; adapt `examples/ulx3s/adc_read/top.v` to read channel 0 and print its code. Require clock/UART. Retain the example's input-only analog/header pads, never drive them. | `ADC_SCLK/CSN/MOSI/MISO` and `ADC_AIN`, `src/board.rs:68–79`; `examples/ulx3s/adc_read/top.lpf:22–45`: LVCMOS33, DRIVE omitted; CS/SCLK/MISO UP, MOSI DOWN, AIN even UP/odd DOWN. Label these weak test pulls in the checklist, not as a neutral external-measurement configuration. |
| RTC | `inout rtc_scl, rtc_sda`; open-drain reader adapted from `examples/ulx3s/rtc_read/{top,rtc_reader}.v`, pointer selection and time-register reads, then print time or NACK. Require clock/UART; no time/SRAM programming. | `RTC_SCL/SDA`, `src/board.rs:62–66`; `examples/ulx3s/rtc_read/top.lpf:6–9`: LVCMOS33, UP, DRIVE omitted. The board comment identifies external pulls and sharing with HDMI DDC; the bars recipe does not use DDC. |
| Flash | Outputs `flash_csn/mosi/wpn/holdn`, input `flash_miso`; read JEDEC ID and print it, adapting `examples/ulx3s/flash_dump/flash_reader.v`. Require clock/UART. Never emit program/erase commands. | `FLASH_*`, `src/board.rs:54–60`; `examples/ulx3s/flash_dump/top.lpf:22–31`: LVCMOS33, UP, DRIVE omitted for those five. `FLASH_SCK=U3` is dedicated CCLK: use `USRMCLK`, as the reader does at line 14, not an ordinary top port/IOBUF. |
| Header pins | Deferred; no selectable ports or electrical defaults yet. A later input-only recipe could display a selected input on an LED after its mapping and pulls are reviewed. | No general gp/gn constants exist in `src/board.rs`. Derive header pad → named net → U1 ball from the KiCad file below; IO_TYPE/PULLMODE/DRIVE need a separately reviewed electrical policy before generation. |

Header mapping must use `assets/ulx3s/ulx3s_v3.0.8.kicad_pcb`, not a guessed
published pinout. Its title block says v3.0.7 (line 14); retain that distinction.
For example J1 pad 5 has net GN0 and pad 6 GP0 (lines 8745–8748), which join U1
C11 and B11 (lines 2723–2724 and 2763–2764). J2 pads 5/6 carry GN14/GP14
(lines 8930–8933), joining U17/U18 (lines 2169–2172), also in `ADC_AIN`.
`tools/ulx3s_3d/kicad_extract.py:56–77,169–224` already parses S-expressions and
exposes footprint references and pad net names; reuse that extraction method for
a reviewed, checked-in catalogue later, not a KiCad/Python runtime dependency.
The generator must reject duplicate ball ownership, including differential mates
and ADC/header overlaps. A net name alone establishes connectivity, not voltage,
direction, pulls or permission to drive it.

Include the full notice from `assets/ulx3s/LICENSE.md` in comment form in generated
`top.lpf`, alongside provenance. The constraints' origin/licence is documented in
`licenses/README.md:18`. Preserve notices for adapted RTL as well; keep generation
self-contained without adding a fourth mandatory project file.

## 5. Verification required for shipping

Create a fresh project through the library, open it with `Project::open`, build
with real Yosys/nextpnr-ecp5/ecppack through `flow::build`, and run the resulting
configuration in the emulator. Template snapshots/parsing alone are insufficient.
Existing real-tool comparisons are in `tests/flow.rs:685–765`; existing configured
emulator setup and clock driving are in `tests/reference_designs.rs:26–123`.
Add a proposed `tests/new_project.rs` using these patterns, with explicit tool/
database requirements and an ignored real-tool group that release validation runs.
Use existing `Tools::from_env` (`src/flow/tools.rs:35–50`).

For the first slice, exercise all legal initial resource combinations through
generation and validation; real-build and emulate at least the default, each
optional resource, and the combined buttons/switches/UART project. Assert:

- The counter and LEDs advance at their specified divider, with no template-only
  fast divider substituted for the acceptance build. Use fast mode for long runs.
- Each button, including PWR, produces the right LED level after synchronization;
  released inputs start correctly, and all four switch bits reach LEDs 4:1.
- UART decodes to the literal greeting at 115200 with a 40 ns clock; RX activity
  requests a new complete greeting, with no corrupt partial frame.
- Ports, LPF sites and decoded pad direction/pulls agree with the catalogue;
  differential negative ownership is checked when GPDI is enabled.
- The generated folder works after relocation, without the source checkout or
  external symlinks; CLI defaults match GUI defaults. A second build can reuse
  the existing cache (`src/flow/cache.rs:40–62`).

Unit tests cover names/paths with spaces, quoting, dependencies, collisions,
invalid resources, publication races and cleanup without overwriting user data.
Later recipes need their observable result checked: OLED fill, decoded GPDI bars,
SD bytes/timeouts, audio transitions, SDRAM pass, ADC code, RTC reply and flash ID.
Use controlled images/model inputs for repeatable peripheral checks, not a lab
board as a test dependency (`src/main.rs:58–78` documents the peripheral options).
No test result is claimed by this design note.

## 6. Start from a demo

**Start from a demo…** has the same name/folder controls and an `ulx3s/name` or
`gallery/name` choice, with description and expected controls. It copies before
opening, never edits the repository example. The project picker already lists
both groups (`gui/src/project/tree.rs:31–69`); the start-screen demo list currently
discovers only ULX3S (`gui/src/start.rs:145–186`). Keep its existing Open/Build
controls for exploring originals, and label the new action as making a copy.

Copy the whole selected demo directory recursively, preserving relative layout:
Verilog, LPFs, includes, `ecp5.toml`, `ecpbram.txt`, firmware source/build scripts,
committed hex/mem/init images, README, local licences, `check.json`, `gui.script`,
`expected/`, `sim/` and `repro/`. Those supporting files are documented in
`examples/gallery/README.md`, “A demo's folder”. Exclude every `build/` directory,
the source manifest's resolved custom build directory, VCS metadata and known
tool caches; do not drop arbitrary `.bin`/`.hex` files as presumed outputs.
Retained demo scripts/docs may refer to repo helpers; copying does not promise to
make their regression harness standalone. The project itself must build alone.

Follow symlinks and write real files/directories, preserving useful executable
bits. Detect cycles, broken links and unsupported special files before publishing.
For this curated catalogue, external link targets must stay within the bundled
example roots; reject missing/unbundled dependencies with their path. For example
`examples/gallery/riscv_shell/picorv32.v` links to
`../../ulx3s/picorv32_console/picorv32.v`; the copy must contain the actual core.
Its manifest explicitly lists it (`examples/gallery/riscv_shell/ecp5.toml:4–7`).
The loader itself permits such links (`src/flow/project.rs:264–266`); copying
them is a deliberate stronger portability rule, not a claim it rejects them.

Change only `[project].name` in an existing manifest; preserve top, sources,
device, build policy and memory images. If none exists, add a manifest containing
the new name and otherwise leave discovery defaults intact. Update a copied
comment that falsely says a source is still a symlink, without removing other
comments. Validate the copy before publication and use the same destination rules
as New project. Demo selection is not mixed with resource generation.

Copy all committed firmware/images, including ones read by `$readmemh` and ones
listed by `[memory].images` or `ecpbram.txt`; keep paths and list order unchanged.
Memory images are distinct from synthesis files (`src/flow/project.rs:188–199,
219–224,271–276`); the build has a separate Swap step
(`src/flow/pipeline.rs:448–475`). No firmware compiler runs during copying.
The committed PicoRV32 hex files avoid needing a RISC-V toolchain just to build
(`examples/ulx3s/README.md:108–127`). Do not copy cached placeholders or bitstreams.
Peripheral runtime images supplied by environment variables are not project
firmware: do not import a user's SD/flash image; explain that external input in
the copied demo's creation summary if the demo requires it.

Preserve source copyright headers and local licence files. Add a plain-text
`THIRD_PARTY_NOTICES.txt` identifying the source demo and copied third-party
material, plus full applicable notices: repository Apache-2.0, ULX3S constraints,
PicoRV32's in-file ISC header and/or SERV's ISC `serv/LICENSE` as applicable.
Use `licenses/README.md:9–18` as the catalogue and the actual shipped texts;
do not relabel a copied core as the student's work. Package these notices and
materialized shared sources with the demo catalogue; installation without examples
must show “Demo sources unavailable”, not fetch unreviewed sources on Create.

## 7. Files inside a project

Proposed `src/flow/project_files.rs` owns validation, file operations and manifest
edits, shared by scripts and UI. Read a fresh manifest, preserving implicit keys;
reopen the project after successful membership changes.
The current tree only opens files (`gui/src/project/tree.rs:22–29,272–294`), and
the editor saves/reverts existing tabs (`gui/src/project/editor.rs:23–47,196–225`).

| Action | File and membership semantics |
| --- | --- |
| New file… | Ask for a relative `.v` path and a valid ordinary Verilog module identifier; default from the basename, validate against keywords and leading digits. Write `module NAME;` / `endmodule` with a brief comment, add to sources, then open it. Never instantiate it in `top` automatically. |
| Add existing… | Select an external file, destination relative path and role: Verilog, auxiliary file or memory image. Copy bytes into the project; never add an absolute path or `..` reference. Re-adding a file already inside only changes membership. A constraint addition is an explicit “Replace active constraints” choice, never a second implicit LPF. |
| Rename… | Rename the on-disk file and every manifest reference to that relative path, preserving list order and role. Update open-tab identity. Filename rename does not rename a Verilog module, instantiate it, or rewrite HDL includes/`$readmemh` references; report that manual step and possible build diagnostics. |
| Remove from project | Remove membership only; keep bytes on disk and close its tab after resolving dirty edits. Do not remove `ecp5.toml`, the active constraint or the last Verilog source; first replace what the loader requires. A removed top module among several sources can still cause a later synthesis error: explain this, do not guess replacements. |
| Save as… | Write the active buffer to a new project-relative file, replace the original's membership in the same role, and switch the tab to it. Keep the original bytes on disk. Materialize defaults to exclude the old file, avoiding duplicate module definitions. This does not rename the module or rewrite HDL references. Disable for the manifest and output/log tabs; an active constraint may be saved as its replacement. |

All destinations must resolve inside the project, including through existing
symlink parents. Refuse overwrite, build-output destinations and mutation of an
external symlink target; offer a real local copy. These protections are new:
`Project::open` currently validates lexical relative paths but allows source links.
Before switching projects or closing/renaming/removing a dirty tab, offer Save,
Discard or Cancel; scripts must specify the choice or fail. Do not silently clear
buffers as the present `open_project` does (`gui/src/project.rs:430–446`).

Manifest rules apply independently to `sources.verilog`, `sources.files`,
`sources.constraints` and `memory.images`:

- An explicit list is edited minimally: append an addition, replace a renamed
  path in place, remove all references for removal, preserve unrelated ordering.
- With defaults, a root-level `.v` or auxiliary file in the recognised extensions
  joins automatically; nothing needs editing. Rename within the same discovery
  class also needs no list edit. Subfolders are not scanned: adding there or
  changing extension requires materializing that role's resolved list first.
- Removal and Save as are exceptions: an implicit file left on disk will be
  rediscovered. Materialize an explicit list of the remaining files before
  removal, explaining “This project's sources are now listed in ecp5.toml”.
  No new exclusions key or change to existing discovery semantics is needed.
- If memory images came from `ecpbram.txt`, materialize `[memory].images` when
  membership changes and update the legacy list too, retaining its comments so
  copied shell scripts are not left with stale image names. Never add an image
  to `sources.files` as well. Refuse an operation that leaves an image also
  discovered as auxiliary input, unless that role is made explicit to exclude it.

Use **`toml_edit` as a new direct dependency**, retaining `toml` for the existing
typed reader (`Cargo.toml:19`, `src/flow/project.rs:183–187`). Do not round-trip
user TOML through `toml` serialization, which loses comments. Patch only affected
items with `toml_edit`, preserving comments, layout and unrelated keys, including
comments attached to arrays. The library needs access to raw text and explicitness;
the current `Project` stores resolved paths, not a comment-preserving document.
This dependency is required from the demo-copy slice onward, before file actions.

Stage file and manifest edits, validate the candidate, recheck external changes,
then replace with rollback on ordinary failure. A multi-file edit is not promised
crash-atomic. Never overwrite a dirty or externally changed manifest buffer:
resolve/save/reload it first and refresh a clean open manifest tab after patching.
Invalidate the previous build/emulator association after membership changes and
show the normal rebuild-needed state. Build currently saves dirty tabs and only
reloads the manifest when it was saved (`gui/src/project.rs:480–496`); update it
to reopen project inputs before every build so default discovery sees new files.

## 8. UI, scripts and records

Start screen: **New project…**, **Start from a demo…**, then the existing Open,
recent designs and exploration demos. Design exposes both creation actions in
plain and project windows. The project tree root/Verilog group context menu has
New file and Add existing; source rows have Rename, Remove from project and Save
as. Save as is also in the editor toolbar and Design's project-only file actions.
Disable mutations during build/load, and on read-only output rows.

Shortcuts use Cmd on macOS, Ctrl elsewhere: Shift+N New project, N New file,
Shift+A Add existing, Shift+S Save as. F2 renames and Delete/Backspace removes
only when a source row owns focus, never while editing text. Demo copying has no
dedicated shortcut. Keep S Save and B Build (`editor.rs:244–249`,
`gui/src/project.rs:1150–1157`). Enter submits a valid dialog; Escape cancels it.
Implement shared dialog state in proposed `gui/src/new_project.rs`; add start
controls and remembered parent in `gui/src/start.rs`, menu dispatch in
`gui/src/app.rs`, and host setup in `gui/src/main.rs`. File actions extend
`gui/src/project.rs` and its `tree.rs`/`editor.rs` children.
Add created/copied folders to Recent projects; make preferences/recents project-aware
in `start.rs` (current `recent_choices`, lines 377–395, requires files).

The current Design action opens `.config`/`.bit` (`gui/src/app.rs:802–899`), and
`gui/src/main.rs:193–248` chooses App versus ProjectApp only at launch. Add a
small proposed `gui/src/window.rs` host to transition to ProjectApp in the same
window after creation. Extend `start::Action`, route embedded App requests back
to the host, and preserve pending scripts, architecture/rate settings and open
trace writers through the transition. Do not restart the process or truncate
records. Reuse project opening/build handlers after resolving dirty buffers.

Every action below needs a proposed script command invoking that same handler.
Creation commands are accepted in **both** `ECP5_GUI_SCRIPT` and
`ECP5_PROJECT_SCRIPT`; file commands in the project script only. Prefix commands
with the existing time/delay field. JSON objects below are proposed argument
syntax, with paths/names as JSON strings, so spaces do not split arguments.

| Action | Proposed command after time/delay | Success record payload |
| --- | --- | --- |
| New project dialog | `new-project {"dir":"DIR","name":"NAME","resources":["clock","leds"],"run":true}` | `new-project dialog NAME resources=clock,leds` |
| Start from demo dialog | `start-demo {"dir":"DIR","name":"NAME","demo":"gallery/riscv_shell","run":true}` | `start-demo dialog gallery/riscv_shell NAME` |
| Change dialog fields/checklist | `project-options {"resources":["clock","leds","buttons"],"run":false}`; same fields as the active dialog | `project-options ok resources=clock,leds,buttons run=false` |
| Submit either dialog | `create-project` | `new-project NAME ok SECONDS` or `copy-demo gallery/riscv_shell NAME ok SECONDS` |
| Cancel either dialog | `cancel-project` | `new-project cancelled` or `copy-demo cancelled` |
| New file | `new-file {"path":"rtl/counter.v","module":"counter"}` | `file new rtl/counter.v ok` |
| Add existing | `add-file {"from":"EXTERNAL","path":"rtl/counter.v","role":"verilog"}` | `file add rtl/counter.v ok` |
| Rename | `rename-file {"path":"old.v","to":"new.v","dirty":"save"}` | `file rename old.v -> new.v ok` |
| Remove | `remove-file {"path":"old.v","dirty":"save"}` | `file remove old.v ok kept-on-disk` |
| Save as | `save-as {"path":"rtl/copy.v"}` (active tab) | `file save-as old.v -> rtl/copy.v ok` |

`project-options` also accepts name/dir/run; demo dialogs accept demo, not resources.
Add-file roles are verilog/auxiliary/memory/constraints; constraints require
`"replace":true`. `dirty` accepts save/discard/cancel, optional only for clean
buffers. Save as writes the buffer without first saving the original.
New/add destinations never overwrite. Include a manifest note such as
`manifest sources.verilog made-explicit ok` when implicit membership becomes
explicit. Emit analogous `failed REASON` payloads for validation/IO failures and
`cancelled` for cancelled file actions, with no success record before publication.
Render displayed paths through the existing `DisplayPaths` in `project.rs` and
`start.rs`; quote ambiguous names/relative paths in notes as JSON strings.

`ECP5_PROJECT_TRACE` writes elapsed seconds then a tab and event; GUI notes get
`# ` from `Trace::note` (`gui/src/project/script.rs:44–73`,
`gui/src/trace.rs:811–815`). Thus creation appears as
`# new-project NAME ok SECONDS` in GUI trace and a timed `new-project ...` event
in project trace. Mirror creation/file outcomes into both when both are active.
Keep existing `# start build <demo> ok|failed <seconds>` for the original demo
Build control (`gui/src/start.rs:40–54`) and project `build ok ...` with error/
warning counts (`gui/src/project.rs:950–957`); copying is not a successful build.

Extend parsers/dispatch in `gui/src/trace.rs::script`, `gui/src/app.rs::scripted`,
`gui/src/project/script.rs::parse_script` and `gui/src/project.rs::poll_script`,
and document them in `gui/src/trace.rs`. JSON payload parsing must preserve `#`
inside quoted strings; today's parsers treat it as a comment. Await creation,
project transition, optional build and emulator readiness before continuing,
using state completion rather than fixed sleeps. Do not replay completed creation
commands when switching script ownership. A window regression creates a project
from the start screen, checks the actual dialog/checklist and editor, builds,
emulates, operates inputs, then exercises copy/file actions as those slices land;
assert both records and disk contents. Include cancellation and occupied-folder
cases, and a commented manifest whose unrelated text stays unchanged.

## 9. Scope and delivery order

First slice: shared generator, `ecp5_emu new`, and New project in the window with
clock/LEDs/buttons/switches/UART, plus real-tool and window acceptance checks.
This closes the immediate gap: create a personal project and see it run without
manual pin lookup. Demo copying alone leaves “make my own counter” dependent on
a template browser. The window transition and visible failure handling belong
here; a CLI-only generator does not answer the missing New button.

Then deliver, in order:

1. **Start from a demo**, including materialized shared cores, memory images,
   licence notices, bundled catalogue availability and `toml_edit` name patching.
2. **Project file operations**, with comment-preserving membership edits, dirty
   buffer handling, default-discovery exceptions and matching script coverage.
3. **Advanced resource recipes**, one working/tested resource at a time; start
   with OLED/audio, then GPDI, then memory and read-only peripheral protocols.
   Keep demo copying available while these remain disabled.
4. **Header inputs**, only after a KiCad-derived mapping, bank/electrical review
   and shared-pin conflict coverage. General header outputs need a later design.

There is no pin planner GUI, arbitrary FPGA/board selector, IP catalogue, testbench
generator, firmware IDE or dependency downloader. Only the LFE5U-85F ULX3S is
offered: `Project::emulatable` currently accepts that part with CABGA381 only
(`src/flow/project.rs:317–326`). No general PLL wizard in these slices. A fixed
PLL helper may be embedded in the later GPDI recipe; an `ecppll` clock-generation
action belongs later, with clock constraints and emulator tests. The existing
`blinky_pll` example already documents `ecppll` 25→100 MHz
(`examples/ulx3s/README.md:11`). Flash programming remains out of this flow.

<a id="10-owner-questions"></a>

## 10. Open design questions

1. Is `~/ecp5-projects` with a remembered last parent a suitable initial folder?
2. Should Build and run stay checked initially, or should Create always stop at
   the source? Recommendation: checked, with clear progress and retained files.
3. Is the new direct `toml_edit` dependency acceptable for preserving user TOML?
   Recommendation: yes; require it before demo-name and file-membership editing.
4. Are clock/LEDs as the only initial checks right, or should buttons/UART also
   start checked? Recommendation: keep the default counter short and opt in.
5. Is deferring generic header selection acceptable while the existing board
   peripherals and Start from a demo cover the first teaching exercises?
