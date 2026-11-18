module top(input clk_25mhz, input ftdi_txd, output ftdi_rxd, input [6:0] btn, output [7:0] led);
  wire [53:0] r; wire [17:0] q;
  filters fir(clk_25mhz, {14'b0,btn[3:0]}, 18'h12345, r, q);
  wire [7:0] key; wire key_valid, tx_ready, overflow;
  gallery_io io(clk_25mhz, 1'b0, ftdi_txd, ftdi_rxd, btn, r[7:0], q[4], tx_ready, key, key_valid, 1'b0, overflow);
  assign led = r[15:8] ^ q[7:0] ^ key;
endmodule
