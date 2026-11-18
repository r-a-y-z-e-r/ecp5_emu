// Original beam-racing effects. See README.md for the integer definition.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
           output ftdi_rxd, output [7:0] led, output [3:0] gpdi_dp);
  wire shift_clk, pixel_clk, locked;
  pll clocks(.clkin(clk_25mhz), .clkout0(shift_clk),
             .clkout1(pixel_clk), .locked(locked));
  wire clk = pixel_clk;
  reg [5:0] reset_count = 0;
  wire reset = !(&reset_count);
  always @(posedge clk) if (reset) reset_count <= reset_count + 1'b1;

  // Same raster and physical output path as video_life / hdmi_moving.
  reg [9:0] x = 0, y = 0;
  always @(posedge clk)
    if (x == 799) begin
      x <= 0;
      y <= y == 524 ? 10'd0 : y + 1'b1;
    end else x <= x + 1'b1;
  wire frame_sync = x == 0 && y == 480;
  reg [15:0] number = 0, tick = 0;
  reg effect = 0, paused = 0;
  reg [1:0] palette = 0, speed = 1;
  wire [15:0] step = 16'd1 << speed;

  wire [23:0] rgb;
  wire de, hs, vs, last_pixel, raster_end;
  beam_effect render(.clk(clk), .x(x), .y(y), .tick(tick),
    .effect(effect), .palette(palette), .rgb(rgb), .de(de), .hs(hs),
    .vs(vs), .last_pixel(last_pixel), .raster_end(raster_end));
  video_out video(.shift_clk(shift_clk), .pixel_clk(clk), .rgb(rgb),
    .de(de), .hs(hs), .vs(vs), .gpdi_dp(gpdi_dp));

  // CRC-32/ISO-HDLC of the exact RGB/de presented to the TMDS encoders.
  // R, G, B; least significant bit first in each byte; initial/final XOR.
  function [31:0] crc_pixel;
    input [31:0] state;
    input [23:0] color;
    reg [31:0] c;
    reg [7:0] b;
    integer ch, k;
    begin
      c = state;
      for (ch = 2; ch >= 0; ch = ch - 1) begin
        b = color[8*ch +: 8];
        for (k = 0; k < 8; k = k + 1)
          c = (c >> 1) ^ ((c[0] ^ b[k]) ? 32'hedb88320 : 32'd0);
      end
      crc_pixel = c;
    end
  endfunction
  reg [31:0] crc = 32'hffffffff, report_crc = 0;
  reg [15:0] report_number = 0;
  reg report_effect = 0;
  reg [1:0] report_palette = 0;
  wire [31:0] next_crc = crc_pixel(crc, rgb);
  always @(posedge clk) begin
    if (de) crc <= next_crc;
    if (raster_end) crc <= 32'hffffffff;
    if (last_pixel) begin
      report_crc <= ~next_crc;
      report_number <= number;
      report_effect <= effect;
      report_palette <= palette;
    end
  end

  reg send = 0, take = 0;
  reg [7:0] data = 0;
  wire busy, available;
  wire [7:0] key;
  wire [6:0] pressed;
  gallery_io io(clk, reset, ftdi_txd, ftdi_rxd, btn, pressed,
                send, data, busy, key, available, take);
  reg [6:0] pending_buttons = 0;
  wire [6:0] waiting_buttons = pending_buttons | (pressed & 7'h7e);
  reg [7:0] command;
  reg [6:0] consumed_button;
  always @* begin
    command = 0; consumed_button = 0;
    if (available) command = key;
    else if (waiting_buttons[1]) begin command = "e"; consumed_button = 7'h02; end
    else if (waiting_buttons[2]) begin command = "p"; consumed_button = 7'h04; end
    else if (waiting_buttons[3]) begin command = "+"; consumed_button = 7'h08; end
    else if (waiting_buttons[4]) begin command = "-"; consumed_button = 7'h10; end
    else if (waiting_buttons[5]) begin command = "["; consumed_button = 7'h20; end
    else if (waiting_buttons[6]) begin command = "]"; consumed_button = 7'h40; end
  end

  localparam IDLE = 0, FRAME = 1, EVENT = 2, MARKER = 3;
  reg [1:0] tx_state = IDLE;
  reg [4:0] pos = 0;
  reg event_pending = 0, marker_pending = 0;
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? "0" + {4'd0,n} : "A" + {4'd0,n} - 8'd10; end
  endfunction
  // F<hex4> E<0/1> P<0/1/2> C<hex8> CR LF. Compatible with gui_run.py.
  function [7:0] frame_char;
    input [4:0] p;
    begin
      case (p)
        0: frame_char = "F";
        1: frame_char = hex(report_number[15:12]);
        2: frame_char = hex(report_number[11:8]);
        3: frame_char = hex(report_number[7:4]);
        4: frame_char = hex(report_number[3:0]);
        5, 8, 11: frame_char = " ";
        6: frame_char = "E";
        7: frame_char = "0" + {7'd0, report_effect};
        9: frame_char = "P";
        10: frame_char = "0" + {6'd0, report_palette};
        12: frame_char = "C";
        13: frame_char = hex(report_crc[31:28]);
        14: frame_char = hex(report_crc[27:24]);
        15: frame_char = hex(report_crc[23:20]);
        16: frame_char = hex(report_crc[19:16]);
        17: frame_char = hex(report_crc[15:12]);
        18: frame_char = hex(report_crc[11:8]);
        19: frame_char = hex(report_crc[7:4]);
        20: frame_char = hex(report_crc[3:0]);
        21: frame_char = 13;
        default: frame_char = 10;
      endcase
    end
  endfunction
  // Event describes the next picture's settings; tick advances using the
  // old speed/pause before the command is applied. No periodic event spam.
  function [7:0] event_char;
    input [4:0] p;
    begin
      case (p)
        0: event_char = "S"; 1: event_char = "E"; 2: event_char = "T";
        3, 6, 9, 12: event_char = " ";
        4: event_char = "E"; 5: event_char = "0" + {7'd0, effect};
        7: event_char = "P"; 8: event_char = "0" + {6'd0, palette};
        10: event_char = "S"; 11: event_char = "0" + {6'd0, speed};
        13: event_char = "H"; 14: event_char = "0" + {7'd0, paused};
        15: event_char = 13;
        default: event_char = 10;
      endcase
    end
  endfunction
  function [7:0] marker_char;
    input [4:0] p;
    begin
      case (p)
        0: marker_char = "R"; 1: marker_char = "E"; 2: marker_char = "S";
        3: marker_char = "T"; 4: marker_char = "A"; 5: marker_char = "R";
        6: marker_char = "T"; 7: marker_char = 13;
        default: marker_char = 10;
      endcase
    end
  endfunction
  always @(posedge clk) begin
    send <= 0; take <= 0;
    pending_buttons <= waiting_buttons;
    if (frame_sync) begin
      // The last active pixel was captured 152 clocks earlier. Snapshot
      // remains immutable for the whole UART transaction, across blanking.
      tx_state <= FRAME; pos <= 0;
      event_pending <= 0; marker_pending <= 0;
      number <= number + 1'b1;
      if (!paused) tick <= tick + step;
      pending_buttons <= waiting_buttons & ~consumed_button;
      if (available) take <= 1;
      case (command)
        "r": begin
          number <= 0; tick <= 0; effect <= 0; palette <= 0;
          speed <= 1; paused <= 0; pending_buttons <= 0;
          marker_pending <= 1; // only an explicit r can emit RESTART
        end
        "e": begin effect <= !effect; event_pending <= 1; end
        "[": begin palette <= palette == 0 ? 2'd2 : palette - 1'b1; event_pending <= 1; end
        "]": begin palette <= palette == 2 ? 2'd0 : palette + 1'b1; event_pending <= 1; end
        "+": if (speed != 3) begin speed <= speed + 1'b1; event_pending <= 1; end
        "-": if (speed != 0) begin speed <= speed - 1'b1; event_pending <= 1; end
        "p": begin paused <= !paused; event_pending <= 1; end
        default: ;
      endcase
    end else if (!busy && !send) begin
      case (tx_state)
        FRAME: begin
          data <= frame_char(pos); send <= 1;
          if (pos == 22) begin
            pos <= 0;
            tx_state <= marker_pending ? MARKER : event_pending ? EVENT : IDLE;
          end else pos <= pos + 1'b1;
        end
        EVENT: begin
          data <= event_char(pos); send <= 1;
          if (pos == 16) begin pos <= 0; tx_state <= IDLE; end
          else pos <= pos + 1'b1;
        end
        MARKER: begin
          data <= marker_char(pos); send <= 1;
          if (pos == 8) begin pos <= 0; tx_state <= IDLE; end
          else pos <= pos + 1'b1;
        end
        default: ;
      endcase
    end
  end
  // bit 7 pause; bits 6:5 speed; bits 4:3 palette; bit 2 effect;
  // bits 1:0 slow animation heartbeat. No switch inputs exist.
  assign led = {paused, speed, palette, effect, tick[7:6]};
endmodule
