// What does a slice FF preload from M when LSRMODE=PRLD?
//
// Design table: 32 directly instantiated TRELLIS_FF, CLK=25 MHz, GSR=ENABLED,
// CLKMUX=CLK, LSRMUX=LSR. Each set of eight enumerates the Cartesian product:
//   SRMODE       REGSET     CEMUX
//   LSR_OVER_CE  RESET/SET  CE/1
//   ASYNC        RESET/SET  CE/1
// FFs 00..23 are three copies of those eight settings with LSRMODE=PRLD;
// FFs 24..31 use the same eight settings with LSRMODE=LSR as controls.
// Every FF receives independent registered DI, M, CE and LSR traffic fields.
// A kept identity TRELLIS_COMB drives each DI so nextpnr packs the FF with its
// LUT and preserves M as the separate preload input (SD=1). The identity LUT
// leaves its unused M port disconnected: LUT M and FF M share a physical wire,
// so tying the LUT port to zero would conflict with the FF preload traffic.
//
// Traffic: one 64-bit LFSR, x^64+x^63+x^61+x^60+1, seed 1D2CACE15EED0001.
// For FF i, take the low seven bits of step^(7*i)(L): DI=bit0, M=bit1,
// CE=(bits3:2 != 0), LSR=(bits6:4 == 0). CE is high about 3/4, LSR about 1/8.
// Advance 224 single-bit steps per clock; traffic is registered, initialized
// from the seed and updated from the next LFSR state, so traffic(k)=fields(L_k).
// CE and LSR are decoded before their fabric registers to avoid decode glitches
// on the asynchronous control pins.
//
// Line: P + 16 hex win_lfsr + 1024 hex Q + CR LF, 115200 8N1.
// Each of 128 samples is Q[31:0] (eight hex); sample zero is in the MSBs.
// Alignment: sample k is BEFORE rising edge k and uses traffic(step224^k(L_0)).
// Synchronous updates at edge k use traffic k; asynchronous responses can also
// use traffic k+1 before sample k+1. No startup Q value is assumed in fitting.
// The LFSR and FFs run through UART. Free-running records a new window after
// sending; variant s repeats the first window from configuration forever.
// Usage: python3 hs02_gen.py [s] > hs02[s].v
// name LSRMODE SRMODE REGSET CEMUX
// ff00 PRLD LSR_OVER_CE RESET CE
// ff01 PRLD LSR_OVER_CE RESET 1
// ff02 PRLD LSR_OVER_CE SET   CE
// ff03 PRLD LSR_OVER_CE SET   1
// ff04 PRLD ASYNC       RESET CE
// ff05 PRLD ASYNC       RESET 1
// ff06 PRLD ASYNC       SET   CE
// ff07 PRLD ASYNC       SET   1
// ff08 PRLD LSR_OVER_CE RESET CE
// ff09 PRLD LSR_OVER_CE RESET 1
// ff10 PRLD LSR_OVER_CE SET   CE
// ff11 PRLD LSR_OVER_CE SET   1
// ff12 PRLD ASYNC       RESET CE
// ff13 PRLD ASYNC       RESET 1
// ff14 PRLD ASYNC       SET   CE
// ff15 PRLD ASYNC       SET   1
// ff16 PRLD LSR_OVER_CE RESET CE
// ff17 PRLD LSR_OVER_CE RESET 1
// ff18 PRLD LSR_OVER_CE SET   CE
// ff19 PRLD LSR_OVER_CE SET   1
// ff20 PRLD ASYNC       RESET CE
// ff21 PRLD ASYNC       RESET 1
// ff22 PRLD ASYNC       SET   CE
// ff23 PRLD ASYNC       SET   1
// ff24 LSR  LSR_OVER_CE RESET CE
// ff25 LSR  LSR_OVER_CE RESET 1
// ff26 LSR  LSR_OVER_CE SET   CE
// ff27 LSR  LSR_OVER_CE SET   1
// ff28 LSR  ASYNC       RESET CE
// ff29 LSR  ASYNC       RESET 1
// ff30 LSR  ASYNC       SET   CE
// ff31 LSR  ASYNC       SET   1
module top(input clk_25mhz, output ftdi_rxd);
  function [63:0] shift7(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 7; j = j + 1)
        a = {a[62:0], a[63] ^ a[62] ^ a[60] ^ a[59]};
      shift7 = a;
    end
  endfunction
  function [127:0] expand(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 32; j = j + 1) begin
        expand[4*j] = a[0];
        expand[4*j+1] = a[1];
        expand[4*j+2] = |a[3:2];
        expand[4*j+3] = ~|a[6:4];
        a = shift7(a);
      end
    end
  endfunction
  function [63:0] step224(input [63:0] x);
    reg [63:0] a;
    integer j;
    begin
      a = x;
      for (j = 0; j < 32; j = j + 1) a = shift7(a);
      step224 = a;
    end
  endfunction
  reg [63:0] lfsr = 64'h1D2CACE15EED0001;
  wire [63:0] next_lfsr = step224(lfsr);
  reg [127:0] t = expand(64'h1D2CACE15EED0001);
  always @(posedge clk_25mhz) begin
    lfsr <= next_lfsr;
    t <= expand(next_lfsr);
  end
  wire [31:0] q, di;
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut00 (
    .A(t[0]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[0]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("CE")) ff00 (
    .CLK(clk_25mhz), .DI(di[0]), .M(t[1]),
    .CE(t[2]), .LSR(t[3]), .Q(q[0]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut01 (
    .A(t[4]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[1]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("1")) ff01 (
    .CLK(clk_25mhz), .DI(di[1]), .M(t[5]),
    .CE(t[6]), .LSR(t[7]), .Q(q[1]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut02 (
    .A(t[8]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[2]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("CE")) ff02 (
    .CLK(clk_25mhz), .DI(di[2]), .M(t[9]),
    .CE(t[10]), .LSR(t[11]), .Q(q[2]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut03 (
    .A(t[12]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[3]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("1")) ff03 (
    .CLK(clk_25mhz), .DI(di[3]), .M(t[13]),
    .CE(t[14]), .LSR(t[15]), .Q(q[3]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut04 (
    .A(t[16]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[4]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("CE")) ff04 (
    .CLK(clk_25mhz), .DI(di[4]), .M(t[17]),
    .CE(t[18]), .LSR(t[19]), .Q(q[4]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut05 (
    .A(t[20]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[5]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("1")) ff05 (
    .CLK(clk_25mhz), .DI(di[5]), .M(t[21]),
    .CE(t[22]), .LSR(t[23]), .Q(q[5]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut06 (
    .A(t[24]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[6]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("CE")) ff06 (
    .CLK(clk_25mhz), .DI(di[6]), .M(t[25]),
    .CE(t[26]), .LSR(t[27]), .Q(q[6]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut07 (
    .A(t[28]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[7]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("1")) ff07 (
    .CLK(clk_25mhz), .DI(di[7]), .M(t[29]),
    .CE(t[30]), .LSR(t[31]), .Q(q[7]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut08 (
    .A(t[32]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[8]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("CE")) ff08 (
    .CLK(clk_25mhz), .DI(di[8]), .M(t[33]),
    .CE(t[34]), .LSR(t[35]), .Q(q[8]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut09 (
    .A(t[36]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[9]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("1")) ff09 (
    .CLK(clk_25mhz), .DI(di[9]), .M(t[37]),
    .CE(t[38]), .LSR(t[39]), .Q(q[9]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut10 (
    .A(t[40]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[10]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("CE")) ff10 (
    .CLK(clk_25mhz), .DI(di[10]), .M(t[41]),
    .CE(t[42]), .LSR(t[43]), .Q(q[10]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut11 (
    .A(t[44]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[11]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("1")) ff11 (
    .CLK(clk_25mhz), .DI(di[11]), .M(t[45]),
    .CE(t[46]), .LSR(t[47]), .Q(q[11]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut12 (
    .A(t[48]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[12]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("CE")) ff12 (
    .CLK(clk_25mhz), .DI(di[12]), .M(t[49]),
    .CE(t[50]), .LSR(t[51]), .Q(q[12]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut13 (
    .A(t[52]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[13]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("1")) ff13 (
    .CLK(clk_25mhz), .DI(di[13]), .M(t[53]),
    .CE(t[54]), .LSR(t[55]), .Q(q[13]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut14 (
    .A(t[56]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[14]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("CE")) ff14 (
    .CLK(clk_25mhz), .DI(di[14]), .M(t[57]),
    .CE(t[58]), .LSR(t[59]), .Q(q[14]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut15 (
    .A(t[60]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[15]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("1")) ff15 (
    .CLK(clk_25mhz), .DI(di[15]), .M(t[61]),
    .CE(t[62]), .LSR(t[63]), .Q(q[15]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut16 (
    .A(t[64]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[16]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("CE")) ff16 (
    .CLK(clk_25mhz), .DI(di[16]), .M(t[65]),
    .CE(t[66]), .LSR(t[67]), .Q(q[16]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut17 (
    .A(t[68]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[17]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("1")) ff17 (
    .CLK(clk_25mhz), .DI(di[17]), .M(t[69]),
    .CE(t[70]), .LSR(t[71]), .Q(q[17]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut18 (
    .A(t[72]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[18]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("CE")) ff18 (
    .CLK(clk_25mhz), .DI(di[18]), .M(t[73]),
    .CE(t[74]), .LSR(t[75]), .Q(q[18]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut19 (
    .A(t[76]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[19]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("1")) ff19 (
    .CLK(clk_25mhz), .DI(di[19]), .M(t[77]),
    .CE(t[78]), .LSR(t[79]), .Q(q[19]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut20 (
    .A(t[80]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[20]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("CE")) ff20 (
    .CLK(clk_25mhz), .DI(di[20]), .M(t[81]),
    .CE(t[82]), .LSR(t[83]), .Q(q[20]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut21 (
    .A(t[84]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[21]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("1")) ff21 (
    .CLK(clk_25mhz), .DI(di[21]), .M(t[85]),
    .CE(t[86]), .LSR(t[87]), .Q(q[21]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut22 (
    .A(t[88]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[22]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("CE")) ff22 (
    .CLK(clk_25mhz), .DI(di[22]), .M(t[89]),
    .CE(t[90]), .LSR(t[91]), .Q(q[22]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut23 (
    .A(t[92]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[23]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("PRLD"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("1")) ff23 (
    .CLK(clk_25mhz), .DI(di[23]), .M(t[93]),
    .CE(t[94]), .LSR(t[95]), .Q(q[23]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut24 (
    .A(t[96]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[24]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("CE")) ff24 (
    .CLK(clk_25mhz), .DI(di[24]), .M(t[97]),
    .CE(t[98]), .LSR(t[99]), .Q(q[24]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut25 (
    .A(t[100]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[25]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("LSR_OVER_CE"),
    .REGSET("RESET"), .CEMUX("1")) ff25 (
    .CLK(clk_25mhz), .DI(di[25]), .M(t[101]),
    .CE(t[102]), .LSR(t[103]), .Q(q[25]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut26 (
    .A(t[104]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[26]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("CE")) ff26 (
    .CLK(clk_25mhz), .DI(di[26]), .M(t[105]),
    .CE(t[106]), .LSR(t[107]), .Q(q[26]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut27 (
    .A(t[108]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[27]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("LSR_OVER_CE"),
    .REGSET("SET"), .CEMUX("1")) ff27 (
    .CLK(clk_25mhz), .DI(di[27]), .M(t[109]),
    .CE(t[110]), .LSR(t[111]), .Q(q[27]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut28 (
    .A(t[112]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[28]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("CE")) ff28 (
    .CLK(clk_25mhz), .DI(di[28]), .M(t[113]),
    .CE(t[114]), .LSR(t[115]), .Q(q[28]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut29 (
    .A(t[116]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[29]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("ASYNC"),
    .REGSET("RESET"), .CEMUX("1")) ff29 (
    .CLK(clk_25mhz), .DI(di[29]), .M(t[117]),
    .CE(t[118]), .LSR(t[119]), .Q(q[29]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut30 (
    .A(t[120]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[30]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("CE")) ff30 (
    .CLK(clk_25mhz), .DI(di[30]), .M(t[121]),
    .CE(t[122]), .LSR(t[123]), .Q(q[30]));
  (* keep = 1 *) TRELLIS_COMB #(.MODE("LOGIC"), .INITVAL(16'hAAAA)) di_lut31 (
    .A(t[124]), .B(1'b0), .C(1'b0), .D(1'b0), .F(di[31]));
  (* keep = 1 *) TRELLIS_FF #(.GSR("ENABLED"), .CLKMUX("CLK"),
    .LSRMUX("LSR"), .LSRMODE("LSR"), .SRMODE("ASYNC"),
    .REGSET("SET"), .CEMUX("1")) ff31 (
    .CLK(clk_25mhz), .DI(di[31]), .M(t[125]),
    .CE(t[126]), .LSR(t[127]), .Q(q[31]));
  reg [63:0] win_lfsr = 64'h1D2CACE15EED0001;
  reg [6:0] u = 0;
  reg [4095:0] rec = 0;
  localparam REC = 0, SEND = 1;
  reg state = REC;
  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction
  reg [10:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "P";
    else if (ci < 17) ch = hex(win_lfsr[63 - 4 * (ci - 1) -: 4]);
    else if (ci < 1041) ch = hex(rec[4095 - 4 * (ci - 17) -: 4]);
    else if (ci == 1041) ch = 8'h0d;
    else ch = 8'h0a;
  end
  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;
  always @(posedge clk_25mhz)
    case (state)
      REC: begin
        if (u == 0) win_lfsr <= lfsr;
        rec <= {rec[4063:0], q};
        u <= u + 1;
        if (u == 127) begin state <= SEND; u <= 0; end
      end
      SEND:
        if (!busy) begin
          shift <= {1'b1, ch, 1'b0};
          busy <= 1; baud <= 0; bitn <= 0;
        end else if (baud == 216) begin
          baud <= 0;
          tx <= shift[0];
          shift <= {1'b1, shift[9:1]};
          bitn <= bitn + 1;
          if (bitn == 9) begin
            busy <= 0;
            if (ci == 1042) begin ci <= 0; state <= SEND; end
            else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  assign ftdi_rxd = tx;
endmodule
