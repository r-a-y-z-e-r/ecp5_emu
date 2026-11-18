// Single halfword requests at 25 MHz. Commands change on the board's rising
// edge; the SDRAM samples on its falling edge. CL2, BL1, auto precharge.
// The caller holds req and its payload until ready, then drops req.
module sdram_controller(
  input clk, input req, input wr, input [23:0] addr,
  input [15:0] wdata, input [1:0] mask,
  output reg ready = 0, output reg [15:0] rdata = 0,
  output sdram_clk, output sdram_cke, output sdram_csn,
  output reg sdram_rasn = 1, output reg sdram_casn = 1, output reg sdram_wen = 1,
  output reg [1:0] sdram_ba = 0, output reg [12:0] sdram_a = 0,
  output reg [1:0] sdram_dqm = 0, inout [15:0] sdram_d
);
  assign sdram_clk = ~clk;
  assign sdram_cke = 1;
  assign sdram_csn = 0;
  reg drive = 0;
  reg [15:0] data_out = 0;
  assign sdram_d = drive ? data_out : 16'bz;
  reg [4:0] state = 0;
  reg [11:0] delay = 2501; // More than 100 us, including the first partial clock.
  reg [7:0] refresh = 0;
  reg saved_wr = 0;
  reg [8:0] saved_col = 0;
  reg [15:0] saved_data = 0;
  reg [1:0] saved_mask = 0;
  always @(posedge clk) begin
    sdram_rasn <= 1; sdram_casn <= 1; sdram_wen <= 1;
    drive <= 0; sdram_dqm <= 0; ready <= 0;
    if (state >= 10) refresh <= refresh + 1;
    case (state)
      0: if (delay != 0) delay <= delay - 1;
         else begin // PRECHARGE ALL
           sdram_rasn <= 0; sdram_wen <= 0; sdram_a <= 13'h400;
           state <= 1;
         end
      1: state <= 2;
      2: begin sdram_rasn <= 0; sdram_casn <= 0; state <= 3; end
      3: state <= 4;
      4: begin sdram_rasn <= 0; sdram_casn <= 0; state <= 5; end
      5: state <= 6;
      6: begin // BL1, sequential, CL2, programmed write burst.
           sdram_rasn <= 0; sdram_casn <= 0; sdram_wen <= 0;
           sdram_ba <= 0; sdram_a <= 13'h020; state <= 7;
         end
      7: state <= 8;
      8: state <= 10;
      10: if (refresh >= 180) begin // Less than 7.8125 us even at maximum request latency.
            sdram_rasn <= 0; sdram_casn <= 0; refresh <= 0; state <= 20;
          end else if (req) begin
            sdram_rasn <= 0; sdram_ba <= addr[23:22]; sdram_a <= addr[21:9];
            saved_col <= addr[8:0]; saved_wr <= wr;
            saved_data <= wdata; saved_mask <= mask; state <= 11;
          end
      11: state <= 12;
      12: begin
            sdram_casn <= 0; sdram_wen <= !saved_wr;
            sdram_a <= {2'b00,1'b1,1'b0,saved_col};
            drive <= saved_wr; data_out <= saved_data;
            sdram_dqm <= saved_wr ? saved_mask : 2'b00; state <= 13;
          end
      13: state <= 14;
      14: begin if (!saved_wr) rdata <= sdram_d; state <= 15; end
      15: state <= 16;
      16: begin ready <= 1; state <= 17; end
      17: if (!req) state <= 10;
      20: state <= 21;
      21: state <= 10;
      default: state <= 0;
    endcase
  end
endmodule
