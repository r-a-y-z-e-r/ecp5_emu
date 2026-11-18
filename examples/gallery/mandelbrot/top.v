// Three 18x18 products evaluate one z <- z*z+c iteration each clock.
// Q3.14 means one sign bit, three integer bits and fourteen fractional bits.
module top(input clk_25mhz, input ftdi_txd, output ftdi_rxd,
           input [6:0] btn, output [7:0] led);
  wire clk=clk_25mhz;
  reg [5:0] reset_count=0;
  wire reset=!(&reset_count);
  always @(posedge clk) if(reset) reset_count<=reset_count+1;
  wire [7:0] key;
  wire key_valid,tx_ready,overflow;
  reg [7:0] tx_data;
  reg tx_valid;
  wire key_pop;
  gallery_io io(clk,reset,ftdi_txd,ftdi_rxd,btn,tx_data,tx_valid,tx_ready,
                key,key_valid,key_pop,overflow);
  localparam IDLE=0, KEYLINE=1, LIMIT=2, START=3, HEADER=4,
             PIXEL=5, ITERATE=6, CHARACTER=7, CR=8, LF=9, FOOTER=10;
  reg [3:0] state=START;
  assign key_pop=(state==IDLE) && tx_ready && key_valid;
  reg signed [17:0] center_x=-12288,center_y=0;
  reg [17:0] width=49152;
  reg [7:0] max_iter=32;
  reg [31:0] view=0,cycles=0,sum=0;
  reg [7:0] last_key=0;
  reg [6:0] pos=0;
  reg [5:0] col=0;
  reg [4:0] row=0;
  reg signed [17:0] cx=0,cy=0,x=0,y=0;
  reg [7:0] iteration=0;
  reg tour=0,limited=0;
  reg [3:0] tour_index=0;
  // Products stay full width until after the subtraction/addition. Truncation
  // is arithmetic (towards minus infinity); escape is tested before updating.
  wire signed [35:0] xx=x*x, yy=y*y, xy=x*y;
  wire [36:0] norm={1'b0,xx}+{1'b0,yy};
  wire signed [36:0] nx=($signed({xx[35],xx})-$signed({yy[35],yy})) >>> 14;
  wire signed [35:0] ny=xy >>> 13;
  wire [17:0] dx=width>>6,dy=width>>5;
  assign led=iteration;
  function [7:0] hex(input [3:0] n);
    hex=n<10 ? "0"+n : "A"+n-10;
  endfunction
  // Fixed strings are small combinational ROMs, not a soft processor.
  function [7:0] htext(input [6:0] p);
    reg [8*70-1:0] s;
    begin s="VIEW 0x00000000 X 0x00000000 Y 0x00000000 W 0x00000000 ITER 0x00 Q14\r\n";
      htext=s[8*(70-p)-1 -:8]; end
  endfunction
  function [7:0] ftext(input [6:0] p);
    reg [8*35-1:0] s;
    begin s="CYCLES 0x00000000 SUM 0x00000000\r\n";
      ftext=s[8*(34-p)-1 -:8]; end
  endfunction
  function [7:0] palette(input [7:0] n,input [7:0] limit);
    reg [3:0] band;
    reg [11:0] scaled;
    begin
      // Nine escape bands and '@' for the bounded/max_iter pixels.
      scaled={4'b0,n}+({4'b0,n}<<3);
      case(limit)
        16:band=scaled>>4;32:band=scaled>>5;
        64:band=scaled>>6;default:band=scaled>>7;endcase
      if(n==limit) palette="@";
      else case(band)
        0:palette=" "; 1:palette="."; 2:palette=":"; 3:palette="-";
        4:palette="="; 5:palette="+"; 6:palette="*"; 7:palette="#";
        default:palette="%";
      endcase
    end
  endfunction
  function signed [17:0] pan(input signed [18:0] v);
    pan=v>32768 ? 18'sd32768 : v< -32768 ? -18'sd32768 : v[17:0];
  endfunction
  always @* begin
    tx_valid=0; tx_data=0;
    case(state)
      KEYLINE: begin
        tx_valid=1;
        case(pos) 0:tx_data="K";1:tx_data="E";2:tx_data="Y";3:tx_data=" ";
          4:tx_data=last_key;5:tx_data=13;default:tx_data=10;endcase
      end
      LIMIT: begin
        tx_valid=1;
        case(pos) 0:tx_data="Z";1:tx_data="O";2:tx_data="O";3:tx_data="M";
          4:tx_data=" ";5:tx_data="L";6:tx_data="I";7:tx_data="M";
          8:tx_data="I";9:tx_data="T";10:tx_data=13;default:tx_data=10;endcase
      end
      HEADER: begin
        tx_valid=1; tx_data=htext(pos);
        if(pos>=7 && pos<15) tx_data=hex(view >> ((14-pos)*4));
        if(pos>=20 && pos<28) tx_data=hex($signed(center_x) >>> ((27-pos)*4));
        if(pos>=33 && pos<41) tx_data=hex($signed(center_y) >>> ((40-pos)*4));
        if(pos>=46 && pos<54) tx_data=hex(width >> ((53-pos)*4));
        if(pos>=62 && pos<64) tx_data=hex(max_iter >> ((63-pos)*4));
      end
      CHARACTER: begin tx_valid=1;tx_data=palette(iteration,max_iter);end
      CR:begin tx_valid=1;tx_data=13;end
      LF:begin tx_valid=1;tx_data=10;end
      FOOTER:begin
        tx_valid=1;tx_data=ftext(pos);
        if(pos>=9 && pos<17) tx_data=hex(cycles >> ((16-pos)*4));
        if(pos>=24 && pos<32) tx_data=hex(sum >> ((31-pos)*4));
      end
    endcase
  end
  always @(posedge clk) begin
    if(reset) begin state<=START; pos<=0; end
    else case(state)
      IDLE: if(tx_ready) begin
        if(key_valid) begin
          last_key<=key; pos<=0; state<=KEYLINE; limited<=0; tour<=0;
          view<=view+1;
          case(key)
            "r":begin center_x<=-12288;center_y<=0;width<=49152;
                      max_iter<=32;view<=0;tour_index<=0;end
            "w":center_y<=pan($signed(center_y)-$signed({1'b0,width>>3}));
            "s":center_y<=pan($signed(center_y)+$signed({1'b0,width>>3}));
            "a":center_x<=pan($signed(center_x)-$signed({1'b0,width>>3}));
            "d":center_x<=pan($signed(center_x)+$signed({1'b0,width>>3}));
            "+":if(width>=128) width<=width>>1; else limited<=1;
            "-":if(width<=24576) width<=width<<1; else limited<=1;
            "i":max_iter<=max_iter==128 ? 16 : max_iter<<1;
            "t":begin tour<=1;tour_index<=0;center_x<=-12288;
                      center_y<=0;width<=49152;max_iter<=64;end
          endcase
        end else if(tour) begin
          if(tour_index==7) tour<=0;
          else begin
            tour_index<=tour_index+1; center_x<=-12182;center_y<=2153;
            width<=width>>1;view<=view+1;state<=START;
          end
        end
      end
      KEYLINE: if(tx_ready) begin
        if(pos==6) begin pos<=0;state<=limited ? LIMIT:START;end
        else pos<=pos+1;
      end
      LIMIT:if(tx_ready) begin
        if(pos==11) begin pos<=0;state<=START;end else pos<=pos+1;
      end
      START:begin
        cycles<=0;sum<=0;col<=0;row<=0;pos<=0;
        cx<=center_x-$signed(width>>1)+$signed(dx>>1);
        cy<=center_y-$signed((width>>2)+(width>>3))+$signed(dy>>1);
        state<=HEADER;
      end
      HEADER:if(tx_ready) begin
        if(pos==69) begin pos<=0;state<=PIXEL;end else pos<=pos+1;
      end
      PIXEL:begin x<=0;y<=0;iteration<=0;state<=ITERATE;end
      ITERATE:begin
        // Count only clocks actually executing an iteration (not UART waits
        // or the final escape/max test). Thus CYCLES is exactly SUM.
        if(norm>37'd1073741824 || iteration==max_iter) begin
          sum<=sum+iteration;state<=CHARACTER;
        end else begin
          x<=nx+cx;y<=ny+cy;iteration<=iteration+1;cycles<=cycles+1;
        end
      end
      CHARACTER:if(tx_ready) begin
        if(col==63) state<=CR;
        else begin col<=col+1;cx<=cx+$signed(dx);state<=PIXEL;end
      end
      CR:if(tx_ready) state<=LF;
      LF:if(tx_ready) begin
        if(row==23) begin state<=FOOTER;pos<=0;end
        else begin
          row<=row+1;col<=0;cy<=cy+$signed(dy);
          cx<=center_x-$signed(width>>1)+$signed(dx>>1);state<=PIXEL;
        end
      end
      FOOTER:if(tx_ready) begin
        if(pos==33) state<=IDLE; else pos<=pos+1;
      end
      default:state<=START;
    endcase
  end
endmodule
