// Four input-only DIP switches; UART order is sw[0] first.
module top(input clk_25mhz, input [3:0] sw, output [7:0] led, output ftdi_rxd);
  assign led = {4'b0000,sw};
  reg [3:0] sync0 = 0, sync1 = 0, candidate = 0, stable = 0;
  reg [17:0] debounce = 0;
  reg initialized = 0;
  reg [24:0] period = 0;
  reg pending = 0;
  always @(posedge clk_25mhz) begin
    sync0 <= sw; sync1 <= sync0;
    if (begin_line) pending <= 0;
    if (sync1 != candidate) begin candidate <= sync1; debounce <= 0; end
    else if (debounce < 249999) debounce <= debounce + 1;
    else if (!initialized || stable != candidate) begin
      stable <= candidate; initialized <= 1; pending <= 1;
    end
    if (period == 24999999) begin period <= 0; if (initialized) pending <= 1; end
    else period <= period + 1;
  end
  reg active = 0;
  reg [3:0] index = 0, sample = 0;
  reg [9:0] frame = 10'h3ff;
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  wire begin_line = !active && pending;
  reg [7:0] char;
  always @* begin
    case (index)
      0: char = "S"; 1: char = "W"; 2: char = " ";
      3: char = sample[0] ? "1" : "0";
      4: char = sample[1] ? "1" : "0";
      5: char = sample[2] ? "1" : "0";
      6: char = sample[3] ? "1" : "0";
      7: char = 13; default: char = 10;
    endcase
  end
  always @(posedge clk_25mhz) begin
    if (begin_line) begin active <= 1; index <= 0; sample <= stable; end
    if (active && bits_left == 0) begin
      frame <= {1'b1,char,1'b0}; bits_left <= 10; baud <= 0;
      if (index == 8) active <= 0; else index <= index + 1;
    end
    if (bits_left != 0) begin
      if (baud == 216) begin
        baud <= 0; frame <= {1'b1,frame[9:1]}; bits_left <= bits_left - 1;
      end else baud <= baud + 1;
    end
  end
  assign ftdi_rxd = frame[0];
endmodule
