// Resolve the bidirectional bus inside Verilator, leaving the hardware top
// unchanged. The extra output observes the FPGA pad's output-enable only.
module sim_top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output [7:0] led,
 output sdram_clk, sdram_cke, sdram_csn, sdram_rasn, sdram_casn, sdram_wen,
 output [1:0] sdram_ba, output [12:0] sdram_a, output [1:0] sdram_dqm,
 input model_drive, input [15:0] model_data,
 output [15:0] bus_data, output fpga_drive);
 wire [15:0] dq;
 assign dq=model_drive ? model_data:16'bz;
 assign bus_data=dq;
 top dut(clk_25mhz,btn,ftdi_txd,ftdi_rxd,led,sdram_clk,sdram_cke,
  sdram_csn,sdram_rasn,sdram_casn,sdram_wen,sdram_ba,sdram_a,sdram_dqm,dq);
 assign fpga_drive=dut.ram.driving;
endmodule
