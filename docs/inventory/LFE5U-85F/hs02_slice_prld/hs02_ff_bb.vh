// Synthesis-only interface for the native primitive, copied from cells_sim.
// Replace Yosys's whitebox after map_ffs to avoid ABC9 processing initial Q=M.
// This does not change any instance parameter or hardware connection.
(* blackbox *)
module TRELLIS_FF(input CLK, LSR, CE, DI, M, output Q);
  parameter GSR = "ENABLED";
  parameter [127:0] CEMUX = "1";
  parameter CLKMUX = "CLK";
  parameter LSRMUX = "LSR";
  parameter SRMODE = "LSR_OVER_CE";
  parameter REGSET = "RESET";
  parameter [127:0] LSRMODE = "LSR";
endmodule
