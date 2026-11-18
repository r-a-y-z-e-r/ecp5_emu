// iverilog testbench for the ALU54B MAC reproducer: samples ftdi_rxd with
// the 25 MHz clock at 217 cycles/bit and prints the seven phase lines.
// Run: sh run.sh
`timescale 1ns/1ns
module tb;
  reg clk = 0;
  wire tx;
  top dut(clk, tx);
  always #20 clk = ~clk;  // 25 MHz

  integer cycle = 0;
  integer k;
  integer bitpos = -1;   // -1 idle, 0 start seen, else data bit index 0..7
  integer cnt = 0;
  reg [7:0] byte;
  reg prev = 1;
  always @(posedge clk) begin
    cycle = cycle + 1;
    prev <= tx;
    if (bitpos == -1) begin
      if (prev && !tx) begin bitpos = 0; cnt = 0; end
    end else begin
      cnt = cnt + 1;
      if (cnt == 217/2 + 1 && bitpos == 0) begin bitpos = 1; cnt = 0; end
      else if (bitpos >= 1 && cnt == 217) begin
        if (bitpos <= 8) byte[bitpos-1] = tx;
        cnt = 0;
        if (bitpos == 9) begin
          bitpos = -1;
          if (byte == 8'h0d) $write("\n");
          else if (byte != 8'h0a) $write("%c", byte);
        end else bitpos = bitpos + 1;
      end
    end
    if (cycle == 520000) begin $write("\n"); $finish; end
  end
endmodule
