// Board check: only fixed reads, UART dumps and a CRC over the two windows.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led,
  output flash_csn, output flash_mosi, input flash_miso,
  output flash_wpn, output flash_holdn);
  wire clk = clk_25mhz;
  reg start = 0;
  reg [1:0] op = 1;
  reg [23:0] addr = 0;
  wire ready, sck;
  wire [31:0] data;
  flash_reader reader(.clk(clk),.start(start),.op(op),.addr(addr),
    .ready(ready),.data(data),.flash_csn(flash_csn),.flash_mosi(flash_mosi),
    .flash_miso(flash_miso),.flash_wpn(flash_wpn),.flash_holdn(flash_holdn),.sck(sck));
  assign led = {6'd0,flash_csn,sck};
  reg send = 0;
  reg [7:0] char = 0;
  wire busy;
  uart_tx uart(.clk(clk),.send(send),.data(char),.busy(busy),.tx(ftdi_rxd));
  reg [4:0] state = 0, resume = 0;
  reg [5:0] index = 0;
  reg [23:0] id = 0;
  reg [7:0] sr = 0;
  reg [31:0] word = 0, crc = 32'hffffffff, final_crc = 0;
  reg [2:0] nibble = 0;
  reg [24:0] delay_count = 0;
  function [7:0] hex;
    input [3:0] n;
    begin hex = n < 10 ? 8'd48 + n : 8'd65 + n - 10; end
  endfunction
  function [31:0] crc_word;
    input [31:0] c, w;
    reg [31:0] v;
    integer b, k;
    begin
      v = c;
      for (b=3;b>=0;b=b-1) begin
        v = v ^ ((w >> (b*8)) & 32'hff);
        for (k=0;k<8;k=k+1) v = (v >> 1) ^ (v[0] ? 32'hedb88320 : 0);
      end
      crc_word = v;
    end
  endfunction
  // State 30 sends one character, then continues at resume.
  always @(posedge clk) begin
    start <= 0; send <= 0;
    case (state)
      0: begin op <= 1; start <= 1; state <= 1; end
      1: if (ready) begin id <= data[23:0]; op <= 2; start <= 1; state <= 2; end
      2: if (ready) begin sr <= data[7:0]; index <= 0; state <= 3; end
      3: begin
        case (index)
          0: char <= "F"; 1: char <= "L"; 2: char <= "A"; 3: char <= "S";
          4: char <= "H"; 5: char <= " "; 6: char <= "I"; 7: char <= "D";
          8: char <= " "; 15: char <= " "; 16: char <= "S"; 17: char <= "R";
          18: char <= " "; 21: char <= 13; 22: char <= 10;
          default: if (index < 15) char <= hex(id >> ((14-index)*4));
                   else char <= hex(sr >> ((20-index)*4));
        endcase
        index <= index + 1; resume <= index == 22 ? 4 : 3; state <= 30;
      end
      4: begin addr <= 0; crc <= 32'hffffffff; index <= 0; state <= 5; end
      5: begin
        if (index < 6) char <= hex(addr >> ((5-index)*4));
        else char <= index == 6 ? ":" : " ";
        index <= index + 1; resume <= index == 7 ? 6 : 5; state <= 30;
      end
      6: begin op <= 0; start <= 1; state <= 7; end
      7: if (ready) begin word <= data; crc <= crc_word(crc,data); nibble <= 0; state <= 8; end
      8: begin
        char <= hex(word[31:28]); word <= {word[27:0],4'd0}; nibble <= nibble + 1;
        resume <= nibble == 7 ? 9 : 8; state <= 30;
      end
      9: begin
        addr <= addr + 4;
        if (addr[4:0] == 28) begin char <= 13; resume <= 10; state <= 30; end
        else state <= 6;
      end
      10: begin char <= 10; resume <= 11; state <= 30; end
      11: begin
        index <= 0;
        if (addr == 24'h010000) begin addr <= 24'h200000; state <= 5; end
        else if (addr == 24'h201000) begin final_crc <= ~crc; state <= 12; end
        else state <= 5;
      end
      12: begin
        case (index)
          0: char <= "C"; 1: char <= "R"; 2: char <= "C"; 3: char <= " ";
          12: char <= 13; 13: char <= 10;
          default: char <= hex(final_crc >> ((11-index)*4));
        endcase
        index <= index + 1; resume <= index == 13 ? 13 : 12; state <= 30;
      end
      13: if (delay_count == 24999999) begin delay_count <= 0; state <= 0; end
          else delay_count <= delay_count + 1;
      30: if (!busy && !send) begin send <= 1; state <= 31; end
      31: if (busy) state <= resume;
    endcase
  end
endmodule
