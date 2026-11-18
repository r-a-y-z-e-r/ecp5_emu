// Board check: only pointer writes, then repeated START and sequential reads.
// The RTC's two blocks wrap separately: read 32 registers, then 64 SRAM bytes.
module top(input clk_25mhz, output ftdi_rxd, inout rtc_scl, inout rtc_sda);
  wire clk = clk_25mhz;
  reg start = 0;
  reg [7:0] pointer = 0;
  reg [6:0] count = 32;
  wire ready, valid, error;
  wire [7:0] data;
  rtc_reader reader(.clk(clk),.start(start),.pointer(pointer),.count(count),
    .ready(ready),.valid(valid),.data(data),.error(error),.rtc_scl(rtc_scl),.rtc_sda(rtc_sda));
  reg [7:0] bytes [0:95];
  reg [6:0] address = 0;
  reg [4:0] state = 0, resume = 0;
  reg [7:0] index = 0;
  reg [24:0] period = 0;
  reg send = 0;
  reg [7:0] char = 0;
  wire busy;
  uart_tx uart(.clk(clk),.send(send),.data(char),.busy(busy),.tx(ftdi_rxd));
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? 8'd48 + n : 8'd65 + n - 10; end
  endfunction
  wire [7:0] raw_hour = bytes[2];
  wire [4:0] hour12 = raw_hour[4] * 10 + raw_hour[3:0];
  wire [4:0] hour24 = raw_hour[6] ? ((hour12 == 12 ? 0 : hour12) + (raw_hour[5] ? 12 : 0))
                                              : raw_hour[5:4] * 10 + raw_hour[3:0];
  always @(posedge clk) begin
    send <= 0;
    // Start-to-start period is exactly one emulated/board second.
    if (period == 24999999) period <= 0; else period <= period + 1;
    if (valid) begin bytes[address] <= data; address <= address + 1; end
    case (state)
      0: begin pointer <= 0; count <= 32; address <= 0; start <= 1; state <= 1; end
      1: if (ready) begin
        start <= 0;
        if (error) state <= 20; else state <= 2;
      end
      2: begin pointer <= 8'h20; count <= 64; start <= 1; state <= 3; end
      3: if (ready) begin start <= 0; index <= 0; state <= error ? 20 : 4; end
      4: begin
        case (index)
          0: char <= "R"; 1: char <= "T"; 2: char <= "C"; 3: char <= " ";
          4: char <= "t"; 5: char <= " ";
          6: char <= "0" + hour24 / 10; 7: char <= "0" + hour24 % 10;
          8: char <= ":"; 9: char <= hex(bytes[1][6:4]); 10: char <= hex(bytes[1][3:0]);
          11: char <= ":"; 12: char <= hex(bytes[0][6:4]); 13: char <= hex(bytes[0][3:0]);
          14: char <= " "; 15: char <= "2"; 16: char <= "0";
          17: char <= hex(bytes[6][7:4]); 18: char <= hex(bytes[6][3:0]); 19: char <= "-";
          20: char <= hex({3'd0,bytes[5][4]}); 21: char <= hex(bytes[5][3:0]); 22: char <= "-";
          23: char <= hex(bytes[4][5:4]); 24: char <= hex(bytes[4][3:0]);
          25: char <= " "; 26: char <= "s"; 27: char <= "t"; 28: char <= " ";
          29: char <= bytes[0][7] ? "1" : "0"; 30: char <= 13; 31: char <= 10;
        endcase
        index <= index + 1; resume <= index == 31 ? 5 : 4; state <= 30;
      end
      5: begin index <= 0; state <= 6; end
      6: begin
        case (index)
          0: char <= "R"; 1: char <= "T"; 2: char <= "C"; 3: char <= " ";
          4: char <= "r"; 5: char <= " "; 70: char <= 13; 71: char <= 10;
          default: char <= index[0] ? hex(bytes[(index-6)>>1][3:0]) : hex(bytes[(index-6)>>1][7:4]);
        endcase
        index <= index + 1; resume <= index == 71 ? 7 : 6; state <= 30;
      end
      7: begin index <= 0; state <= 8; end
      8: begin
        case (index)
          0: char <= "R"; 1: char <= "T"; 2: char <= "C"; 3: char <= " ";
          4: char <= "s"; 5: char <= " "; 134: char <= 13; 135: char <= 10;
          default: char <= index[0] ? hex(bytes[32+((index-6)>>1)][3:0]) : hex(bytes[32+((index-6)>>1)][7:4]);
        endcase
        index <= index + 1; resume <= index == 135 ? 9 : 8; state <= 30;
      end
      9: if (period == 24999999) state <= 0;
      // Report a missing device; never try another command or write data.
      20: begin index <= 0; state <= 21; end
      21: begin
        case (index)
          0: char <= "R"; 1: char <= "T"; 2: char <= "C"; 3: char <= " ";
          4: char <= "N"; 5: char <= "A"; 6: char <= "C"; 7: char <= "K";
          8: char <= 13; 9: char <= 10;
        endcase
        index <= index + 1; resume <= index == 9 ? 9 : 21; state <= 30;
      end
      30: if (!busy && !send) begin send <= 1; state <= 31; end
      31: if (busy) state <= resume;
    endcase
  end
endmodule
