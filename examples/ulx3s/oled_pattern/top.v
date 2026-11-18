// SSD1331 colour pattern, 6.25 MHz mode-0 SPI in the 25 MHz domain.
// Each frame is followed by Oxxxx Cxxxxxxxx on the 115200 8N1 UART.
module top(input clk_25mhz, output reg oled_clk = 0,
  output reg oled_mosi = 0, output reg oled_dc = 0,
  output reg oled_resn = 0, output reg oled_csn = 1,
  output ftdi_rxd, output [7:0] led);
  function [7:0] init_byte;
    input [5:0] index;
    begin
      case (index)
        0: init_byte = 8'hae;
        1: init_byte = 8'ha0;
        2: init_byte = 8'h72;
        3: init_byte = 8'ha1;
        4: init_byte = 8'h00;
        5: init_byte = 8'ha2;
        6: init_byte = 8'h00;
        7: init_byte = 8'ha4;
        8: init_byte = 8'ha8;
        9: init_byte = 8'h3f;
        10: init_byte = 8'had;
        11: init_byte = 8'h8e;
        12: init_byte = 8'hb0;
        13: init_byte = 8'h0b;
        14: init_byte = 8'hb1;
        15: init_byte = 8'h31;
        16: init_byte = 8'hb3;
        17: init_byte = 8'hf0;
        18: init_byte = 8'h8a;
        19: init_byte = 8'h64;
        20: init_byte = 8'h8b;
        21: init_byte = 8'h78;
        22: init_byte = 8'h8c;
        23: init_byte = 8'h64;
        24: init_byte = 8'hbb;
        25: init_byte = 8'h3a;
        26: init_byte = 8'hbe;
        27: init_byte = 8'h3e;
        28: init_byte = 8'h87;
        29: init_byte = 8'h06;
        30: init_byte = 8'h81;
        31: init_byte = 8'h91;
        32: init_byte = 8'h82;
        33: init_byte = 8'h50;
        34: init_byte = 8'h83;
        35: init_byte = 8'h7d;
        36: init_byte = 8'haf;
        default: init_byte = 8'haf;
      endcase
    end
  endfunction
  // Reflected CRC-32/ISO-HDLC; each byte is folded into parallel fabric.
  function [31:0] crc_byte;
    input [31:0] crc;
    input [7:0] data;
    reg [31:0] c;
    integer i;
    begin
      c = crc ^ {24'b0, data};
      for (i = 0; i < 8; i = i + 1)
        if (c[0]) c = (c >> 1) ^ 32'hedb88320;
        else c = c >> 1;
      crc_byte = c;
    end
  endfunction
  function [7:0] hex;
    input [3:0] nibble;
    begin
      if (nibble < 10) hex = "0" + nibble;
      else hex = "A" + nibble - 10;
    end
  endfunction

  reg spi_start = 0, spi_busy = 0, spi_done = 0;
  reg [7:0] spi_data = 0, spi_shift = 0;
  reg spi_dc = 0;
  reg [1:0] spi_phase = 0;
  reg [2:0] spi_bit = 0;
  always @(posedge clk_25mhz) begin
    spi_done <= 0;
    if (spi_start && !spi_busy) begin
      spi_busy <= 1;
      oled_csn <= 0;
      oled_dc <= spi_dc;
      oled_clk <= 0;
      oled_mosi <= spi_data[7];
      spi_shift <= spi_data;
      spi_bit <= 7;
      spi_phase <= 0;
    end else if (spi_busy) begin
      spi_phase <= spi_phase + 1;
      if (spi_phase == 1) oled_clk <= 1;
      if (spi_phase == 3) begin
        oled_clk <= 0;
        if (spi_bit == 0) begin
          spi_busy <= 0;
          spi_done <= 1;
          oled_csn <= 1;
        end else begin
          spi_bit <= spi_bit - 1;
          spi_shift <= {spi_shift[6:0], 1'b0};
          oled_mosi <= spi_shift[6];
        end
      end
    end
  end

  reg uart_start = 0;
  reg [7:0] uart_data = 0;
  reg [9:0] uart_shift = 10'h3ff;
  reg [3:0] uart_bits = 0;
  reg [7:0] baud = 0;
  wire uart_busy = uart_bits != 0;
  assign ftdi_rxd = uart_shift[0];
  always @(posedge clk_25mhz)
    if (uart_busy) begin
      if (baud == 216) begin
        baud <= 0;
        uart_shift <= {1'b1, uart_shift[9:1]};
        uart_bits <= uart_bits - 1;
      end else baud <= baud + 1;
    end else if (uart_start) begin
      uart_shift <= {1'b1, uart_data, 1'b0};
      uart_bits <= 10;
      baud <= 0;
    end

  localparam RESET = 0, INIT_SEND = 1, INIT_WAIT = 2,
    WINDOW_SEND = 3, WINDOW_WAIT = 4, DATA_SEND = 5, DATA_WAIT = 6,
    UART_SEND = 7, UART_WAIT_START = 8, UART_WAIT_END = 9;
  reg [3:0] state = RESET;
  reg [14:0] reset_cycles = 0;
  reg [5:0] init_index = 0;
  reg [2:0] window_index = 0;
  reg [6:0] x = 0;
  reg [5:0] y = 0;
  reg low_byte = 0;
  reg [15:0] frame_number = 0;
  reg [31:0] crc = 32'hffffffff, frame_crc = 0;
  reg [4:0] text_index = 0;
  assign led = frame_number[7:0];
  // A moving white 16x16 box over a gradient. Truncation is modulo 32/64.
  wire [4:0] red = x[4:0] + frame_number[4:0];
  wire [5:0] green = y + frame_number[5:0];
  wire [4:0] blue = x[4:0] ^ y[4:0] ^ frame_number[4:0];
  wire box = x >= {1'b0, frame_number[5:0]} &&
    x < ({1'b0, frame_number[5:0]} + 7'd16) &&
    y >= {1'b0, frame_number[4:0]} &&
    y < ({1'b0, frame_number[4:0]} + 6'd16);
  wire [15:0] pixel = box ? 16'hffff : {red, green, blue};
  wire [7:0] picture_byte = low_byte ? pixel[7:0] : pixel[15:8];
  reg [7:0] window_byte;
  always @* case (window_index)
    0: window_byte = 8'h15;
    1: window_byte = 0;
    2: window_byte = 95;
    3: window_byte = 8'h75;
    4: window_byte = 0;
    default: window_byte = 63;
  endcase
  reg [7:0] text_byte;
  always @* case (text_index)
    0: text_byte = "O";
    1: text_byte = hex(frame_number[15:12]);
    2: text_byte = hex(frame_number[11:8]);
    3: text_byte = hex(frame_number[7:4]);
    4: text_byte = hex(frame_number[3:0]);
    5: text_byte = " ";
    6: text_byte = "C";
    7: text_byte = hex(frame_crc[31:28]);
    8: text_byte = hex(frame_crc[27:24]);
    9: text_byte = hex(frame_crc[23:20]);
    10: text_byte = hex(frame_crc[19:16]);
    11: text_byte = hex(frame_crc[15:12]);
    12: text_byte = hex(frame_crc[11:8]);
    13: text_byte = hex(frame_crc[7:4]);
    14: text_byte = hex(frame_crc[3:0]);
    15: text_byte = 8'h0d;
    default: text_byte = 8'h0a;
  endcase
  always @(posedge clk_25mhz) begin
    spi_start <= 0;
    uart_start <= 0;
    case (state)
      RESET: if (reset_cycles == 24999) begin
        oled_resn <= 1;
        state <= INIT_SEND;
      end else reset_cycles <= reset_cycles + 1;
      INIT_SEND: begin
        spi_data <= init_byte(init_index); spi_dc <= 0; spi_start <= 1;
        state <= INIT_WAIT;
      end
      INIT_WAIT: if (spi_done) begin
        if (init_index == 36) state <= WINDOW_SEND;
        else begin init_index <= init_index + 1; state <= INIT_SEND; end
      end
      WINDOW_SEND: begin
        spi_data <= window_byte; spi_dc <= 0; spi_start <= 1;
        state <= WINDOW_WAIT;
      end
      WINDOW_WAIT: if (spi_done) begin
        if (window_index == 5) begin
          x <= 0; y <= 0; low_byte <= 0; crc <= 32'hffffffff;
          state <= DATA_SEND;
        end else begin window_index <= window_index + 1; state <= WINDOW_SEND; end
      end
      DATA_SEND: begin
        spi_data <= picture_byte; spi_dc <= 1; spi_start <= 1;
        crc <= crc_byte(crc, picture_byte);
        state <= DATA_WAIT;
      end
      DATA_WAIT: if (spi_done) begin
        low_byte <= !low_byte;
        state <= DATA_SEND;
        if (low_byte) begin
          if (x == 95) begin
            x <= 0;
            if (y == 63) begin
              frame_crc <= ~crc; text_index <= 0; state <= UART_SEND;
            end else y <= y + 1;
          end else x <= x + 1;
        end
      end
      UART_SEND: begin
        uart_data <= text_byte; uart_start <= 1; state <= UART_WAIT_START;
      end
      UART_WAIT_START: if (uart_busy) state <= UART_WAIT_END;
      UART_WAIT_END: if (!uart_busy) begin
        if (text_index == 16) begin
          frame_number <= frame_number + 1; window_index <= 0; state <= WINDOW_SEND;
        end else begin text_index <= text_index + 1; state <= UART_SEND; end
      end
      default: state <= RESET;
    endcase
  end
endmodule
