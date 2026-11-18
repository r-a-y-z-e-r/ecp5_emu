// UART transmitter: sends "Hello, ULX3S!\r\n" at 115200 baud, 8N1, over the
// FTDI USB serial port, right after start and then about ten times per
// second. The LEDs show the byte being sent.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  localparam BIT_CYCLES = 217;      // 25 MHz / 115200
  localparam PAUSE_CYCLES = 2500000; // 0.1 s between messages
  localparam LENGTH = 15;

  reg [3:0] index = 0;
  reg [7:0] char;
  always @*
    case (index)
      0: char = "H"; 1: char = "e"; 2: char = "l"; 3: char = "l"; 4: char = "o";
      5: char = ","; 6: char = " "; 7: char = "U"; 8: char = "L"; 9: char = "X";
      10: char = "3"; 11: char = "S"; 12: char = "!"; 13: char = 8'h0d;
      default: char = 8'h0a;
    endcase

  reg [9:0] frame = 10'h3ff;  // stop bit, 8 data bits LSB first, start bit
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  reg [21:0] pause = 0;
  always @(posedge clk_25mhz)
    if (bits_left != 0) begin
      if (baud == BIT_CYCLES - 1) begin
        baud <= 0;
        frame <= {1'b1, frame[9:1]};
        bits_left <= bits_left - 1;
      end else
        baud <= baud + 1;
    end else if (pause != 0)
      pause <= pause - 1;
    else begin
      frame <= {1'b1, char, 1'b0};
      bits_left <= 10;
      if (index == LENGTH - 1) begin
        index <= 0;
        pause <= PAUSE_CYCLES;
      end else
        index <= index + 1;
    end
  assign ftdi_rxd = frame[0];
  assign led = char;
endmodule
