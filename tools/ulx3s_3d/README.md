# ULX3S 3D board model

A Blender model of the ULX3S v3.0.8 (LFE5U-85F) built from the board's own KiCad
design, for the virtual board in the GUI: `ecp5_gui` draws it left of the die, as tall as the window
(`gui/src/board_model.rs`, `gui/src/board_view.rs`) from the glTF export, with
the design's LEDs lit and its buttons pressed on the model.

## Build

```bash
python3 tools/ulx3s_3d/kicad_extract.py          # assets/ulx3s/*.kicad_pcb -> out/ulx3s_3d/board.json
python3 tools/ulx3s_3d/layers.py                 # kicad-cli + rsvg-convert -> out/ulx3s_3d/textures/
blender -b -P tools/ulx3s_3d/build_blend.py      # -> out/ulx3s_3d/ulx3s_v3.0.8.blend (~7 s)
blender -b out/ulx3s_3d/ulx3s_v3.0.8.blend -P tools/ulx3s_3d/export_glb.py
                                                 # -> out/ulx3s_3d/ulx3s_v3.0.8.glb, read by the GUI
```

The GUI looks for the .glb there (or at `$ECP5_BOARD_MODEL`); without it the
board pane says how to build the model and shows the LEDs and buttons as plain rows.
`ecp5_gui --board-picture FILE.png [--board-view 3d|top|bottom] [--board-lit BITS]`
draws the GUI's board without a window.

Run the glTF export headless: exporting from a live Blender session through the
MCP add-on crashed Blender 5.1.2 once (2026-11-11).

Renders and QA sheets: run `render_qa.py` inside Blender on the `ULX3S` scene
(`out/ulx3s_3d/renders/`), then `python3 tools/ulx3s_3d/qa_sheets.py` (`docs/qa/ulx3s_3d/`).
The Blender scripts also run from a live session (exec the file with `__file__`
set) and rebuild only what they made before; run the glTF export headless.

Needs: KiCad 10 `kicad-cli` (PATH, `$KICAD_CLI` or found via Spotlight), Homebrew
`rsvg-convert`, Pillow + numpy, Blender 5.1.

## What comes from where

| Data | Source |
|---|---|
| Board outline (93.98 x 50.80 mm), 1.6 mm thickness | Edge.Cuts and `general/thickness` of the PCB |
| Part positions, rotations, sides, pads, Fab/courtyard outlines | footprints of the PCB |
| Pad net names (`net`, null when absent) | each pad's KiCad `(net N NAME)` |
| Through holes (108) and mounting holes | pad drills of the PCB, cut into the board mesh |
| Copper, solder mask, silkscreen (both sides, incl. the EMARD/RADIONA/FER logos) | `kicad-cli pcb export svg` of F/B.Cu, F/B.Mask, F/B.SilkS, 40 px/mm |
| Via drills (509) | vias of the PCB, painted into the texture |
| Part bodies | datasheet dimensions in `build_blend.py` (one builder per package) |
| Colours | fitted to the radiona.org photo: the camera was solved from 7 landmarks (1.8 px RMS), then mask/ENIG albedos adjusted until median colours at ~3000 projected board points matched |

## Scene conventions (for the GUI)

- 1 unit = 1 mm; board centre at the origin; top copper at z = +0.8, bottom at -0.8;
  Blender X = KiCad x - 141.09, Blender Y = 86.82 - KiCad y.
- One object per fitted part, named by reference designator (`U1`, `U2`, `U9`, `D0`..`D7`,
  `B0`..`B6`, `SW1`, `GPDI1`, `US1`, `US2`, `AUDIO1`, `SD1`, `Y1`, `BAT1`, ...). Origin
  at the footprint origin on the board surface; bottom parts are turned 180 deg about X.
- Custom properties on each: `ref`, `value`, `footprint`, `side`, `kicad_x`, `kicad_y`,
  `kicad_rot`, `populated`.
- LEDs D0..D7: `led_index` (= `led[i]` in `src/board.rs`), `led_color`, `led_material`.
  Each LED lens is its own material `LED_<ref>` with Emission Strength 0; raise it to
  light the LED (colour already set from the KiCad value: RED/ORANGE/GREEN/BLUE).
- Buttons B0..B6: `btn_index` / `btn_name` (= `btn[i]` in `src/board.rs`: PWR, FIRE1,
  FIRE2, UP, DOWN, LEFT, RIGHT); the moving cap is the child `B<n>_plunger`.
