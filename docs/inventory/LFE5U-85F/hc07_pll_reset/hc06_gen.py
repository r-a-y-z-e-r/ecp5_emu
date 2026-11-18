"""Writes hc06.v: four EHXPLLL on the 25 MHz reference, RST or STDBY pulsed.

Usage: python3 hc06_gen.py > hc06.v; python3 hc06_gen.py b > hc06b.v

PLLs (each CLKOP = 50 MHz from a 600 MHz VCO):
  pa: RST pulsed, PLLRST_ENA=DISABLED, internal feedback through CLKOS3 (div 1)
  pb: RST pulsed, PLLRST_ENA=ENABLED, the same feedback
  pc: STDBY pulsed, STDBY_ENABLE=DISABLED, feedback through CLKOP as ecppll writes it
  pd: STDBY pulsed, STDBY_ENABLE=ENABLED, the same feedback

One pulse drives all four. Its width is drawn from a table of sixteen. After
it falls the sequencer waits until every PLL runs at its rate and shows LOCK
(or 2^20 reference cycles), then reports one line and waits 2.6..5.2 ms.

Line: H, the width (5 hex), then 35 hex per PLL (pa, pb, pc, pd):
  drop   5  reference cycles from the pulse's rise to the first LOCK low (FFFFF: none)
  during 4  CLKOP rising edges seen while the pulse was high (saturates)
  level  1  rising minus falling CLKOP edges, mod 2, 3 cycles after the fall
  first  5  cycles from the fall to the first cycle with a new rising edge
  stable 5  cycles from the fall to the end of the fourth 64-cycle block in a
            row with 127..129 rising edges
  lock   5  cycles from the fall to the first LOCK high
  bmin   3  fewest rising edges in a 64-cycle block during the event
  bmax   3  most
  ph0    2  (rising + falling edges - 4 * cycles) mod 64 at the pulse's rise
  ph1    2  the same at the report
All counts are of the CLKOP edges seen through 2-flop synchronizers of gray
counters, so they lag the PLL by two to three reference cycles.

In hc06 the report takes ph1 from the event before (it is latched at the same
edge), "lock" is 0 when LOCK had not yet been seen low at the fall (W = 1),
and "during" wraps at 65536.

Variant b (hc06b.v) has every enable ENABLED and fixes those three:
  pa: RST, feedback through CLKOP (as pc and pd of hc06)
  pb: STDBY, feedback through CLKOS3 (as pa and pb of hc06)
  pc: RST, feedback through CLKOS3, CLKI_DIV 5 (a 5 MHz phase detector)
  pd: RST, feedback through CLKOP, CLKI_DIV 5
Its counters of falling edges start at the first rising edge, so "level" is
CLKOP's level and ph1 - ph0 is the edges lost in the event. Each PLL's field
is 72 hex: the 35 above (lock now counts from the fourth cycle after the fall,
during saturates at FFFF, ph1 is this event's), then
  lrise  1  LOCK rises seen in the event (saturates at F)
  ramp  36  rising CLKOP edges in each of the twelve 1024-cycle windows after
            the fall, 3 hex each, nominal 800
The report waits for all twelve windows.
"""

WIDTHS = [1, 2, 3, 4, 5, 8, 16, 32, 100, 256, 1000, 4096, 10000, 25000, 50000, 100000]

# name, pulsed port, enable parameter and value, feedback
PLLS = [
    ("pa", "RST", "PLLRST_ENA", "DISABLED", "os3"),
    ("pb", "RST", "PLLRST_ENA", "ENABLED", "os3"),
    ("pc", "STDBY", "STDBY_ENABLE", "DISABLED", "op"),
    ("pd", "STDBY", "STDBY_ENABLE", "ENABLED", "op"),
]
# the same, with CLKI_DIV last
PLLS_B = [
    ("pa", "RST", "PLLRST_ENA", "ENABLED", "op", 1),
    ("pb", "STDBY", "STDBY_ENABLE", "ENABLED", "os3", 1),
    ("pc", "RST", "PLLRST_ENA", "ENABLED", "os3", 5),
    ("pd", "RST", "PLLRST_ENA", "ENABLED", "op", 5),
]


