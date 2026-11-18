// HC05 slice: EHXPLLL (ecppll 25 -> 100 MHz) lock time, pre-lock output
// activity and frequency ratio, reported over the ULX3S FTDI UART.
// The observer runs on the 25 MHz reference, not on the PLL under test.
// Line: "L=<cycles to first LOCK> P=<PLL edges before LOCK> F=<PLL edges in
// 65536 reference cycles> D=<LOCK drops after first lock>", all hex.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  wire clk100, locked;
  pll100 pll(.clkin(clk_25mhz), .clkout0(clk100), .locked(locked));

  // Reference domain.
  reg [1:0] lsync = 0;
  always @(posedge clk_25mhz) lsync <= {lsync[0], locked};
  wire lk = lsync[1];
  reg [23:0] lock_cycles = 0;
  reg seen = 0;
  always @(posedge clk_25mhz)
    if (!seen) begin
      if (lk) seen <= 1;
      else if (lock_cycles != 24'hFFFFFF) lock_cycles <= lock_cycles + 1;
    end
  reg lk_d = 0;
  reg [7:0] drops = 0;
  always @(posedge clk_25mhz) begin
    lk_d <= lk;
    if (seen && lk_d && !lk && drops != 8'hFF) drops <= drops + 1;
  end
  // Measurement window: 65536 reference cycles, starting 65536 after lock.
  reg [17:0] win = 0;
  reg gate = 0, measured = 0;
  always @(posedge clk_25mhz)
    if (seen && !measured) begin
      win <= win + 1;
      if (win == 18'h10000) gate <= 1;
      if (win == 18'h20000) gate <= 0;
      if (win == 18'h30000) measured <= 1;
    end

  // PLL domain.
  reg [1:0] lsync_p = 0, gsync_p = 0;
  always @(posedge clk100) begin
    lsync_p <= {lsync_p[0], locked};
    gsync_p <= {gsync_p[0], gate};
  end
  reg [23:0] pre = 0, freq = 0;
  always @(posedge clk100) begin
    if (!lsync_p[1] && pre != 24'hFFFFFF) pre <= pre + 1;
    if (gsync_p[1]) freq <= freq + 1;
  end

  // UART reporter, 115200 8N1 from 25 MHz.
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [5:0] ci = 0;
  reg [7:0] ch;
  always @* case (ci)
    0: ch = "L"; 1: ch = "=";
    2: ch = hex(lock_cycles[23:20]); 3: ch = hex(lock_cycles[19:16]); 4: ch = hex(lock_cycles[15:12]);
    5: ch = hex(lock_cycles[11:8]); 6: ch = hex(lock_cycles[7:4]); 7: ch = hex(lock_cycles[3:0]);
    8: ch = " "; 9: ch = "P"; 10: ch = "=";
    11: ch = hex(pre[23:20]); 12: ch = hex(pre[19:16]); 13: ch = hex(pre[15:12]);
    14: ch = hex(pre[11:8]); 15: ch = hex(pre[7:4]); 16: ch = hex(pre[3:0]);
    17: ch = " "; 18: ch = "F"; 19: ch = "=";
    20: ch = hex(freq[23:20]); 21: ch = hex(freq[19:16]); 22: ch = hex(freq[15:12]);
    23: ch = hex(freq[11:8]); 24: ch = hex(freq[7:4]); 25: ch = hex(freq[3:0]);
    26: ch = " "; 27: ch = "D"; 28: ch = "=";
    29: ch = hex(drops[7:4]); 30: ch = hex(drops[3:0]);
    31: ch = 8'h0d; default: ch = 8'h0a;
  endcase
  reg tx = 1;
  reg busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  reg [22:0] pause = 0;
  always @(posedge clk_25mhz)
    if (!busy) begin
      if (pause != 0) pause <= pause - 1;
      else if (measured) begin
        shift <= {1'b1, ch, 1'b0};
        busy <= 1; baud <= 0; bitn <= 0;
      end
    end else if (baud == 216) begin
      baud <= 0;
      tx <= shift[0];
      shift <= {1'b1, shift[9:1]};
      bitn <= bitn + 1;
      if (bitn == 9) begin
        busy <= 0;
        if (ci == 32) begin ci <= 0; pause <= 23'd2500000; end
        else ci <= ci + 1;
      end
    end else baud <= baud + 1;
  assign ftdi_rxd = tx;
  assign led = {measured, gate, seen, lk, 4'b0};
endmodule
