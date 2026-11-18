// Read-only DDC base block, once per second. No EEPROM data-write path.
// DVI is the hdmi_bars path; DDC/UART stay in the board's 25 MHz domain.
module top(input clk_25mhz, output [3:0] gpdi_dp, output ftdi_rxd,
           inout ddc_scl, inout ddc_sda);
  dvi_source source(.clk_25mhz(clk_25mhz),.gpdi_dp(gpdi_dp));
  reg start = 0;
  wire ready, valid, error;
  wire [7:0] data;
  edid_reader reader(.clk(clk_25mhz),.start(start),.ready(ready),.valid(valid),
    .data(data),.error(error),.ddc_scl(ddc_scl),.ddc_sda(ddc_sda));
  reg [7:0] bytes [0:127];
  reg [6:0] address = 0;
  reg [7:0] sum = 0;
  reg [24:0] period = 0;
  reg [3:0] state = 0, resume = 0;
  reg [8:0] index = 0;
  reg send = 0;
  reg [7:0] char = 0;
  wire busy;
  uart_tx uart(.clk(clk_25mhz),.send(send),.data(char),.busy(busy),.tx(ftdi_rxd));
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? 8'd48 + n : 8'd65 + n - 10; end
  endfunction
  always @(posedge clk_25mhz) begin
    send <= 0;
    if (period == 24999999) period <= 0; else period <= period + 1;
    if (valid) begin bytes[address] <= data; address <= address + 1; sum <= sum + data; end
    case (state)
      0: begin address <= 0; sum <= 0; start <= 1; state <= 1; end
      1: if (ready) begin start <= 0; index <= 0; state <= error ? 4 : 2; end
      2: begin
        case (index)
          0: char <= "E"; 1: char <= "D"; 2: char <= "I"; 3: char <= "D"; 4: char <= " ";
          261: char <= " "; 262: char <= "s"; 263: char <= "u"; 264: char <= "m"; 265: char <= " ";
          266: char <= sum == 0 ? "o" : "b";
          267: char <= sum == 0 ? "k" : "a";
          268: char <= sum == 0 ? 13 : "d";
          269: char <= sum == 0 ? 10 : 13;
          270: char <= 10;
          default: char <= index[0] ? hex(bytes[(index-5)>>1][7:4]) : hex(bytes[(index-5)>>1][3:0]);
        endcase
        index <= index + 1;
        resume <= index == (sum == 0 ? 269 : 270) ? 3 : 2; state <= 6;
      end
      3: if (period == 24999999) state <= 0;
      4: begin
        case (index)
          0: char <= "E"; 1: char <= "D"; 2: char <= "I"; 3: char <= "D"; 4: char <= " ";
          5: char <= "n"; 6: char <= "a"; 7: char <= "c"; 8: char <= "k";
          9: char <= 13; 10: char <= 10;
        endcase
        index <= index + 1; resume <= index == 10 ? 3 : 4; state <= 6;
      end
      6: if (!busy && !send) begin send <= 1; state <= 7; end
      7: if (busy) state <= resume;
    endcase
  end
endmodule