def pll(name, port, enable, value, feedback, clki=1, variant=""):
    rst = "pulse" if port == "RST" else "1'b0"
    stdby = "pulse" if port == "STDBY" else "1'b0"
    other = "STDBY_ENABLE" if enable == "PLLRST_ENA" else "PLLRST_ENA"
    if feedback == "os3":
        # Internal feedback through CLKOS3 at the VCO rate, CPHASE = div - 1.
        attrs = '(* ICP_CURRENT="6" *) (* LPF_RESISTOR="16" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)'
        params = f"""
    .FEEDBK_PATH("INT_OS3"), .CLKFB_DIV({24 * clki}), .CLKI_DIV({clki}),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(11), .CLKOP_FPHASE(0),
    .CLKOS3_ENABLE("ENABLED"), .CLKOS3_DIV(1), .CLKOS3_CPHASE(0), .CLKOS3_FPHASE(0),"""
        fb = f"{name}_fb"
        clkfb = fb
        intfb = fb
    else:
        # ecppll -i 25 -o 50: feedback through CLKOP, CPHASE = div / 2 - 1.
        attrs = '(* ICP_CURRENT="12" *) (* LPF_RESISTOR="8" *) (* MFG_ENABLE_FILTEROPAMP="1" *) (* MFG_GMCREF_SEL="2" *)'
        params = f"""
    .FEEDBK_PATH("CLKOP"), .CLKFB_DIV({2 * clki}), .CLKI_DIV({clki}),
    .CLKOP_ENABLE("ENABLED"), .CLKOP_DIV(12), .CLKOP_CPHASE(5), .CLKOP_FPHASE(0),"""
        fb = None
        clkfb = f"{name}_clk"
        intfb = ""
    decl = f"  wire {name}_clk, {name}_lock;\n"
    if fb:
        decl += f"  wire {fb};\n"
    decl += f"""  wire [19:0] {name}_drop, {name}_first, {name}_stable, {name}_lockt;
  wire [15:0] {name}_during;
  wire {name}_level, {name}_done;
  wire [9:0] {name}_bmin, {name}_bmax;
  wire [5:0] {name}_ph0, {name}_ph1;
"""
    module, extra = "obs", ""
    if variant == "b":
        decl += f"""  wire [3:0] {name}_lrise;
  wire [143:0] {name}_ramp;
"""
        module = "obs_b"
        extra = f", .lrise({name}_lrise), .ramp({name}_ramp)"
    return decl, f"""  (* FREQUENCY_PIN_CLKI="25" *) (* FREQUENCY_PIN_CLKOP="50" *)
  {attrs}
  EHXPLLL #(
    .{enable}("{value}"), .{other}("DISABLED"),
    .INTFB_WAKE("DISABLED"), .DPHASE_SOURCE("DISABLED"),
    .OUTDIVIDER_MUXA("DIVA"), .OUTDIVIDER_MUXB("DIVB"),
    .OUTDIVIDER_MUXC("DIVC"), .OUTDIVIDER_MUXD("DIVD"),{params}
    .CLKOP_TRIM_POL("RISING"), .CLKOP_TRIM_DELAY(0)
  ) {name}_pll (
    .RST({rst}), .STDBY({stdby}), .CLKI(clk_25mhz),
    .CLKOP({name}_clk), .CLKFB({clkfb}), .CLKINTFB({intfb}),
    .PHASESEL0(1'b0), .PHASESEL1(1'b0), .PHASEDIR(1'b0), .PHASESTEP(1'b0),
    .PHASELOADREG(1'b0), .PLLWAKESYNC(1'b0), .ENCLKOP(1'b0),
    .LOCK({name}_lock)
  );
  {module} {name}_obs(
    .sys(clk_25mhz), .co({name}_clk), .lock({name}_lock),
    .ev_start(ev_start), .in_pulse(pd[2]), .waiting(st == 2), .blk_end(blk == 63),
    .t(t), .u(u), .snap(go_send),
    .drop({name}_drop), .during({name}_during), .level({name}_level),
    .first({name}_first), .stable({name}_stable), .lockt({name}_lockt),
    .bmin({name}_bmin), .bmax({name}_bmax), .ph0({name}_ph0), .ph1({name}_ph1),
    .done({name}_done){extra});
"""


OBS = """
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
"""


