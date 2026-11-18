// Writes 64 KB, then checks every halfword. The LFSR seed changes each pass.
module top(
  input clk_25mhz, output ftdi_rxd, output [7:0] led,
  output sdram_clk, output sdram_cke, output sdram_csn,
  output sdram_rasn, output sdram_casn, output sdram_wen,
  output [1:0] sdram_ba, output [12:0] sdram_a,
  output [1:0] sdram_dqm, inout [15:0] sdram_d
);
  reg req = 0, writing = 1;
  reg [14:0] word = 0;
  reg [15:0] pattern = 16'hace1, seed = 16'hace1, errors = 0;
  wire [15:0] next_pattern = {pattern[14:0],pattern[15]^pattern[13]^pattern[12]^pattern[10]};
  wire ready;
  wire [15:0] data;
  sdram_controller ram(
    .clk(clk_25mhz),.req(req),.wr(writing),.addr({9'd0,word}),
    .wdata(pattern),.mask(2'b00),.ready(ready),.rdata(data),
    .sdram_clk(sdram_clk),.sdram_cke(sdram_cke),.sdram_csn(sdram_csn),
    .sdram_rasn(sdram_rasn),.sdram_casn(sdram_casn),.sdram_wen(sdram_wen),
    .sdram_ba(sdram_ba),.sdram_a(sdram_a),.sdram_dqm(sdram_dqm),.sdram_d(sdram_d)
  );
  reg [2:0] state = 0;
  reg [4:0] index = 0;
  reg [9:0] frame = 10'h3ff;
  reg [3:0] bits_left = 0;
  reg [7:0] baud = 0;
  reg [7:0] char;
  reg [3:0] digit;
  always @* begin
    case (index)
      14: digit = errors[15:12]; 15: digit = errors[11:8];
      16: digit = errors[7:4]; default: digit = errors[3:0];
    endcase
    case (index)
      0: char="S"; 1: char="D"; 2: char="R"; 3: char="A"; 4: char="M";
      5: char=" "; 6: char="W"; 7: char="8";
      8,9,10: char="0"; 11: char=" "; 12: char="E";
      13: char = errors[15:12] < 10 ? "0"+errors[15:12] : "A"+errors[15:12]-10;
      14: char = errors[11:8] < 10 ? "0"+errors[11:8] : "A"+errors[11:8]-10;
      15: char = errors[7:4] < 10 ? "0"+errors[7:4] : "A"+errors[7:4]-10;
      16: char = errors[3:0] < 10 ? "0"+errors[3:0] : "A"+errors[3:0]-10;
      17: char=8'h0d; default: char=8'h0a;
    endcase
  end
  always @(posedge clk_25mhz) begin
    case (state)
      0: begin req <= 1; state <= 1; end
      1: if (ready) begin
           req <= 0;
           if (!writing && data != pattern) errors <= errors + 1;
           state <= 2;
         end
      2: begin
           if (&word) begin
             word <= 0; pattern <= seed;
             if (writing) begin writing <= 0; state <= 0; end
             else begin index <= 0; state <= 3; end
           end else begin word <= word + 1; pattern <= next_pattern; state <= 0; end
         end
      3: if (bits_left == 0) begin
           frame <= {1'b1,char,1'b0}; bits_left <= 10; baud <= 0;
           if (index == 18) state <= 4; else index <= index + 1;
         end
      4: if (bits_left == 0) begin
           seed <= {seed[14:0],seed[15]^seed[13]^seed[12]^seed[10]};
           pattern <= {seed[14:0],seed[15]^seed[13]^seed[12]^seed[10]};
           writing <= 1; errors <= 0; state <= 0;
         end
    endcase
    if (bits_left != 0) begin
      if (baud == 216) begin
        baud <= 0; frame <= {1'b1,frame[9:1]}; bits_left <= bits_left - 1;
      end else baud <= baud + 1;
    end
  end
  assign ftdi_rxd = frame[0];
  assign led = {errors != 0,7'd0};
endmodule
