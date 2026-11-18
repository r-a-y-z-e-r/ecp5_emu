// The DVI pipeline of hdmi_bars, with scrolling bars and a bouncing square.
// Every active RGB byte also enters a parallel CRC, independently of TMDS.
module top(input clk_25mhz, output [3:0] gpdi_dp, output ftdi_rxd, output [7:0] led);
  wire shift_clk, pixel_clk, locked;
  pll clocks(.clkin(clk_25mhz), .clkout0(shift_clk), .clkout1(pixel_clk), .locked(locked));
  reg [9:0] x = 0, y = 0;
  reg [15:0] number = 0;
  reg [9:0] scroll = 0;
  reg [9:0] sx = 0, sy = 0;
  reg dx = 0, dy = 0;
  always @(posedge pixel_clk)
    if (x == 799) begin
      x <= 0;
      if (y == 524) begin
        y <= 0;
        number <= number + 1;
        scroll <= scroll == 639 ? 0 : scroll + 1;
        if (sx == 575) dx <= 1;
        else if (sx == 1) dx <= 0;
        if (sy == 415) dy <= 1;
        else if (sy == 1) dy <= 0;
        sx <= dx ? sx - 1 : sx + 1;
        sy <= dy ? sy - 1 : sy + 1;
      end else y <= y + 1;
    end else x <= x + 1;
  wire [10:0] sum = {1'b0,x} + {1'b0,scroll};
  wire [9:0] bx = sum >= 640 ? sum - 640 : sum;
  reg [23:0] rgb;
  always @* begin
    if      (bx < 80)  rgb = 24'hffffff;
    else if (bx < 160) rgb = 24'hffff00;
    else if (bx < 240) rgb = 24'h00ffff;
    else if (bx < 320) rgb = 24'h00ff00;
    else if (bx < 400) rgb = 24'hff00ff;
    else if (bx < 480) rgb = 24'hff0000;
    else if (bx < 560) rgb = 24'h0000ff;
    else              rgb = 24'h000000;
    if (x >= sx && x < sx + 64 && y >= sy && y < sy + 64) rgb = 24'h4080c0;
  end
  wire active = x < 640 && y < 480;
  video_out video(.shift_clk(shift_clk), .pixel_clk(pixel_clk), .rgb(rgb),
    .de(active), .hs(!(x >= 656 && x < 752)),
    .vs(!(y >= 490 && y < 492)), .gpdi_dp(gpdi_dp));

  // Reflected CRC-32/ISO-HDLC: R, then G, then B; bits LSB first in each byte.
  function [31:0] crc_pixel;
    input [31:0] state;
    input [23:0] color;
    reg [31:0] c;
    reg [7:0] byte_value;
    integer channel, bit_index;
    begin
      c = state;
      for (channel=2; channel>=0; channel=channel-1) begin
        byte_value = color >> (8*channel);
        for (bit_index=0; bit_index<8; bit_index=bit_index+1)
          c = (c >> 1) ^ ((c[0] ^ byte_value[bit_index]) ? 32'hedb88320 : 0);
      end
      crc_pixel = c;
    end
  endfunction
  reg [31:0] crc = 32'hffffffff;
  reg [31:0] report_crc = 0;
  reg [15:0] report_number = 0;
  reg pending = 0;
  wire [31:0] next_crc = crc_pixel(crc, rgb);
  always @(posedge pixel_clk) begin
    if (active) crc <= next_crc;
    if (x == 799 && y == 524) crc <= 32'hffffffff;
    if (x == 639 && y == 479) begin
      report_crc <= ~next_crc;
      report_number <= number;
    end
  end
  function [7:0] hex;
    input [3:0] nibble;
    begin hex = nibble < 10 ? "0" + nibble : "A" + nibble - 10; end
  endfunction
  reg [4:0] index = 0;
  reg [7:0] char;
  always @* begin
    case (index)
      0: char = "F";
      1: char = hex(report_number[15:12]);
      2: char = hex(report_number[11:8]);
      3: char = hex(report_number[7:4]);
      4: char = hex(report_number[3:0]);
      5: char = " ";
      6: char = "C";
      7: char = hex(report_crc[31:28]);
      8: char = hex(report_crc[27:24]);
      9: char = hex(report_crc[23:20]);
      10: char = hex(report_crc[19:16]);
      11: char = hex(report_crc[15:12]);
      12: char = hex(report_crc[11:8]);
      13: char = hex(report_crc[7:4]);
      14: char = hex(report_crc[3:0]);
      15: char = 8'h0d;
      default: char = 8'h0a;
    endcase
  end
  reg [9:0] uart_frame = 10'h3ff;
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  always @(posedge pixel_clk) begin
    // Publish at the end of the raster; the CRC was latched at active end.
    if (x == 799 && y == 524) begin pending <= 1; index <= 0; end
    if (bits_left != 0) begin
      if (baud == 216) begin
        baud <= 0;
        uart_frame <= {1'b1, uart_frame[9:1]};
        bits_left <= bits_left - 1;
      end else baud <= baud + 1;
    end else if (pending) begin
      uart_frame <= {1'b1,char,1'b0};
      bits_left <= 10;
      if (index == 16) pending <= 0;
      else index <= index + 1;
    end
  end
  assign ftdi_rxd = uart_frame[0];
  assign led = number[7:0];
endmodule
