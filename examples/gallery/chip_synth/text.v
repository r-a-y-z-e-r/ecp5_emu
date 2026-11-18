// A line is held through its final stop bit. Payload is copied at start.
module synth_text(input clk, input start, input [2:0] kind,
 input [15:0] frame, input [31:0] crc, input [6:0] note,
 output reg send=0, output reg [7:0] data=0, input tx_busy,
 output busy, output reg done=0);
 reg [2:0] state=0, saved_kind=0;
 reg [4:0] index=0, last=0;
 reg [15:0] saved_frame=0;
 reg [31:0] saved_crc=0;
 reg [6:0] saved_note=0;
 assign busy=state!=0;
 function [7:0] hex;
   input [3:0] n;
   begin hex=n<10?"0"+n:"A"+n-10; end
 endfunction
 reg [7:0] character;
 always @* begin
   character=0;
   case(saved_kind)
     0: case(index)
       0:character="A"; 1:character="U"; 2:character="D"; 3:character="I"; 4:character="O";
       5,10:character=" "; 6:character="3"; 7:character="1"; 8:character="2"; 9:character="5";
       19:character=13; 20:character=10;
       default:character=hex(saved_crc>>((18-index)*4));
     endcase
     1: case(index)
       0:character="F"; 5:character=" "; 6:character="C"; 15:character=13; 16:character=10;
       default:if(index<5) character=hex(saved_frame>>((4-index)*4));
         else character=hex(saved_crc>>((14-index)*4));
     endcase
     2: case(index)
       0:character="N"; 1:character="O"; 2:character="T"; 3:character="E"; 4:character=" ";
       5:character="0"+saved_note/100; 6:character="0"+(saved_note/10)%10; 7:character="0"+saved_note%10;
       8:character=" "; 17:character=13; 18:character=10;
       default:character=hex(saved_crc>>((16-index)*4));
     endcase
     3: case(index)
       0:character="R"; 1:character="E"; 2:character="S"; 3:character="T";
       4:character="A"; 5:character="R"; 6:character="T"; 7:character=13; 8:character=10;
     endcase
     default: case(index)
       0:character="O"; 1:character="V"; 2:character="E"; 3:character="R";
       4:character="F"; 5:character="L"; 6:character="O"; 7:character="W"; 8:character=13; 9:character=10;
     endcase
   endcase
 end
 always @(posedge clk) begin
   send<=0; done<=0;
   case(state)
     0: if(start) begin
       saved_kind<=kind; saved_frame<=frame; saved_crc<=crc; saved_note<=note; index<=0;
       case(kind) 0:last<=20; 1:last<=16; 2:last<=18; 3:last<=8; default:last<=9; endcase
       state<=1;
     end
     1: begin data<=character; state<=2; end
     2: if(!tx_busy && !send) begin send<=1; state<=3; end
     3: if(tx_busy) begin
       if(index==last) state<=4;
       else begin index<=index+1; state<=1; end
     end
     4: if(!tx_busy) begin done<=1; state<=0; end
   endcase
 end
endmodule
