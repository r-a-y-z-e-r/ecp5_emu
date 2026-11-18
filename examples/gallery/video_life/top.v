// Conway's Game of Life on an HDMI monitor: a 64 x 48 torus (3 072 cells) in
// block RAM, each cell a 10 x 10 pixel square of a 640 x 480 DVI picture.
// The sweep of ebr_life computes one generation into the other world in about
// 3 500 clocks, inside the vertical blank of one 420 000-clock frame, so the
// engine is idle 99% of each frame and a frame never shows half of two
// generations. Every frame's pixels also go through a parallel CRC-32, which
// the design prints once a frame; the emulator's receiver decodes the TMDS
// pads and must find the same CRC for the same frame number.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
           output ftdi_rxd, output [7:0] led, output [3:0] gpdi_dp);
  // One PLL: 125 MHz shift clock for the TMDS lanes, 25 MHz pixel clock for
  // everything else, including the UART (217 clocks per bit at 115200 baud).
  // The LOCK output is not used, as in hdmi_moving: the logic resets itself
  // with a power-on counter once the pixel clock runs.
  wire shift_clk, pixel_clk, locked;
  pll clocks(.clkin(clk_25mhz), .clkout0(shift_clk), .clkout1(pixel_clk), .locked(locked));
  wire clk = pixel_clk;
  reg [5:0] reset_count = 0;
  wire reset = !(&reset_count);
  always @(posedge clk) if (reset) reset_count <= reset_count + 1;

  // The exact video timing of hdmi_moving: 800 x 525 total, syncs active low.
  reg [9:0] x = 0, y = 0;
  always @(posedge clk)
    if (x == 799) begin
      x <= 0;
      if (y == 524) y <= 0; else y <= y + 1;
    end else x <= x + 1;
  wire active = x < 640 && y < 480;
  wire frame_sync = x == 0 && y == 480; // first clock of the vertical blank

  // Engine states and registers (declared before the memories that use them).
  localparam IDLE = 0, KEY_WAIT = 1, LOAD_ADDR = 2, LOAD_WAIT = 3,
    LOAD_CAPTURE = 4, LOAD_BITS = 5, LOAD_WRITE = 6, FETCH_N = 7, WAIT_N = 8,
    CAP_N = 9, WAIT_M = 10, CAP_M = 11, WAIT_S = 12, CAP_S = 13, CELLS = 14,
    WRITE_ROW = 15, SWAP = 16, MARKER = 17, PRINT_LINE = 18;
  reg [4:0] state = LOAD_ADDR;
  reg [5:0] row = 0, column = 0;
  reg bank = 0;
  reg [63:0] north = 0, middle = 0, south = 0, result = 0;
  reg [127:0] amid = 0, aresult = 0;
  reg [11:0] population = 0, sum = 0;
  reg [15:0] generation = 0;
  reg [15:0] number = 0; // the frame number, as in hdmi_moving
  reg paused = 0, fill_soup = 0;
  // Generations in a row whose population equaled the one before. After 32
  // the world is still lifes and blinkers, and a new soup is sown.
  reg [5:0] still = 0;
  reg [31:0] lfsr = 32'h1aceb00c;
  reg [2:0] speed = 2; // frames per generation: N = 1 << speed, default 4
  reg [3:0] phase = 0;
  reg [7:0] command = 0;
  reg [4:0] pos = 0;
  reg [6:0] pending_buttons = 0;
  reg restart_pending = 0, line_kill = 0;

  // Two worlds and a 2-bit age per cell, all in block RAM. Each memory has
  // one read port and one write port (PDPW16KD): the engine and the display
  // share the read address, and they never use it at the same time, because
  // the engine only runs in the vertical blank (lines 480 to about 485) and
  // the display only fetches in horizontal blanks of active lines, plus once
  // at line 520.
  (* ram_style="block" *) reg [63:0] world0[0:47];
  (* ram_style="block" *) reg [63:0] world1[0:47];
  (* ram_style="block" *) reg [127:0] age0[0:47]; // 2 bits per cell, 64 cells
  (* ram_style="block" *) reg [127:0] age1[0:47];
  (* rom_style="block" *) reg [63:0] seed[0:47];
  integer i;
  initial begin
    for (i = 0; i < 48; i = i + 1) begin world0[i] = 0; world1[i] = 0; age0[i] = 0; age1[i] = 0; end
    $readmemh("seed.hex", seed);
  end
  reg [5:0] address = 0;
  reg [63:0] q0 = 0, q1 = 0, qs = 0;
  reg [127:0] a0 = 0, a1 = 0;
  wire [63:0] source = bank ? q1 : q0;
  wire [127:0] age_source = bank ? a1 : a0;
  // No read of a world in a clock that writes it: an EBR row read in the
  // clock its other port writes it returns undefined data on the chip, and
  // the emulator makes it unknown, so the read registers hold while writing.
  // No state captures a read made in such a clock.
  wire writing = state == WRITE_ROW || state == LOAD_WRITE;
  always @(posedge clk) begin
    if (!writing) begin
      q0 <= world0[address]; q1 <= world1[address];
      a0 <= age0[address]; a1 <= age1[address];
    end
    qs <= seed[address];
  end
  always @(posedge clk)
    if (writing) begin
      if (state == LOAD_WRITE || bank) begin world0[row] <= result; age0[row] <= aresult; end
      else begin world1[row] <= result; age1[row] <= aresult; end
    end

  // The display. The row word is fetched three clocks in the horizontal
  // blank (address, wait, capture) and shifted out one cell per 10 pixels;
  // it is refetched every line, because a line of shifts destroys it.
  // Nothing is fetched at line 479: the world may swap at line 480, and a
  // row fetched before that would be from the old world. Line 520, after any
  // swap, fetches row 0 for the next frame.
  reg [63:0] wshift = 0;
  reg [127:0] ashift = 0;
  reg [3:0] pix = 0;    // pixel within its 10-pixel cell
  reg [3:0] cline = 0;  // line within its 10-line cell row
  reg [5:0] crow = 0;   // cell row of the current line
  wire display_window = y < 479 || y >= 520;
  wire [5:0] next_cell_row = cline == 9 ? (crow == 47 ? 6'd0 : crow + 6'd1) : crow;
  wire display_load = display_window && x == 700;
  wire [5:0] display_row = y >= 520 ? 6'd0 : next_cell_row;
  always @(posedge clk) begin
    if (display_window && x == 702) begin
      wshift <= source; ashift <= age_source; pix <= 0;
    end else if (active) begin
      if (pix == 9) begin
        pix <= 0;
        wshift <= {1'b0, wshift[63:1]};
        ashift <= {2'b00, ashift[127:2]};
      end else pix <= pix + 1;
    end else pix <= 0;
    if (x == 799 && y < 480) begin
      if (cline == 9) begin cline <= 0; crow <= crow == 47 ? 6'd0 : crow + 6'd1; end
      else cline <= cline + 1;
    end
  end
  // Live cells are drawn 9 x 9 on a 1-pixel dark grid; colour tells the age.
  wire grid = pix == 9 || cline == 9;
  reg [23:0] rgb;
  always @* begin
    if (grid) rgb = 24'h000000;            // the dark grid line
    else if (!wshift[0]) rgb = 24'h0a0a0e; // a dead cell is near-black
    else case (ashift[1:0])
      0: rgb = 24'hd8f040;                 // newborn: bright yellow-green
      1: rgb = 24'h40c860;                 // green
      2: rgb = 24'h2880d0;                 // blue
      default: rgb = 24'h9840c8;           // old: violet
    endcase
  end
  video_out video(.shift_clk(shift_clk), .pixel_clk(clk), .rgb(rgb),
    .de(active), .hs(!(x >= 656 && x < 752)),
    .vs(!(y >= 490 && y < 492)), .gpdi_dp(gpdi_dp));

  // Reflected CRC-32/ISO-HDLC: R, then G, then B; bits LSB first in each
  // byte. Copied from examples/ulx3s/hdmi_moving/top.v, so the design's CRC
  // is computed exactly as the emulator's receiver computes it (zlib crc32).
  function [31:0] crc_pixel;
    input [31:0] state;
    input [23:0] color;
    reg [31:0] c;
    reg [7:0] byte_value;
    integer channel, bit_index;
    begin
      c = state;
      for (channel = 2; channel >= 0; channel = channel - 1) begin
        byte_value = color >> (8 * channel);
        for (bit_index = 0; bit_index < 8; bit_index = bit_index + 1)
          c = (c >> 1) ^ ((c[0] ^ byte_value[bit_index]) ? 32'hedb88320 : 0);
      end
      crc_pixel = c;
    end
  endfunction
  reg [31:0] crc = 32'hffffffff;
  reg [31:0] report_crc = 0;
  reg [15:0] report_number = 0, report_gen = 0, report_pop = 0;
  wire [31:0] next_crc = crc_pixel(crc, rgb);
  always @(posedge clk) begin
    if (active) crc <= next_crc;
    if (x == 799 && y == 524) crc <= 32'hffffffff;
    if (x == 639 && y == 479) begin
      report_crc <= ~next_crc;
      report_number <= number;
      report_gen <= generation;
      report_pop <= {4'b0000, population};
    end
  end

  // UART and buttons, shared with the other gallery demos. Received bytes
  // wait in a 256-byte FIFO; one is acted on per frame boundary, in order,
  // so the output depends on what arrives, never on when.
  reg send = 0, take = 0;
  reg [7:0] data = 0;
  wire busy, available;
  wire [7:0] key;
  wire [6:0] pressed;
  gallery_io io(clk, reset, ftdi_txd, ftdi_rxd, btn, pressed,
                send, data, busy, key, available, take);

  wire [3:0] nframes = 4'b1 << speed;
  wire random_bit = lfsr[0];
  wire [3:0] neighbours = {3'b0, north[63]} + {3'b0, north[0]} + {3'b0, north[1]} +
    {3'b0, middle[63]} + {3'b0, middle[1]} +
    {3'b0, south[63]} + {3'b0, south[0]} + {3'b0, south[1]};
  wire born = (neighbours == 3) || ((neighbours == 2) && middle[0]);
  // A newborn cell is 0; an old one saturates at 3; a dead cell has no age.
  wire [1:0] new_age = !born ? 2'd0 : !middle[0] ? 2'd0 :
    amid[1:0] == 3 ? 2'd3 : amid[1:0] + 2'd1;
  wire fill_bit = fill_soup ? random_bit : middle[0];

  function [7:0] hex;
    input [3:0] nibble;
    begin hex = nibble < 10 ? "0" + nibble : "A" + nibble - 10; end
  endfunction
  // F<frame> G<generation> P<population> C<CRC-32 of the frame's pixels>
  function [7:0] line_char;
    input [4:0] p;
    begin
      case (p)
        0: line_char = "F";
        1: line_char = hex(report_number[15:12]);
        2: line_char = hex(report_number[11:8]);
        3: line_char = hex(report_number[7:4]);
        4: line_char = hex(report_number[3:0]);
        5: line_char = " ";
        6: line_char = "G";
        7: line_char = hex(report_gen[15:12]);
        8: line_char = hex(report_gen[11:8]);
        9: line_char = hex(report_gen[7:4]);
        10: line_char = hex(report_gen[3:0]);
        11: line_char = " ";
        12: line_char = "P";
        13: line_char = hex(report_pop[15:12]);
        14: line_char = hex(report_pop[11:8]);
        15: line_char = hex(report_pop[7:4]);
        16: line_char = hex(report_pop[3:0]);
        17: line_char = " ";
        18: line_char = "C";
        19: line_char = hex(report_crc[31:28]);
        20: line_char = hex(report_crc[27:24]);
        21: line_char = hex(report_crc[23:20]);
        22: line_char = hex(report_crc[19:16]);
        23: line_char = hex(report_crc[15:12]);
        24: line_char = hex(report_crc[11:8]);
        25: line_char = hex(report_crc[7:4]);
        26: line_char = hex(report_crc[3:0]);
        27: line_char = 8'h0d;
        default: line_char = 8'h0a;
      endcase
    end
  endfunction
  function [7:0] restart_char;
    input [4:0] p;
    begin
      case (p)
        0: restart_char = "R"; 1: restart_char = "E"; 2: restart_char = "S";
        3: restart_char = "T"; 4: restart_char = "A"; 5: restart_char = "R";
        6: restart_char = "T"; default: restart_char = 8'h0a;
      endcase
    end
  endfunction

  // The engine: one cell per clock, three row shift registers, torus wrap by
  // rotation; ebr_life's sweep adapted to 48 rows. A sweep or a load starts
  // at frame_sync and finishes within the vertical blank (about 3 600
  // clocks, near line 485), so the worlds only ever swap in the blank.
  always @(posedge clk) begin
    send <= 0; take <= 0;
    // Button pulses are latched until a frame boundary takes them.
    pending_buttons <= pending_buttons | (pressed & 7'b1111110);
    if (display_load) address <= display_row;
    if (reset) begin state <= LOAD_ADDR; row <= 0; sum <= 0; end
    else case (state)
      IDLE:
        if (x == 799 && y == 524) begin
          // The end of the raster: print the frame's line (29 characters
          // take about 63 000 clocks, so it is done long before the next
          // frame boundary). A restart cancels the line and renumbers.
          if (line_kill) begin line_kill <= 0; number <= 0; end
          else begin number <= number + 1; pos <= 0; state <= PRINT_LINE; end
        end else if (frame_sync) begin
          // One frame boundary, one command, in order. A pending restart
          // goes first: its marker has been printed by now.
          if (restart_pending) begin
            restart_pending <= 0; fill_soup <= 0; row <= 0; sum <= 0;
            paused <= 0; speed <= 2; phase <= 0; still <= 0;
            lfsr <= 32'h1aceb00c; generation <= 0; line_kill <= 1;
            pending_buttons <= 0; state <= LOAD_ADDR;
          end else if (available && !take) begin
            command <= key; take <= 1; state <= KEY_WAIT;
          end else if (pending_buttons[1]) begin
            command <= "s"; pending_buttons[1] <= 0; state <= KEY_WAIT;
          end else if (pending_buttons[2]) begin
            command <= "p"; pending_buttons[2] <= 0; state <= KEY_WAIT;
          end else if (pending_buttons[3]) begin
            command <= "+"; pending_buttons[3] <= 0; state <= KEY_WAIT;
          end else if (pending_buttons[4]) begin
            command <= "-"; pending_buttons[4] <= 0; state <= KEY_WAIT;
          end else if (pending_buttons[5] || pending_buttons[6]) begin
            command <= "n"; pending_buttons[6:5] <= 0; state <= KEY_WAIT;
          end else if (!paused && still == 32) begin
            command <= "s"; still <= 0; state <= KEY_WAIT;
          end else if (!paused) begin
            if (phase == 0) begin
              phase <= nframes - 1; row <= 0; sum <= 0; state <= FETCH_N;
            end else phase <= phase - 1;
          end
        end
      KEY_WAIT: begin
        state <= IDLE;
        case (command)
          "r": // Finish the current byte, print the marker, then restart at
               // the next frame boundary, frame counter included.
            if (busy || send) state <= KEY_WAIT;
            else begin
              pos <= 0; state <= MARKER;
              line_kill <= 1; restart_pending <= 1;
            end
          "s", "g": begin
            fill_soup <= command == "s"; still <= 0; phase <= 0;
            row <= 0; sum <= 0; state <= LOAD_ADDR;
          end
          "p": paused <= !paused;
          "n": if (paused) begin row <= 0; sum <= 0; state <= FETCH_N; end
          "+": if (speed != 0) begin speed <= speed - 1; phase <= 0; end
          "-": if (speed != 4) begin speed <= speed + 1; phase <= 0; end
          default: ;
        endcase
      end
      // Loading a world (start pattern or LFSR soup) into world0.
      LOAD_ADDR: begin address <= row; state <= LOAD_WAIT; end
      LOAD_WAIT: state <= LOAD_CAPTURE;
      LOAD_CAPTURE: begin
        middle <= qs; column <= 0; result <= 0; aresult <= 0; state <= LOAD_BITS;
      end
      LOAD_BITS: begin
        result <= {fill_bit, result[63:1]};
        middle <= {middle[0], middle[63:1]};
        lfsr <= {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
        sum <= sum + fill_bit;
        if (column == 63) state <= LOAD_WRITE; else column <= column + 1;
      end
      LOAD_WRITE:
        if (row == 47) begin
          population <= sum; bank <= 0; generation <= 0; still <= 0;
          state <= IDLE;
        end else begin row <= row + 1; state <= LOAD_ADDR; end
      // One generation: rows above, at, below; 64 cells; write the row.
      FETCH_N: begin address <= row == 0 ? 6'd47 : row - 6'd1; state <= WAIT_N; end
      WAIT_N: state <= CAP_N;
      CAP_N: begin north <= source; address <= row; state <= WAIT_M; end
      WAIT_M: state <= CAP_M;
      CAP_M: begin
        middle <= source; amid <= age_source;
        address <= row == 47 ? 6'd0 : row + 6'd1; state <= WAIT_S;
      end
      WAIT_S: state <= CAP_S;
      CAP_S: begin
        south <= source; column <= 0; result <= 0; aresult <= 0; state <= CELLS;
      end
      CELLS: begin
        result <= {born, result[63:1]};
        aresult <= {new_age, aresult[127:2]};
        sum <= sum + born;
        north <= {north[0], north[63:1]};
        middle <= {middle[0], middle[63:1]};
        south <= {south[0], south[63:1]};
        amid <= {amid[1:0], amid[127:2]};
        if (column == 63) state <= WRITE_ROW; else column <= column + 1;
      end
      WRITE_ROW:
        if (row == 47) state <= SWAP;
        else begin row <= row + 1; state <= FETCH_N; end
      SWAP: begin
        bank <= !bank; population <= sum; generation <= generation + 1;
        still <= sum != population ? 6'd0 : still == 32 ? still : still + 1'b1;
        state <= IDLE;
      end
      MARKER: if (!busy && !send) begin
        data <= restart_char(pos); send <= 1;
        if (pos == 7) begin pos <= 0; state <= IDLE; end else pos <= pos + 1;
      end
      PRINT_LINE: if (!busy && !send) begin
        data <= line_char(pos); send <= 1;
        if (pos == 28) begin pos <= 0; state <= IDLE; end else pos <= pos + 1;
      end
      default: state <= IDLE;
    endcase
  end

  // population / 16, saturated (3 072 cells reach 192, so it never clips).
  assign led = population > 12'd4080 ? 8'hff : population[11:4];
endmodule