OBS_B = """
// As obs, with the fixes and additions of variant b.
module obs_b(
  input sys, input co, input lock,
  input ev_start, input in_pulse, input waiting, input blk_end,
  input [19:0] t, input [19:0] u, input snap,
  output reg [19:0] drop, output reg [15:0] during, output reg level,
  output reg [19:0] first, output reg [19:0] stable, output reg [19:0] lockt,
  output reg [9:0] bmin, output reg [9:0] bmax,
  output reg [5:0] ph0, output reg [5:0] ph1,
  output done, output reg [3:0] lrise, output reg [143:0] ramp);
  reg [5:0] rb = 0, rg = 0, fb = 0, fg = 0;
  reg started = 0;
  wire [5:0] rn = rb + 6'd1, fn = fb + 6'd1;
  always @(posedge co) begin rb <= rn; rg <= rn ^ (rn >> 1); started <= 1; end
  always @(negedge co) if (started) begin fb <= fn; fg <= fn ^ (fn >> 1); end

  function [5:0] g2b(input [5:0] g);
    g2b = {g[5], ^g[5:4], ^g[5:3], ^g[5:2], ^g[5:1], ^g[5:0]};
  endfunction
  reg [5:0] rs1 = 0, rs2 = 0, fs1 = 0, fs2 = 0;
  reg [2:0] ls = 0;
  always @(posedge sys) begin
    rs1 <= rg; rs2 <= rs1; fs1 <= fg; fs2 <= fs1; ls <= {ls[1:0], lock};
  end
  wire [5:0] r = g2b(rs2), f = g2b(fs2);
  reg [5:0] rp = 0, x = 0;
  wire [5:0] dr = r - rp;
  wire [5:0] ph = r + f - x;
  reg [9:0] acc = 0;
  wire [9:0] bsum = acc + dr;
  reg [3:0] good = 0;
  wire [3:0] good_n = (bsum >= 127 && bsum <= 129) ? (good == 15 ? good : good + 4'd1) : 4'd0;
  wire [16:0] during_n = during + dr;
  reg [11:0] wacc = 0;
  wire [11:0] wacc_n = wacc + dr;
  assign done = first != 20'hFFFFF && stable != 20'hFFFFF && lockt != 20'hFFFFF;
  initial begin
    drop = 0; during = 0; level = 0; first = 0; stable = 0; lockt = 0;
    bmin = 0; bmax = 0; ph0 = 0; ph1 = 0; lrise = 0; ramp = 0;
  end
  always @(posedge sys) begin
    rp <= r;
    x <= x + 6'd4;
    if (blk_end) begin acc <= 0; good <= good_n; end
    else acc <= bsum;
    if (ev_start) begin
      drop <= 20'hFFFFF; during <= 0; first <= 20'hFFFFF; stable <= 20'hFFFFF;
      lockt <= 20'hFFFFF; bmin <= 10'h3FF; bmax <= 0; ph0 <= ph;
      lrise <= 0; ramp <= 0; wacc <= 0;
    end else begin
      if (!ls[1] && drop == 20'hFFFFF) drop <= t;
      if (ls[1] && !ls[2] && lrise != 15) lrise <= lrise + 4'd1;
      if (in_pulse) during <= during_n[16] ? 16'hFFFF : during_n[15:0];
      if (blk_end) begin
        if (bsum < bmin) bmin <= bsum;
        if (bsum > bmax) bmax <= bsum;
      end
      if (waiting) begin
        if (u == 3) level <= r[0] ^ f[0];
        if (first == 20'hFFFFF && dr != 0) first <= u;
        if (stable == 20'hFFFFF && blk_end && good_n >= 4) stable <= u;
        if (lockt == 20'hFFFFF && ls[1] && u >= 4) lockt <= u;
        if (u < 12288) begin
          if (u[9:0] == 10'd1023) begin ramp <= {ramp[131:0], wacc_n}; wacc <= 0; end
          else wacc <= wacc_n;
        end
      end
    end
    if (snap) ph1 <= ph;
  end
endmodule
"""