- DIP switch: children `SW1_slider1`..`SW1_slider4`, with `sw_index` and
  `sw_travel` extras for the live levers (see below).
- Unfitted through-hole connectors (`J1`, `J2` 2x20 GPIO; `J3` WIFI_OFF jumper; `J4` 2x3
  JTAG; `J5` VJ1 voltage select; `OLED1` 7-pin SSD1331 header)
  are objects made of their plated rings, with a `pins` property: JSON
  `[[pad, x, y], ...]` in the object's frame.
- `microSD_card` (collection `ULX3S_Accessories`) is a card pushed fully into SD1
  (`SD_CARD_OUT` in `build_blend.py` sets how far it sticks out; property `card_out_mm`);
  it is not part of the board.
- Cameras: `Cam_Top`, `Cam_Bottom` (orthographic, bottom mirrored left-right like the
  board photo), `Cam_Photo` (the radiona.org photo's viewpoint, 2000 x 838 frame),
  `Cam_Angle_Bottom`. The ground is a shadow catcher; `render_qa.py` composites on white.

## Live SW1 levers

Levers start at OFF. `SW1_slider4`..`SW1_slider1` carry `sw_index` 0..3,
matched to row nets `SW1`..`SW4`: physical switches 1..4, left to right.
A 2.6 mm slot and 0.9 mm lever give 1.7 mm travel.

`FP` negates local KiCad y, placement rotates into board Blender coordinates,
and export maps `(x, y, z)` to `(x, z, -y)`. Each `sw_travel` is already in
board glTF coordinates: `(0, 0, -1.7)` mm, from the bottom of the slot (OFF)
toward the board's top edge and smaller KiCad y (ON).

The view API is separate from `BoardInputs`:

```rust
pub fn set_switches(&mut self, used: [bool; 4], on: [bool; 4]);
pub fn take_switch_clicks(&mut self) -> Vec<usize>;
```

Call `set_switches` before `show()`: `used[i]` enables clicks and `on[i]`
alone sets the displayed position; defaults are unused and OFF. Used-lever
clicks queue indices 0..3 in click order, drained by `take_switch_clicks`
without changing lever state. Drags turn the board; double clicks on levers
do not reset the view.

`ECP5_BOARD_PICTURE_SWITCHES=1010` sets switches 1 and 3 (leftmost and third)
ON in windowless pictures; characters run from switch 1 to 4. Unset or invalid
values leave all four OFF.

The model was rebuilt and checked: GLB 11,507,464 bytes (previously 11,507,244), 174,036 triangles (unchanged); 12 board tests passed, clippy and rustfmt clean; windowless top views confirmed all OFF and switches 1/3 ON with `1010`.

## Connector geometry and dimensions

All bodies are made in `build_blend.py`, without downloaded meshes, textures or
fonts. The existing marking font is local. KiCad pads, Fab outlines and
`conn_frame` still determine every connector's origin, rotation and mouth plane.
The visual references are the repository's
[`ULX3S_v303_top.png`](../../assets/ulx3s/ULX3S_v303_top.png) and
[`ULX3S_v303_bottom.png`](../../assets/ulx3s/ULX3S_v303_bottom.png).
The dimensions below are the modelling targets; small stamped details are
representations of those photos, rather than a manufacturer CAD model.

| Part / mechanical reference | Geometry (mm) |
|---|---|
| US1/US2, FCI 10118192-0001LF ([drawing 10118192, Rev B](https://cdn.amphenol-cs.com/media/wysiwyg/files/drawing/10118192.pdf)) | 7.5 × 5.0 × 2.6 shell; 0.25 sheet; open 6.9 × 1.85 micro-B mouth with 0.75 lower chamfers. Full-depth inner walls, 5.4 × 0.6 black tongue, top 0.25 below the roof; five 0.25 contacts at 0.65 pitch underneath. Two recessed 1.0 × 0.8 latch tabs, white rear insulator, five pad-aligned tails and four mounting flanges. |
| GPDI1, FCI 10029449-111RLF ([drawing 10029449](https://cdn.amphenol-cs.com/media/wysiwyg/files/drawing/10029449.pdf)) | 15.0 × 9.6 × 5.8 envelope; mouth remains 1.2 past the board edge. 0.3 sheet and entry lip; open 14.0 × 4.55 mouth narrows to 10.6 at the bottom with 45° corners. Centred 10.0 × 1.6 tongue, 19 contacts (10 top, 9 bottom) at 1.0 pitch, offset 0.5. Actual roof windows with bent springs, rear insulator, staggered lead origins routed to all 19 pads, four continuous bent 0.3-sheet tabs rooted in the side walls, about 1.0 wide through the mounting holes, ending 0.8 below the board bottom. |
| AUDIO1, CUI SJ-43516-SMT ([SJ-4351X-SMT series reference](https://www.sameskydevices.com/product/resource/sj-4351x-smt.pdf)) | Photo-era 6.0 × 14.5 × 5.0 body, 0.2 top recess and 0.3 bottom chamfer; Ø5.0 × 2.4 nozzle, Ø3.6 bore 3.0 deep, 0.4 collar. Six silver pad-aligned tabs, folded silver rear strip near D0, two original locating pegs. The series was reengineered in 2022; the newer drawing does not replace the older photo's housing. |
| SD1, Alps SCHD3A0100 footprint | Existing 12.3 × 16.8 layout and black frame, continuous 0.2 sheet U on the sides/back, 1.8 high; bent side solder tabs. Front 30% strip has rounded entry corners and a real latch slot; eight 0.6 gold strips at 1.1 pitch rise and curl at their tips. Tails return from the rear frame to the footprint's internal contact pad row. Construction follows the board photos; pad positions follow the extracted KiCad footprint. |
| `microSD_card` | 11.0 × 15.0 × 0.8 outline, narrowed on one side to 9.7 over the rear 6.4; 0.6 corner radii, existing text and insertion setting. |

Connectors remain one mesh per original reference; the card remains a separate
accessory. Exposed seams and moulded edges use 0.05–0.2 bevels/rounding.
Two-terminal resistors, capacitors, inductors, LEDs and SOD-323 diodes have one
small `Tin` wedge at each termination. Each wedge is an unbevelled triangular
prism (8 triangles), 60% of the pad width, up to 0.6 long, and one third of the
part height. SOD-323 fillets start at the existing flat leads' outer ends.
The current extracted board has 166 fitted two-terminal chips: 332 wedges add
2,656 triangles, below the 6,000-triangle fillet cap. IC leads receive no fillets.

## Optional OLED, HDMI and GPIO headers

`ULX3S_Options` is a child collection of `ULX3S`, included in the GLB export.
Its mesh objects, `OLED_module`, `HDMI_plug`, `J1_header` and `J2_header`,
have `hide_render = True` in the saved scene: these options are absent on the board as sold. Each has
`ref`, `value`, and `option` (`oled` / `hdmi` / `headers`) custom properties
exported as glTF extras.
The `options` QA view uses `Cam_Photo`, temporarily shows all three options
(OLED, HDMI and J1/J2 headers), and restores their visibility even if rendering
fails. Existing cameras and board objects are unchanged.

The OLED assembly includes a seven-way female header on OLED1 (2.54 mm pitch,
8.5 mm tall), 0.64 mm square gold male pins, and a 27.7 × 30.1 × 1.6 mm blue
PCB, its underside 11 mm above the board top. Its four 2.2 mm holes are 2 mm
from the corners. Black glass is 25.9 × 21.6 × 1 mm, starting 4.5 mm from the
header edge; the 20.14 × 13.42 mm active face starts 6.5 mm from that edge,
0.02 mm above the glass. A folded orange ribbon and dark bonded foot finish
the opposite edge. Materials are `OLED_pcb`, `OLED_glass`, `OLED_screen`,
`OLED_flex`, `Plastic_black`, and `ENIG`.

Held display-side up with the header edge at the top, the module's pins are
GND VCC SCL SDA RES DC CS, left to right. The builder matches these against
OLED1's nets GND +3V3 OLED_CLK OLED_MOSI OLED_RES OLED_DC OLED_CS and asserts
that every pin lands at the expected 2.54 mm pitch. In v3.0.8 these are pads
1–7; GND is at board x=+6.35 mm, CS at x=−8.89 mm. Thus the module is turned
180° relative to the board's text and extends toward the top edge, over U1.

`OLED_screen` is a separate black glossy face. Its **exported glTF** UV (0,0)
is the GND-side corner nearest the header; U grows toward CS and V away from
the header. Image row zero is at V=0. Blender UVs use V=1 at that edge because
the exporter flips V. The real-model Rust test checks the exported corners
against OLED1's extracted GND net and header position, and checks the active size.

The HDMI type-A shell has a 13.9 × 4.45 mm cross-section, inserted 7 mm into
GPDI1's mouth (the same front plane as `build_hdmi`, 1.6 mm past the Fab
outline). Its black body is 21 × 11 mm across and 28 mm long. The 6.5 mm round
cable is a bevelled Bezier curve converted to mesh, about 80 mm long, bending
down and away until its underside rests at the board model's lowest Z.
`cable_end` stores its endpoint in the object's Blender-local frame; the GLB
reader converts it to board coordinates for the label.

The GUI contract is `BoardView::set_oled(Option<BoardScreen>)` and
`set_hdmi(Option<BoardHdmi>)`, independent of `BoardInputs`. `BoardScreen`
contains width, height, row-major RGBA8 sRGB pixels in `Arc<[u8]>`, and a `u64`
serial; change the serial when pixels change. The GPU uploads only when serial
or size changes, uses nearest sampling, and draws the pixels as emitted light
plus glass reflection. `BoardHdmi.mode` supplies the label and hover text.
None hides the OLED/HDMI options and excludes them from picking.
`BoardView::set_headers(bool)` independently shows both GPIO headers, default
false. Object IDs 8/9/10 (OLED/HDMI/headers) and their children's inherited IDs
control visibility and picking; image selector 3 is OLED.
Options do not change the camera's fit to the board.

`J1_header` and `J2_header` are straight 2×20 male headers at the J1/J2
pad drill centres: 2.54 pitch, 2.5 high black plastic, 0.64 square gold pins
extending 6.0 above the plastic and 3.0 below the board bottom. Each has
underside `Tin` solder fillets, `ref` J1/J2, `value` "2x20 pin header" and
`option` "headers". The original plated-ring meshes `J1`/`J2` stay present.

Windowless checks use the same GPU drawing path:

```bash
ECP5_BOARD_PICTURE_OLED=test ECP5_BOARD_PICTURE_HDMI="640x480 60 Hz" ECP5_BOARD_PICTURE_HEADERS=1 \
  target/release/ecp5_gui --board-picture out/ulx3s_3d/gui/options_3d.png
ECP5_BOARD_PICTURE_OLED=test ECP5_BOARD_PICTURE_HDMI="640x480 60 Hz" ECP5_BOARD_PICTURE_HEADERS=1 \
  target/release/ecp5_gui --board-picture out/ulx3s_3d/gui/options_top.png --board-view top
target/release/ecp5_gui --board-picture out/ulx3s_3d/gui/plain.png
ECP5_BOARD_PICTURE_HEADERS=1 \
  target/release/ecp5_gui --board-picture out/ulx3s_3d/gui/look_headers.png
```

`test` is a built-in 96×64 picture: colour bars in the top half, a one-pixel
white border, and ULX3S in a 5×7 font in the lower half (2× pixel scale).
The text is upright held pins-up, so it should read upside down in the board's
top view. This is the physical mounting orientation; the SSD1331 remap in the
design can flip its own output. The HDMI label is painted by egui in the live
pane, near the cable end or GPDI1 if the end is off-screen; windowless PNGs
contain only the 3D geometry.

## Connector rebuild measurements (2026-11-16)

The model was rebuilt with the solder fillets and the connected GPDI1 mounting
tabs; windowless and Cycles renders of US1/US2, GPDI1, AUDIO1, SD1, the card and
the J1/J2 headers were checked. The rebuilt GLB is present
in `out/ulx3s_3d/`; all nine board tests pass. Final index-accessor triangle counts:

| Object | Before | Final rebuild |
|---|---:|---:|
| US1 | 252 | 1,724 |
| US2 | 252 | 1,724 |
| GPDI1 | 932 | 2,312 |
| AUDIO1 | 824 | 2,148 |
| SD1 | 672 | 1,884 |
| microSD_card | 2,144 | 2,176 |
| J1 / J2 plated rings, each | 13,120 | 13,120 |
| J1_header / J2_header, each | 0 | 3,600 |
| All objects, including options | 157,288 | 174,036 |

GLB size: 10,775,636 → 11,507,244 bytes. The final model includes 332 solder
wedges (2,656 triangles) on 166 chips and remains below the limits of 181,000
triangles and 12,400,000 bytes.

Totals after the fillets: **174,036 triangles (+10.6%)**, **11,507,244 bytes (+6.8%)**.

Cycles QA views `photo`, `top` and `options` were rendered on `rserv` using
Blender 5.1.2 and a Radeon RX 9070 via HIP. The updated `render_photo.jpg`,
`render_top.jpg` and `options.jpg` are in `docs/qa/ulx3s_3d/`; `options` shows
OLED, HDMI and the J1/J2 headers. The updated windowless GUI pictures there are
`gui_board_3d.jpg` and `gui_options_3d.jpg`, with all three options in the latter.
