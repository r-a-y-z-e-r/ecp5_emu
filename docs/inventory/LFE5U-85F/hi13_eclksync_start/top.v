// HI13: identical /2 chains, pre-running pin versus delayed DCCA clock.
// build.py sets GSR=DISABLED on every synthesized TRELLIS_FF.
module top(input clk, output ftdi_rxd);
  localparam N=64, MID=524288, END=1048576;
  wire refclk, gated, ep, eg, qp, qg;
  DCCA support_buffer(.CLKI(clk), .CE(1'b1), .CLKO(refclk));
  reg ce=0;
  DCCA delayed_buffer(.CLKI(clk), .CE(ce), .CLKO(gated));
  (* BEL="X0/Y46/ECLKSYNC1_BK7" *)
  ECLKSYNCB sync_p(.ECLKI(clk), .STOP(1'b0), .ECLKO(ep));
  (* BEL="X0/Y47/ECLKSYNC0_BK6" *)
  ECLKSYNCB sync_g(.ECLKI(gated), .STOP(1'b0), .ECLKO(eg));
  (* BEL="X0/Y46/CLKDIV1" *)
  CLKDIVF #(.DIV("2.0")) div_p(.CLKI(ep), .RST(1'b0), .ALIGNWD(1'b0), .CDIVX(qp));
  (* BEL="X0/Y46/CLKDIV0" *)
  CLKDIVF #(.DIV("2.0")) div_g(.CLKI(eg), .RST(1'b0), .ALIGNWD(1'b0), .CDIVX(qg));

  reg freeze=0, mid_done=0, finished=0;
  (* keep=1 *) reg [31:0] i=0, a=0, ce_at=0, gr=0, pr=0, pf=0, qr=0, qf=0;
  // i counts observable pin rises from fabric activation, NOT configuration traffic.
  // a counts pin rises strictly after CE; gr independently qualifies the DCCA.
  always @(posedge refclk) if (!freeze) begin i<=i+1'b1; if (ce) a<=a+1'b1; end
  always @(posedge gated) if (!freeze) gr<=gr+1'b1;
  always @(posedge qp) if (!freeze) pr<=pr+1'b1;
  always @(negedge qp) if (!freeze) pf<=pf+1'b1;
  always @(posedge qg) if (!freeze) qr<=qr+1'b1;
  always @(negedge qg) if (!freeze) qf<=qf+1'b1;

  reg [255:0] mid=0, last=0;
  reg [4:0] settle=0;
  // CE changes in the low half-cycle, away from the next pin rising edge.
  // /2 CDIVX transitions occur only at pin rises: freeze has 20 ns of margin.
  always @(negedge refclk) begin
    if (i==N) begin ce<=1; ce_at<=i; end
    if (!mid_done && i==MID) begin mid<={i,a,ce_at,gr,pr,pf,qr,qf}; mid_done<=1; end
    if (!freeze && i==END) freeze<=1;
  end
  always @(posedge refclk) begin
    if (freeze && !finished) begin
      settle<=settle+1'b1;
      if (&settle) begin last<={i,a,ce_at,gr,pr,pf,qr,qf}; finished<=1; end
    end
  end

  // Independent pin-clock heartbeat and UART: no divided-clock progress required.
  reg [31:0] live=0;
  always @(posedge refclk) live<=live+1'b1;
  wire [511:0] payload={mid,last};
  reg [511:0] word=0;
  reg [39:0] heartbeat=0;
  reg [7:0] ci=8'hff, baud=0;
  reg [3:0] bitn=0;
  reg busy=0, sending=0, full_record=0;
  reg [9:0] shift=10'h3ff;
  assign ftdi_rxd=busy ? shift[0] : 1'b1;
  function [7:0] hex(input [3:0] n);
    hex=n<10 ? 8'h30+n : 8'h41+n-10;
  endfunction
  reg [7:0] ch;
  always @* begin
    if (ci==0) ch="H";
    else if (ci==1) ch=full_record ? "C" : "B";
    else if (ci==2) ch="D";
    else if (ci==3) ch=" ";
    else if (full_record) begin
      if (ci<132) ch=hex(word[511:508]);
      else if (ci==132) ch=13;
      else ch=10;
    end else begin
      if (ci==6) ch=" ";
      else if (ci<15) ch=hex(heartbeat[39:36]);
      else if (ci==15) ch=13;
      else ch=10;
    end
  end
  always @(posedge refclk) begin
    if (!sending) begin
      full_record<=finished; word<=payload;
      heartbeat<={5'b0,finished,freeze,ce,live}; ci<=0; sending<=1;
    end else if (!busy) begin shift<={1'b1,ch,1'b0}; bitn<=0; baud<=216; busy<=1; end
    else if (baud!=0) baud<=baud-1'b1;
    else begin
      baud<=216; shift<={1'b1,shift[9:1]}; bitn<=bitn+1'b1;
      if (bitn==9) begin
        busy<=0; ci<=ci+1'b1;
        if (full_record && ci>=4 && ci<132) word<=word<<4;
        if (!full_record && ci>=4 && ci<15 && ci!=6) heartbeat<=heartbeat<<4;
        if (ci==(full_record ? 133 : 16)) sending<=0;
      end
    end
  end
endmodule
