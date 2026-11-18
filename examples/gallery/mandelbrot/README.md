# mandelbrot: a hardware Mandelbrot explorer

The Mandelbrot set drawn by the ECP5's DSP multipliers, panned and zoomed from the board's
buttons or the UART, and printed as ASCII art. There is no CPU: a three-multiplier pipeline
evaluates one `z ← z² + c` iteration per clock, and a small state machine interleaves pixel
computations with the UART transmission.

The lesson is fixed-point arithmetic on real DSP hardware. Every value is a signed 18-bit
Q3.14 number — one sign bit, three integer bits, fourteen fractional bits — because 18 bits
is the widest operand one MULT18X18D takes, and the iteration needs the range [−8, 8): the
interesting area spans about −2.25…+0.75 and the escape test is |z|² > 4. Three integer bits
cover both, fourteen fractional bits are the most precision the multiplier leaves. That
choice also sets the zoom limit: a pixel is `width/64`, and when the width itself reaches
2⁻⁸ the pixel step is 2⁻¹⁴, the smallest step Q3.14 can express. The design refuses to zoom
further and says `ZOOM LIMIT`.

## How it draws

One view is a header line, 24 rows × 64 columns of art and a footer:

```text
VIEW 0x00000000 X 0xFFFFD000 Y 0x00000000 W 0x0000C000 ITER 0x20 Q14
                                    ........:.......
                                 ...........:+-=......
                              ............::+@@:........
                            .............:-@@@@@-........       
                          ..........::::::-@@@@*-::....-...     
                       ...........:=@@-+@@@@@@@@@@@-:+-=-...    
                    .............::-+@@@@@@@@@@@@@@@@@@#....    
               .....::..........:-@@@@@@@@@@@@@@@@@@@@@@:....   
          ..........:=#-:-=:-:::-@@@@@@@@@@@@@@@@@@@@@@@@-....  
       .............::+@@@@@@@--@@@@@@@@@@@@@@@@@@@@@@@@@+....  
      ...........:::-=@@@@@@@@@*@@@@@@@@@@@@@@@@@@@@@@@@@.....  
     .....:...:::-@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@:....... 
     .....:...:::-@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@:....... 
      ...........:::-=@@@@@@@@@*@@@@@@@@@@@@@@@@@@@@@@@@@.....  
       .............::+@@@@@@@--@@@@@@@@@@@@@@@@@@@@@@@@@+....  
          ..........:=%-:-=:-:::-@@@@@@@@@@@@@@@@@@@@@@@@-....  
               .....::..........:-@@@@@@@@@@@@@@@@@@@@@@:....   
                    .............::-+@@@@@@@@@@@@@@@@@@#....    
                       ...........:=@@-+@@@@@@@@@@@-:+-=-...    
                          ..........::::::-@@@@*-::....-...     
                            .............:-@@@@@-........       
                              ............::+@@:........        
                                 ...........:+-=......          
                                    ........:.......            
CYCLES 0x000043F3 SUM 0x000043F3
```

Numbers are printed in hex: the Q3.14 coordinates read directly off the bits (0xFFFFD000 is
−0.75, 0x0000C000 is 3.0). The palette maps the escape iteration to nine bands
` .:-=+*#%`, scaled to the current `max_iter`, with `@` for pixels that never escaped.

The engine computes one pixel per character, in the gap while the UART sends the previous
pixel's character. A pixel takes at most `max_iter` clocks (≤ 128); a character takes 2 170.
The engine always outruns the UART, so a line buffer would buy nothing — the UART baud rate,
not the multipliers, sets the frame time (about 0.15 s on the board).

`CYCLES` counts the clocks the iterator actually ran and `SUM` the total of all iteration
counts; every iteration increments both, so the two are equal. Both are deterministic: the
same view always prints the same footer.

## Controls

| Button | UART key | Action |
|---|---|---|
| UP / DOWN / LEFT / RIGHT | `w` `s` / `a` `d` | pan by 1/8 of the width |
| FIRE1 | `+` | zoom in ×2 |
| FIRE2 | `-` | zoom out ×2 |
| — | `i` | cycle max_iter 16 / 32 / 64 / 128 |
| — | `t` | tour: 8 views zooming into the seahorse valley (−0.7435 + 0.1314i) |
| — | `r` | restart at the power-on view; prints `KEY r` |

Keys wait in a 256-byte FIFO ([io.v](io.v)) and are acted on in order, between frames; a key
arriving mid-frame takes effect on the next frame. Each key prints its `KEY <c>` line before
the frame it causes. Buttons are debounced for about 10 ms and inject the same key bytes as
the UART.

The LEDs show the iteration count of the pixel being computed, so they shimmer while a view
is drawn.

## Resources

nextpnr for the LFE5U-85F, seed 1:
- 2 161 LUT4 and 484 TRELLIS_FF;
- 3 MULT18X18D — the three products `x²`, `y²`, `xy`, inferred from `*`;
- no block RAM, no ALU54B.

Fmax is 65.70 MHz for the 25 MHz clock. The three multipliers sit together on the left side
of the die, in one DSP row (X5/X8/X9 at Y34); the die map in the window lights exactly those
tiles.

## Build and check

```sh
examples/ulx3s/build.sh examples/gallery/mandelbrot target/gallery/mandelbrot
python3 examples/gallery/check.py mandelbrot --build --emulator
```

[check.json](check.json) sends `r+d++wit`: restart, zoom in, pan right, zoom in twice, pan
up, more iterations, then the tour. The expected text
([expected/uart.txt](expected/uart.txt), 26 KB, 16 views) comes from a Verilator simulation
of the RTL: [sim/run.sh](sim/run.sh). The emulator prints the same bytes in fast mode, at
2.58 MHz on the Mac (the check's 62.9 million clocks in 24.4 s of wall time).

## In the emulator's window

```sh
ECP5_GUI_SCRIPT=examples/gallery/mandelbrot/gui.script \
  target/release/ecp5_gui target/gallery/mandelbrot/top.config
```

[gui.script](gui.script) does the following:
- zooms in with FIRE1;
- pans right and up with the d-pad;
- zooms in again;
- types `t` for the 8-view tour.

Buttons are held for a second: the design debounces them for 10 ms of board time, which the window
takes a while to run.

The window's record of one run (`ECP5_GUI_TRACE`, summed up by [gui_run.py](../gui_run.py)):
- 70 s at 60 frames per second (median frame 16.7 ms);
- 2.78 MHz;
- 428 drawings of the 3D board read back from the GPU, none with an LED drawn wrong;
- 42 different LED patterns, the iteration counts of the pixels.

The record's `# uart` lines hold the console's 22 KB: the views the buttons asked for, then the
tour. 10 of its views have the same position, width and `ITER` as views of the board-checked
[expected/uart.txt](expected/uart.txt), and all 10 are identical to them: the art and the
`CYCLES`/`SUM` footer.

## On the board

```sh
openFPGALoader -b ulx3s target/gallery/mandelbrot/top.bit   # SRAM only
```

Then press the buttons or type the keys at 115200 baud. `python3 examples/gallery/check.py
mandelbrot --board` loads the bitstream three times, types `r+d++wit` each time, and compares
4 s of output.

**Board:** on 2026-11-16, 3 of 3 loads printed the expected 24 567 bytes from `KEY r` on: all 16
views with their `CYCLES`/`SUM` footers, the tour included. Records are in
[docs/inventory/LFE5U-85F/hr03_gallery](../../../docs/inventory/LFE5U-85F/hr03_gallery/README.md).
