# ULX3S 3D model — QA

Model: `out/ulx3s_3d/ulx3s_v3.0.8.blend`, built by `tools/ulx3s_3d/` from the ULX3S
v3.0.8 KiCad board (git tag `v3.0.8`; the title block still says v3.0.7, the bottom
silkscreen says v3.0.8). Renders: Cycles, 192 samples, light-tent lighting.
The updated `photo`, `top` and `options` views were rendered on `rserv` with
Blender 5.1.2 and a Radeon RX 9070 via HIP. The updated windowless GUI pictures
are `gui_board_3d.jpg` and `gui_options_3d.jpg`.

| File | View |
|---|---|
| `render_top.jpg` | top, orthographic |
| `render_photo.jpg` | the viewpoint of the radiona.org/ulx3s photo (camera solved from the photo) |
| `render_bottom.jpg` | bottom, orthographic, mirrored left-right like the board's bottom photo |
| `render_angle_bottom.jpg` | bottom, perspective |
| `compare_top.jpg`, `compare_bottom.jpg` | the v3.0.3 board photos from the ULX3S repository above the orthographic renders |

The radiona.org photo has no stated licence, so the side-by-side with it
(`compare_radiona_photo.png`, `zoom_*.png`) is written to `out/ulx3s_3d/renders/` only
(`qa_sheets.py --photo <screenshot>`).

## How it was checked

- Camera: solved from 7 landmarks in the radiona.org photo (the four M3 holes and three
  board corners), principal point fixed at the frame centre: 1.8 px RMS in a 2000 px
  frame (about 0.1 mm on the board). With that camera every part of the render lands
  on its counterpart in the photo (`compare_radiona_photo.png`): buttons, U1, U2, SW1,
  connectors, LEDs, header rings, silkscreen.
- Colours: median sRGB at ~3000 projected board points per class, photo vs render,
  final render — mask over bare laminate (77,163,113) vs (84,160,112), mask over
  copper (47,132,92) vs (67,132,92), silkscreen (180,199,185) vs (181,186,180), ENIG
  (195,180,151) vs (177,167,146).
- Bottom: layout matches the v3.0.3 bottom photo part for part (ESP32 antenna past the
  edge, CR1225 holder, U10 flash, U6 FT231X, Y2, KONČAR/inem logos).
- LEDs: lighting D0..D7 through their `LED_D<n>` materials works
  (`out/ulx3s_3d/renders/leds_on.png`).
- The saved .blend opens in a fresh `blender -b --factory-startup`: one scene,
  textures packed, units mm.

## Still different from the photos

1. **DIP switch** shows amber tape over a light top with dark slots and white sliders,
   without the printed ON/CTS/1-4.
2. **Board green** follows the radiona.org photo; the v3.0.3 top photo is darker and
   bluer. Mask over copper keeps a slightly redder tint than the photo (67 vs 47 red).
3. **ENIG pads** come out a little paler and cooler than on the photos.
4. **Markings**: text only (no Micron logo, no small lines under the FPGA marking).
   The FPGA reads 6BG381I as on the photo, while the KiCad value is LFE5U-85F-6BG381**C** —
   which grade is on the actual board is not checked.
5. **Small components**: passives retain simple box bodies and flat solder wedges;
   LEDs have no visible die. IC solder joints remain unmodelled.
6. **Back side**: ESP32 can text is approximate; the CR1225 holder is a simplified clip
   with no battery; the small ICs carry no markings.
7. The **microSD card** (`microSD_card`, not part of the board) is pushed all the way in,
   flush with the socket mouth; the radiona.org photo shows it half out (6.4 mm past
   the mouth, `SD_CARD_OUT` in `build_blend.py`). The dark strip seen past the board
   edge under SD1 is the ESP32 antenna on the bottom side (6.3 mm overhang), which the
   half-inserted card hid on the photos.

## Connector/header rebuild checked (2026-11-16)

The model was rebuilt with the solder fillets and the connected GPDI1 mounting
tabs; windowless and Cycles renders of US1/US2, GPDI1, AUDIO1, SD1, the card and
the J1/J2 headers were checked. The rebuilt GLB is now
in `out/ulx3s_3d/`, and all nine board tests pass.
Dimensions and mechanical references are in `tools/ulx3s_3d/README.md`.

Final measured index-accessor triangle counts, including fillets and connected tabs:

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

GLB size: 10,775,636 → 11,507,244 bytes. The final model remains below the
limits of 181,000 triangles and 12,400,000 bytes.

The model includes one eight-triangle `Tin` prism per two-terminal chip
termination (60% of pad width, up to 0.6 mm long, one third of part height).
The current extracted board has 166 fitted chips, so 332 wedges add 2,656
triangles, below the 6,000-triangle cap. IC leads receive none. GPDI1's separate
posts are replaced by continuous bent 0.3 mm shell tabs, about 1.0 mm wide,
rooted inside the side walls and centred on the existing four drills; tips end
0.8 mm below the board bottom.

Totals after the fillets: **174,036 triangles (+10.6%)**, **11,507,244 bytes (+6.8%)**.

Release build and `cargo clippy -p ecp5_gui --all-targets -- -D warnings` pass.
`cargo test -p ecp5_gui --release board`: 9 passed, including the rebuilt real
model and header visibility/picking tests. Rust formatting and Python compilation
checks pass.

## In the GUI

`gui_board_3d.jpg`, `gui_board_top.jpg`, `gui_board_bottom.jpg`: the GUI board pane's
own renderer (`ecp5_gui --board-picture`, wgpu, the same code the window runs),
drawn without a window — LEDs D0, D2, D3, D5, D7 lit and B1 pressed in the 3D
one. These show the look only; what the window draws while a design runs is
checked from its own record (`ECP5_GUI_TRACE`, `# board` lines, see
`gui/src/trace.rs`).

## OLED module, HDMI cable and GPIO headers (options)

What the board shows when the emulated design drives the OLED header or HDMI
(`BoardView::set_oled` / `set_hdmi`), with optional J1/J2 headers controlled by
`BoardView::set_headers`. The updated `options.jpg` and `gui_options_3d.jpg`
show all three options: OLED, HDMI and headers.

| File | View |
|---|---|
| `gui_options_3d.jpg` | Updated windowless GUI renderer, photo view: the SSD1331 module on OLED1 with the built-in 96x64 test picture, the HDMI cable in GPDI1 and J1/J2 headers |
| `gui_options_top.jpg` | Earlier top view with the OLED module and HDMI cable |
| `options.jpg` | Updated Cycles `Cam_Photo` with OLED, HDMI and J1/J2 headers fitted (the glass is dark: Cycles has no live picture) |

Checked (2026-11-16, model rebuilt from scratch, `cargo test -p ecp5_gui --release board`:
9 passed): the module's pins are matched to OLED1's pads by net (GND, +3V3, OLED_CLK,
OLED_MOSI, OLED_RES, OLED_DC, OLED_CS), which turns the module 180 degrees, its body
over U1; the exported screen's UV (0,0) is at the GND end of the header edge and v grows
away from the header, active area 20.14 x 13.42 mm. So a picture sent pins-up reads
upside down in the board's normal view, as it would on the real board until the
design flips it with the SSD1331 remap. Pixels are sampled nearest (crisp). The plain
picture (no options) is unchanged.

Not a model error: C59 and C62 next to the ADC exist in v3.0.8 but not on the v3.0.3
photo. The J1/J2 headers are 2x20 each (GPIO pairs 0-13 and 14-27 plus power), not 2x28.
