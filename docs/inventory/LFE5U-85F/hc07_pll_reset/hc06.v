// HC06: EHXPLLL RST and STDBY while running. Written by hc06_gen.py; the
// line format is described there.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
  wire pa_clk, pa_lock;
  wire pa_fb;
  wire [19:0] pa_drop, pa_first, pa_stable, pa_lockt;
  wire [15:0] pa_during;
  wire pa_level, pa_done;
  wire [9:0] pa_bmin, pa_bmax;
  wire [5:0] pa_ph0, pa_ph1;
  wire pb_clk, pb_lock;
  wire pb_fb;
  wire [19:0] pb_drop, pb_first, pb_stable, pb_lockt;
  wire [15:0] pb_during;
  wire pb_level, pb_done;
  wire [9:0] pb_bmin, pb_bmax;
  wire [5:0] pb_ph0, pb_ph1;
  wire pc_clk, pc_lock;
  wire [19:0] pc_drop, pc_first, pc_stable, pc_lockt;
  wire [15:0] pc_during;
  wire pc_level, pc_done;
  wire [9:0] pc_bmin, pc_bmax;
  wire [5:0] pc_ph0, pc_ph1;
  wire pd_clk, pd_lock;
  wire [19:0] pd_drop, pd_first, pd_stable, pd_lockt;
  wire [15:0] pd_during;
  wire pd_level, pd_done;
  wire [9:0] pd_bmin, pd_bmax;
  wire [5:0] pd_ph0, pd_ph1;

  // Sequencer: gap, pulse, wait, send.
  reg [31:0] lfsr = 32'h1F2E3D4C;
  always @(posedge clk_25mhz)
    lfsr <= {lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]};
  reg [19:0] wtab;
  always @* case (lfsr[3:0])
    4'd0: wtab = 20'd1;
    4'd1: wtab = 20'd2;
    4'd2: wtab = 20'd3;
    4'd3: wtab = 20'd4;
    4'd4: wtab = 20'd5;
    4'd5: wtab = 20'd8;
    4'd6: wtab = 20'd16;
    4'd7: wtab = 20'd32;
    4'd8: wtab = 20'd100;
    4'd9: wtab = 20'd256;
    4'd10: wtab = 20'd1000;
    4'd11: wtab = 20'd4096;
    4'd12: wtab = 20'd10000;
    4'd13: wtab = 20'd25000;
    4'd14: wtab = 20'd50000;
    4'd15: wtab = 20'd100000;
  endcase
  reg [1:0] st = 0;
  reg [19:0] t = 0, u = 0, w = 0;
  reg [16:0] gap = 17'd65535;
  reg pulse = 0;
  reg [2:0] pd = 0;
  reg [5:0] blk = 0;
  wire ev_start = st == 0 && gap == 0;
  wire all_done = pa_done && pb_done && pc_done && pd_done;
  wire go_send = st == 2 && ((all_done && u >= 1023) || u == 20'hFFFFF);
  reg line_done = 0;
  always @(posedge clk_25mhz) begin
    blk <= blk + 1;
    pd <= {pd[1:0], pulse};
    case (st)
      0: if (gap != 0) gap <= gap - 1;
         else begin st <= 1; pulse <= 1; t <= 0; w <= wtab; end
      1: begin
           t <= t + 1;
           if (t + 1 == w) begin pulse <= 0; st <= 2; u <= 0; end
         end
      2: begin
           if (t != 20'hFFFFF) t <= t + 1;
           u <= u + 1;
           if (go_send) st <= 3;
         end
      3: if (line_done) begin st <= 0; gap <= {1'b1, lfsr[15:0]}; end
    endcase
  end
  (* FREQUENCY_PIN_CLKI="25" *) (* FREQUENCY_PIN_CLKOP="50" *)
  (* ICP_CURRENT="6" *) (* LPF_RESISTOR="16" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)
  EHXPLLL #(
    .PLLRST_ENA("DISABLED"), .STDBY_ENABLE("DISABLED"),
    .INTFB_WAKE("DISABLED"), .DPHASE_SOURCE("DISABLED"),
    .OUTDIVIDER_MUXA("DIVA"), .OUTDIVIDER_MUXB("DIVB"),
    .OUTDIVIDER_MUXC("DIVC"), .OUTDIVIDER_MUXD("DIVD"),
    .FEEDBK_PATH("INT_OS3"), .CLKFB_DIV(24), .CLKI_DIV(1),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(11), .CLKOP_FPHASE(0),
    .CLKOS3_ENABLE("ENABLED"), .CLKOS3_DIV(1), .CLKOS3_CPHASE(0), .CLKOS3_FPHASE(0),
    .CLKOP_TRIM_POL("RISING"), .CLKOP_TRIM_DELAY(0)
  ) pa_pll (
    .RST(pulse), .STDBY(1'b0), .CLKI(clk_25mhz),
    .CLKOP(pa_clk), .CLKFB(pa_fb), .CLKINTFB(pa_fb),
    .PHASESEL0(1'b0), .PHASESEL1(1'b0), .PHASEDIR(1'b0), .PHASESTEP(1'b0),
    .PHASELOADREG(1'b0), .PLLWAKESYNC(1'b0), .ENCLKOP(1'b0),
    .LOCK(pa_lock)
  );
  obs pa_obs(
    .sys(clk_25mhz), .co(pa_clk), .lock(pa_lock),
    .ev_start(ev_start), .in_pulse(pd[2]), .waiting(st == 2), .blk_end(blk == 63),
    .t(t), .u(u), .snap(go_send),
    .drop(pa_drop), .during(pa_during), .level(pa_level),
    .first(pa_first), .stable(pa_stable), .lockt(pa_lockt),
    .bmin(pa_bmin), .bmax(pa_bmax), .ph0(pa_ph0), .ph1(pa_ph1),
    .done(pa_done));
  (* FREQUENCY_PIN_CLKI="25" *) (* FREQUENCY_PIN_CLKOP="50" *)
  (* ICP_CURRENT="6" *) (* LPF_RESISTOR="16" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)
  EHXPLLL #(
    .PLLRST_ENA("ENABLED"), .STDBY_ENABLE("DISABLED"),
    .INTFB_WAKE("DISABLED"), .DPHASE_SOURCE("DISABLED"),
    .OUTDIVIDER_MUXA("DIVA"), .OUTDIVIDER_MUXB("DIVB"),
    .OUTDIVIDER_MUXC("DIVC"), .OUTDIVIDER_MUXD("DIVD"),
    .FEEDBK_PATH("INT_OS3"), .CLKFB_DIV(24), .CLKI_DIV(1),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(11), .CLKOP_FPHASE(0),
    .CLKOS3_ENABLE("ENABLED"), .CLKOS3_DIV(1), .CLKOS3_CPHASE(0), .CLKOS3_FPHASE(0),
    .CLKOP_TRIM_POL("RISING"), .CLKOP_TRIM_DELAY(0)
  ) pb_pll (
    .RST(pulse), .STDBY(1'b0), .CLKI(clk_25mhz),
    .CLKOP(pb_clk), .CLKFB(pb_fb), .CLKINTFB(pb_fb),
    .PHASESEL0(1'b0), .PHASESEL1(1'b0), .PHASEDIR(1'b0), .PHASESTEP(1'b0),
    .PHASELOADREG(1'b0), .PLLWAKESYNC(1'b0), .ENCLKOP(1'b0),
    .LOCK(pb_lock)
  );
  obs pb_obs(
    .sys(clk_25mhz), .co(pb_clk), .lock(pb_lock),
    .ev_start(ev_start), .in_pulse(pd[2]), .waiting(st == 2), .blk_end(blk == 63),
    .t(t), .u(u), .snap(go_send),
    .drop(pb_drop), .during(pb_during), .level(pb_level),
    .first(pb_first), .stable(pb_stable), .lockt(pb_lockt),
    .bmin(pb_bmin), .bmax(pb_bmax), .ph0(pb_ph0), .ph1(pb_ph1),
    .done(pb_done));
  (* FREQUENCY_PIN_CLKI="25" *) (* FREQUENCY_PIN_CLKOP="50" *)
  (* ICP_CURRENT="12" *) (* LPF_RESISTOR="8" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)
  EHXPLLL #(
    .STDBY_ENABLE("DISABLED"), .PLLRST_ENA("DISABLED"),
    .INTFB_WAKE("DISABLED"), .DPHASE_SOURCE("DISABLED"),
    .OUTDIVIDER_MUXA("DIVA"), .OUTDIVIDER_MUXB("DIVB"),
    .OUTDIVIDER_MUXC("DIVC"), .OUTDIVIDER_MUXD("DIVD"),
    .FEEDBK_PATH("CLKOP"), .CLKFB_DIV(2), .CLKI_DIV(1),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(5), .CLKOP_FPHASE(0),
    .CLKOP_TRIM_POL("RISING"), .CLKOP_TRIM_DELAY(0)
  ) pc_pll (
    .RST(1'b0), .STDBY(pulse), .CLKI(clk_25mhz),
    .CLKOP(pc_clk), .CLKFB(pc_clk), .CLKINTFB(),
    .PHASESEL0(1'b0), .PHASESEL1(1'b0), .PHASEDIR(1'b0), .PHASESTEP(1'b0),
    .PHASELOADREG(1'b0), .PLLWAKESYNC(1'b0), .ENCLKOP(1'b0),
    .LOCK(pc_lock)
  );
  obs pc_obs(
    .sys(clk_25mhz), .co(pc_clk), .lock(pc_lock),
    .ev_start(ev_start), .in_pulse(pd[2]), .waiting(st == 2), .blk_end(blk == 63),
    .t(t), .u(u), .snap(go_send),
    .drop(pc_drop), .during(pc_during), .level(pc_level),
    .first(pc_first), .stable(pc_stable), .lockt(pc_lockt),
    .bmin(pc_bmin), .bmax(pc_bmax), .ph0(pc_ph0), .ph1(pc_ph1),
    .done(pc_done));
  (* FREQUENCY_PIN_CLKI="25" *) (* FREQUENCY_PIN_CLKOP="50" *)
  (* ICP_CURRENT="12" *) (* LPF_RESISTOR="8" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)
  EHXPLLL #(
    .STDBY_ENABLE("ENABLED"), .PLLRST_ENA("DISABLED"),
    .INTFB_WAKE("DISABLED"), .DPHASE_SOURCE("DISABLED"),
    .OUTDIVIDER_MUXA("DIVA"), .OUTDIVIDER_MUXB("DIVB"),
    .OUTDIVIDER_MUXC("DIVC"), .OUTDIVIDER_MUXD("DIVD"),
    .FEEDBK_PATH("CLKOP"), .CLKFB_DIV(2), .CLKI_DIV(1),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(5), .CLKOP_FPHASE(0),
    .CLKOP_TRIM_POL("RISING"), .CLKOP_TRIM_DELAY(0)
  ) pd_pll (
    .RST(1'b0), .STDBY(pulse), .CLKI(clk_25mhz),
    .CLKOP(pd_clk), .CLKFB(pd_clk), .CLKINTFB(),
    .PHASESEL0(1'b0), .PHASESEL1(1'b0), .PHASEDIR(1'b0), .PHASESTEP(1'b0),
    .PHASELOADREG(1'b0), .PLLWAKESYNC(1'b0), .ENCLKOP(1'b0),
    .LOCK(pd_lock)
  );
  obs pd_obs(
    .sys(clk_25mhz), .co(pd_clk), .lock(pd_lock),
    .ev_start(ev_start), .in_pulse(pd[2]), .waiting(st == 2), .blk_end(blk == 63),
    .t(t), .u(u), .snap(go_send),
    .drop(pd_drop), .during(pd_during), .level(pd_level),
    .first(pd_first), .stable(pd_stable), .lockt(pd_lockt),
    .bmin(pd_bmin), .bmax(pd_bmax), .ph0(pd_ph0), .ph1(pd_ph1),
    .done(pd_done));

  // UART reporter, 115200 8N1 from 25 MHz: "H" + 145 hex + CR LF.
  reg [579:0] payload = 0;
  always @(posedge clk_25mhz)
    if (go_send) payload <= {
      w,
      pa_drop, pa_during, 3'b0, pa_level, pa_first, pa_stable, pa_lockt, 2'b0, pa_bmin, 2'b0, pa_bmax, 2'b0, pa_ph0, 2'b0, pa_ph1,
      pb_drop, pb_during, 3'b0, pb_level, pb_first, pb_stable, pb_lockt, 2'b0, pb_bmin, 2'b0, pb_bmax, 2'b0, pb_ph0, 2'b0, pb_ph1,
      pc_drop, pc_during, 3'b0, pc_level, pc_first, pc_stable, pc_lockt, 2'b0, pc_bmin, 2'b0, pc_bmax, 2'b0, pc_ph0, 2'b0, pc_ph1,
      pd_drop, pd_during, 3'b0, pd_level, pd_first, pd_stable, pd_lockt, 2'b0, pd_bmin, 2'b0, pd_bmax, 2'b0, pd_ph0, 2'b0, pd_ph1
    };
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [7:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "H";
    else if (ci <= 145) ch = hex(payload[579 - 4 * (ci - 1) -: 4]);
    else if (ci == 146) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0, sending = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz) begin
    line_done <= 0;
    if (go_send) begin sending <= 1; ci <= 0; end
    else if (sending && !busy) begin
      shift <= {1'b1, ch, 1'b0};
      busy <= 1; baud <= 0; bitn <= 0;
    end else if (busy) begin
      if (baud == 216) begin
        baud <= 0;
        tx <= shift[0];
        shift <= {1'b1, shift[9:1]};
        bitn <= bitn + 1;
        if (bitn == 9) begin
          busy <= 0;
          if (ci == 147) begin sending <= 0; line_done <= 1; end
          else ci <= ci + 1;
        end
      end else baud <= baud + 1;
    end
  end
  assign ftdi_rxd = tx;
  assign led = {pa_lock, pb_lock, pc_lock, pd_lock, 2'b0, st};
endmodule

// Counts CLKOP edges in its own domain and measures one event in the
// reference domain.
module obs(
  input sys, input co, input lock,
  input ev_start, input in_pulse, input waiting, input blk_end,
  input [19:0] t, input [19:0] u, input snap,
  output reg [19:0] drop, output reg [15:0] during, output reg level,
  output reg [19:0] first, output reg [19:0] stable, output reg [19:0] lockt,
  output reg [9:0] bmin, output reg [9:0] bmax,
  output reg [5:0] ph0, output reg [5:0] ph1,
  output done);
  reg [5:0] rb = 0, rg = 0, fb = 0, fg = 0;
  wire [5:0] rn = rb + 6'd1, fn = fb + 6'd1;
  always @(posedge co) begin rb <= rn; rg <= rn ^ (rn >> 1); end
  always @(negedge co) begin fb <= fn; fg <= fn ^ (fn >> 1); end

  function [5:0] g2b(input [5:0] g);
    g2b = {g[5], ^g[5:4], ^g[5:3], ^g[5:2], ^g[5:1], ^g[5:0]};
  endfunction
  reg [5:0] rs1 = 0, rs2 = 0, fs1 = 0, fs2 = 0;
  reg [1:0] ls = 0;
  always @(posedge sys) begin
    rs1 <= rg; rs2 <= rs1; fs1 <= fg; fs2 <= fs1; ls <= {ls[0], lock};
  end
  wire [5:0] r = g2b(rs2), f = g2b(fs2);
  reg [5:0] rp = 0, x = 0;
  wire [5:0] dr = r - rp;
  wire [5:0] ph = r + f - x;
  reg [9:0] acc = 0;
  wire [9:0] bsum = acc + dr;
  reg [3:0] good = 0;
  wire [3:0] good_n = (bsum >= 127 && bsum <= 129) ? (good == 15 ? good : good + 4'd1) : 4'd0;
  assign done = first != 20'hFFFFF && stable != 20'hFFFFF && lockt != 20'hFFFFF;
  initial begin
    drop = 0; during = 0; level = 0; first = 0; stable = 0; lockt = 0;
    bmin = 0; bmax = 0; ph0 = 0; ph1 = 0;
  end
  always @(posedge sys) begin
    rp <= r;
    x <= x + 6'd4;
    if (blk_end) begin acc <= 0; good <= good_n; end
    else acc <= bsum;
    if (ev_start) begin
      drop <= 20'hFFFFF; during <= 0; first <= 20'hFFFFF; stable <= 20'hFFFFF;
      lockt <= 20'hFFFFF; bmin <= 10'h3FF; bmax <= 0; ph0 <= ph;
    end else begin
      if (!ls[1] && drop == 20'hFFFFF) drop <= t;
      if (in_pulse && during != 16'hFFFF) during <= during + dr;
      if (blk_end) begin
        if (bsum < bmin) bmin <= bsum;
        if (bsum > bmax) bmax <= bsum;
      end
      if (waiting) begin
        if (u == 3) level <= r[0] ^ f[0];
        if (first == 20'hFFFFF && dr != 0) first <= u;
        if (stable == 20'hFFFFF && blk_end && good_n >= 4) stable <= u;
        if (lockt == 20'hFFFFF && ls[1]) lockt <= u;
      end
    end
    if (snap) ph1 <= ph;
  end
endmodule