def top(variant=""):
    plls_used = PLLS_B if variant == "b" else PLLS
    width_cases = "\n".join(f"    4'd{i}: wtab = 20'd{w};" for i, w in enumerate(WIDTHS))
    fields = []
    for name, *_ in plls_used:
        fields.append(
            f"{name}_drop, {name}_during, 3'b0, {name}_level, {name}_first, {name}_stable, "
            f"{name}_lockt, 2'b0, {name}_bmin, 2'b0, {name}_bmax, 2'b0, {name}_ph0, 2'b0, {name}_ph1"
            + (f", {name}_lrise, {name}_ramp" if variant == "b" else "")
        )
    payload = ",\n      ".join(fields)
    done = " && ".join(f"{name}_done" for name, *_ in plls_used)
    if variant == "b":
        parts = [pll(*p[:5], clki=p[5], variant="b") for p in plls_used]
    else:
        parts = [pll(*p) for p in plls_used]
    decls = "".join(d for d, _ in parts)
    plls = "".join(b for _, b in parts)
    per = 72 if variant == "b" else 35
    nhex = 5 + per * len(plls_used)
    title = "HC06b: EHXPLLL RST and STDBY enabled" if variant == "b" else "HC06: EHXPLLL RST and STDBY"
    least = "12288" if variant == "b" else "1023"
    latch = "send_d" if variant == "b" else "go_send"
    cbits = 8 if nhex + 2 < 256 else 9
    text = f"""// {title} while running. Written by hc06_gen.py; the
// line format is described there.
module top(input clk_25mhz, output ftdi_rxd, output [7:0] led);
{decls}
  // Sequencer: gap, pulse, wait, send.
  reg [31:0] lfsr = 32'h1F2E3D4C;
  always @(posedge clk_25mhz)
    lfsr <= {{lfsr[30:0], lfsr[31] ^ lfsr[21] ^ lfsr[1] ^ lfsr[0]}};
  reg [19:0] wtab;
  always @* case (lfsr[3:0])
{width_cases}
  endcase
  reg [1:0] st = 0;
  reg [19:0] t = 0, u = 0, w = 0;
  reg [16:0] gap = 17'd65535;
  reg pulse = 0;
  reg [2:0] pd = 0;
  reg [5:0] blk = 0;
  wire ev_start = st == 0 && gap == 0;
  wire all_done = {done};
  wire go_send = st == 2 && ((all_done && u >= {least}) || u == 20'hFFFFF);
  reg line_done = 0;
  always @(posedge clk_25mhz) begin
    blk <= blk + 1;
    pd <= {{pd[1:0], pulse}};
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
      3: if (line_done) begin st <= 0; gap <= {{1'b1, lfsr[15:0]}}; end
    endcase
  end
{plls}
  // UART reporter, 115200 8N1 from 25 MHz: "H" + {nhex} hex + CR LF.
  reg [{nhex * 4 - 1}:0] payload = 0;
  always @(posedge clk_25mhz)
    if ({latch}) payload <= {{
      w,
      {payload}
    }};
  function [7:0] hex(input [3:0] n);
    hex = n < 10 ? 8'h30 + n : 8'h37 + n;
  endfunction
  reg [{cbits - 1}:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "H";
    else if (ci <= {nhex}) ch = hex(payload[{nhex * 4 - 1} - 4 * (ci - 1) -: 4]);
    else if (ci == {nhex + 1}) ch = 8'h0d;
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
      shift <= {{1'b1, ch, 1'b0}};
      busy <= 1; baud <= 0; bitn <= 0;
    end else if (busy) begin
      if (baud == 216) begin
        baud <= 0;
        tx <= shift[0];
        shift <= {{1'b1, shift[9:1]}};
        bitn <= bitn + 1;
        if (bitn == 9) begin
          busy <= 0;
          if (ci == {nhex + 2}) begin sending <= 0; line_done <= 1; end
          else ci <= ci + 1;
        end
      end else baud <= baud + 1;
    end
  end
  assign ftdi_rxd = tx;
  assign led = {{pa_lock, pb_lock, pc_lock, pd_lock, 2'b0, st}};
endmodule
"""
    if variant == "b":
        # The payload is latched one edge after ph1.
        text = text.replace(
            "  reg line_done = 0;\n",
            "  reg line_done = 0;\n  reg send_d = 0;\n"
            "  always @(posedge clk_25mhz) send_d <= go_send;\n")
    return text


if __name__ == "__main__":
    import sys
    variant = sys.argv[1] if len(sys.argv) > 1 else ""
    print(top(variant) + (OBS_B if variant == "b" else OBS), end="")
