// A 31-tap low-pass FIR you can watch, built twice on the same chip.
// Filter 1 computes all 31 products in parallel on 31 MULT18X18D; filter 2
// time-shares ONE MULT18X18D with a hand-instantiated ALU54B accumulator.
// The chip subtracts the two results on every sample and counts the
// disagreements since the last restart (MISMATCH in the footer): zero means
// the two implementations agree bit for bit.
//
// The UART scope locks its time base to the tone: the 64 columns are always
// exactly two periods (one column = period/32, strobed off the tone's own
// DDS phase, so the trace cannot drift). Above 32 Hz a column would be
// shorter than one sample; then every sample is a column and the header
// says how many whole periods the window holds. Every frame shows a freshly
// captured, complete window: after a restart or any key the scope waits for
// 64 new columns before drawing, and sampling never stops while it prints.
//
// fs = 2 kHz rather than the sketch's 1 kHz: the emulator's fast mode runs
// this DSP-dense design at only 0.2-0.4 MHz, and at 1 kHz the check's
// frames would need twice the wall time. Every per-second figure (tones,
// cutoff) simply doubles; the normalized cutoff is unchanged.
module top(input clk_25mhz,input ftdi_txd,output ftdi_rxd,
           input [6:0] btn,output reg [7:0] led=0);
  wire clk=clk_25mhz;
  reg [5:0] reset_count=0;
  wire por=!(&reset_count);
  always @(posedge clk) if(por) reset_count<=reset_count+1;
  localparam WAIT=0,IDLE=1,KEYLINE=2,COPY=3,HEADER=4,NUMBERS=5,ART=6,FOOTER=7;
  reg [2:0] state=WAIT;
  wire [7:0] key;
  wire key_valid,tx_ready,overflow;
  // Keys are acted on one per frame boundary, in arrival order. Exception:
  // the very first key after configuration is taken at once, so a restart
  // typed early does not wait behind a whole power-on capture.
  reg greeted=0;
  wire pop=(state==IDLE||(state==WAIT&&!greeted)) && tx_ready && key_valid;
  wire restart=pop && key=="r";
  wire reset=por||restart;
  reg [7:0] tx_data;
  reg tx_valid;
  gallery_io #(.K1("f"),.K2("n"),.K3("u"),.K4("j"),.K5("1"),.K6("2"))
    io(clk,por,ftdi_txd,ftdi_rxd,btn,tx_data,tx_valid,tx_ready,key,key_valid,pop,overflow);
  reg filtered=1,selected=0,square=0;
  reg [1:0] noise=1;
  reg [2:0] tone=2;
  reg [31:0] phase=0,lfsr=32'h1acebeef;
  reg [13:0] divider=0;
  wire tick=divider==12499;
  // 2^32/2000 phase counts per sample is a 1 Hz tone; the ladder doubles to 128 Hz.
  wire [31:0] increment=32'd2147484<<tone;
  (* ram_style="block" *) reg signed [17:0] sine[0:255];
  initial $readmemh("sine.hex",sine);
  reg signed [17:0] sine_q=0;
  always @(posedge clk) sine_q<=sine[phase[31:24]];
  wire signed [17:0] random_value=$signed(lfsr[17:0]) >>> (noise==1 ? 5:2);
  wire signed [17:0] square_value=phase[31] ? -18'sd16384 : 18'sd16384;
  wire signed [17:0] sample=sine_q+(noise==0 ? 18'sd0:random_value)+(square ? square_value:18'sd0);
  reg signed [17:0] raw=0;
  // The phase the sample in `raw` was generated from: by the time the
  // filters' valid arrives, `phase` has already advanced to the next sample.
  reg [31:0] sphase=0;
  always @(posedge clk) begin
    if(reset) begin divider<=0;phase<=0;sphase<=0;lfsr<=32'h1acebeef;raw<=0;end
    else if(tick) begin
      divider<=0;sphase<=phase;phase<=phase+increment;raw<=sample;
      lfsr<={lfsr[30:0],lfsr[31]^lfsr[21]^lfsr[1]^lfsr[0]};
    end else divider<=divider+1;
  end
  wire valid;
  wire signed [17:0] parallel_out,serial_out;
  wire [31:0] mismatches;
  filters fir(clk,reset,tick,sample,valid,parallel_out,serial_out,mismatches);
  wire signed [17:0] chosen=selected ? serial_out:parallel_out;
  wire signed [17:0] displayed=filtered ? chosen:raw;
  // Sixteen rows over +/-65536: a full-scale sine fills the screen.
  function [3:0] ordinate(input signed [17:0] v);
    reg signed [6:0] s;
    begin s=(v>>>13)+7'sd8;
      ordinate=s<0 ? 0:s>15 ? 15:s[3:0];end
  endfunction
  // Capture: one column per strobe, 64 columns a window. The strobe is the
  // sample's phase crossing a 1/32-period boundary, so a window is exactly
  // two periods of the tone at any frequency. At 64 and 128 Hz a column
  // would be shorter than a sample, so every sample becomes a column.
  reg [3:0] cap_in[0:63],cap_out[0:63];
  reg [3:0] pic_in[0:63],pic_out[0:63];
  reg [5:0] wcol=0;
  reg [4:0] col_bin=0;
  reg full=0;
  reg signed [17:0] min_in=0,max_in=0,min_out=0,max_out=0;
  wire fast=tone>=6;
  wire [4:0] bin=sphase[31:27];
  wire take=full && state==WAIT;
  integer j;
  initial for(j=0;j<64;j=j+1) begin
    cap_in[j]=8;cap_out[j]=8;pic_in[j]=8;pic_out[j]=8;
  end
  always @(posedge clk) begin
    // Every key asks for a fresh window: the next frame waits for 64 new columns.
    if(reset||pop) begin wcol<=0;full<=0;col_bin<=0;end
    else if(take) begin full<=0;wcol<=0;end
    else if(valid && !full && (fast || bin!=col_bin)) begin
      col_bin<=bin;
      cap_in[wcol]<=ordinate(raw);
      cap_out[wcol]<=ordinate(chosen);
      if(wcol==0) begin
        min_in<=raw;max_in<=raw;min_out<=chosen;max_out<=chosen;
      end else begin
        if(raw<min_in) min_in<=raw;
        if(raw>max_in) max_in<=raw;
        if(chosen<min_out) min_out<=chosen;
        if(chosen>max_out) max_out<=chosen;
      end
      if(wcol==63) full<=1;
      wcol<=wcol+1;
    end
  end
  reg [6:0] pos=0;
  reg [5:0] col=0,copy_col=0;
  reg [3:0] row=0;
  reg [7:0] last_key=0;
  reg [31:0] frame=0,held_mismatch=0;
  reg started=0;
  function [7:0] hex(input [3:0] n);
    hex=n<10 ? "0"+n:"A"+n-10;
  endfunction
  function [7:0] header(input [6:0] p);
    reg [8*62-1:0] s;
    begin s="FRAME 0x00000000 FIR 1 LED F TONE 000Hz NOISE 0 SQ 0 FS 2000\r\n";
      header=s[8*(62-p)-1 -:8];end
  endfunction
  function [7:0] line2(input [6:0] p);
    reg [8*53-1:0] s;
    begin s="IN 000000 OUT 000000 WINDOW 0 PERIODS COL PERIOD/32\r\n";
      line2=s[8*(53-p)-1 -:8];end
  endfunction
  function [7:0] persample(input [3:0] p);
    reg [8*9-1:0] s;
    begin s="1 SAMPLE ";persample=s[8*(9-p)-1 -:8];end
  endfunction
  function [7:0] footer(input [6:0] p);
    reg [8*21-1:0] s;
    begin s="MISMATCH 0x00000000\r\n";footer=s[8*(21-p)-1 -:8];end
  endfunction
  // One decimal decade of v: the digit of v/s is the count of the nine
  // multiples s..9s that fit in v (nine PARALLEL comparators, not a
  // subtractor chain — Fmax), and the remainder feeds the next decade.
  // COPY runs this once per clock for six clocks, so the full six-digit
  // conversion never sits on one combinational path. The multiples are
  // written as adds: a `*` here would cost the demo's DSP multipliers.
  // They are 20 bits wide: 9 x 100 000 does not fit in 18.
  function [21:0] decade(input [17:0] v,input [17:0] s);
    reg [19:0] vv,s2,s3,s4,s5,s6,s7,s8,s9,ds,rr;
    reg [3:0] d;
    begin
      vv={2'b0,v};
      s2=s+s;s3=s2+s;s4=s2+s2;s5=s4+s;s6=s4+s2;s7=s4+s3;s8=s4+s4;s9=s8+s;
      d=(vv>=s)+(vv>=s2)+(vv>=s3)+(vv>=s4)+(vv>=s5)+(vv>=s6)+(vv>=s7)+(vv>=s8)+(vv>=s9);
      ds=(d[0] ? s:0)+(d[1] ? s2:0)+(d[2] ? s4:0)+(d[3] ? s8:0);
      // The remainder is below s (<= 100 000), so its low 18 bits hold it.
      rr=vv-ds;
      decade={d,rr[17:0]};
    end
  endfunction
  reg [17:0] rem_in=0,rem_out=0,scale=0;
  reg [23:0] bcd_in=0,bcd_out=0;
  wire [21:0] dec_in=decade(rem_in,scale),dec_out=decade(rem_out,scale);
  wire [7:0] hz=8'd1<<tone;
  // The digit being printed in NUMBERS and the value it comes from.
  wire [23:0] bcd_num=pos<14 ? bcd_in:bcd_out;
  wire [2:0] dpos=pos<14 ? pos-7'd3:pos-7'd14;
  wire [3:0] dig=bcd_num[4*(3'd5-dpos) +:4];
  always @* begin
    tx_valid=0;tx_data=0;
    case(state)
      KEYLINE:begin
        tx_valid=1;
        case(pos) 0:tx_data="K";1:tx_data="E";2:tx_data="Y";3:tx_data=" ";
          4:tx_data=last_key;5:tx_data=13;default:tx_data=10;endcase
      end
      HEADER:begin
        tx_valid=1;tx_data=header(pos);
        if(pos>=8 && pos<16) tx_data=hex(frame>>((15-pos)*4));
        if(pos==21) tx_data=selected ? "2":"1";
        if(pos==27) tx_data=filtered ? "F":"R";
        if(pos==34) tx_data="0"+hz/100;
        if(pos==35) tx_data="0"+(hz/10)%10;
        if(pos==36) tx_data="0"+hz%10;
        if(pos==46) tx_data="0"+noise;
        if(pos==51) tx_data=square ? "1":"0";
      end
      NUMBERS:begin
        tx_valid=1;tx_data=line2(pos);
        // Leading-zero suppression; the last digit always prints.
        if(pos>=3 && pos<=8)
          tx_data=(pos==8 || started || dig!=0) ? "0"+dig:" ";
        if(pos>=14 && pos<=19)
          tx_data=(pos==19 || started || dig!=0) ? "0"+dig:" ";
        if(pos==28) tx_data=tone==7 ? "4":"2";
        if(pos>=42 && pos<=50 && fast) tx_data=persample(pos-42);
      end
      ART:begin
        tx_valid=1;
        if(pos==64) tx_data=13;
        else if(pos==65) tx_data=10;
        else if((filtered ? pic_out[col]:pic_in[col])==row) tx_data="#";
        else if(pic_in[col]==row) tx_data=".";
        else tx_data=" ";
      end
      FOOTER:begin
        tx_valid=1;tx_data=footer(pos);
        if(pos>=11 && pos<19) tx_data=hex(held_mismatch>>((18-pos)*4));
      end
    endcase
  end
  always @(posedge clk) begin
    if(por) begin state<=WAIT;frame<=0;greeted<=0;started<=0;end
    else begin
      if(pop) begin
        greeted<=1;
        last_key<=key;pos<=0;state<=KEYLINE;
        case(key)
          "r":begin filtered<=1;selected<=0;square<=0;noise<=1;tone<=2;frame<=0;end
          "f":filtered<=!filtered;
          "n":noise<=noise==2 ? 0:noise+1;
          "u":if(tone!=7) tone<=tone+1;
          "j":if(tone!=0) tone<=tone-1;
          "1":selected<=0;
          "2":selected<=1;
          "q":square<=!square;
        endcase
      end else case(state)
      WAIT:if(full) begin
        frame<=frame+1;copy_col<=0;held_mismatch<=mismatches;
        // max-min computed in 19 bits; the value never exceeds 262143, so
        // the low 18 bits hold the peak-to-peak amplitude of the window.
        // COPY's first six clocks turn the two remainders into digits.
        rem_in<=$signed({max_in[17],max_in})-$signed({min_in[17],min_in});
        rem_out<=$signed({max_out[17],max_out})-$signed({min_out[17],min_out});
        scale<=100000;
        state<=COPY;
      end
      IDLE:if(tx_ready) state<=WAIT;
      KEYLINE:if(tx_ready) begin
        if(pos==6) begin
          state<=WAIT;
          pos<=0;
        end else pos<=pos+1;
      end
      COPY:begin
        // The capture side was released at `take`; its first rewrite of
        // cap_in[0] is a full column interval (>= 12 500 clocks) away.
        pic_in[copy_col]<=cap_in[copy_col];
        pic_out[copy_col]<=cap_out[copy_col];
        if(copy_col<6) begin
          bcd_in[4*(3'd5-copy_col[2:0]) +:4]<=dec_in[21:18];
          rem_in<=dec_in[17:0];
          bcd_out[4*(3'd5-copy_col[2:0]) +:4]<=dec_out[21:18];
          rem_out<=dec_out[17:0];
          case(copy_col)
            0:scale<=10000;1:scale<=1000;2:scale<=100;3:scale<=10;
            default:scale<=1;
          endcase
        end
        if(copy_col==63) begin state<=HEADER;pos<=0;end
        else copy_col<=copy_col+1;
      end
      HEADER:if(tx_ready) begin
        if(pos==61) begin state<=NUMBERS;pos<=0;end
        else pos<=pos+1;
      end
      NUMBERS:if(tx_ready) begin
        if(pos==2 || pos==13) started<=0;
        else if(((pos>=3 && pos<8) || (pos>=14 && pos<19)) && dig!=0)
          started<=1;
        if(pos==52) begin state<=ART;pos<=0;col<=0;row<=0;end
        else pos<=pos+1;
      end
      ART:if(tx_ready) begin
        if(pos==65) begin
          pos<=0;col<=0;
          if(row==15) state<=FOOTER;
          else row<=row+1;
        end else begin pos<=pos+1;col<=col+1;end
      end
      FOOTER:if(tx_ready) begin
        if(pos==20) begin state<=IDLE;pos<=0;end else pos<=pos+1;
      end
      default:state<=WAIT;
    endcase
    end
  end
  // LED bar of the displayed signal, D7 physically leftmost.
  always @(posedge clk) begin
    if(reset) led<=0;
    else if(valid) begin
      case({~displayed[17],displayed[16:15]})
        0:led<=8'h00;1:led<=8'h80;2:led<=8'hc0;3:led<=8'he0;
        4:led<=8'hf0;5:led<=8'hf8;6:led<=8'hfc;7:led<=8'hff;
      endcase
    end
  end
endmodule
