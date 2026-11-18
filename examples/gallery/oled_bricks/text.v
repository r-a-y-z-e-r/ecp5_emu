// A line printer with backpressure. Abort leaves the UART's current byte intact.
module bricks_text(input clk, input abort_line, input start, input [3:0] kind,
 input [15:0] frame, input [31:0] crc, input [11:0] score,
 input [1:0] lives, level, input [6:0] remaining,
 input [3:0] col, input [2:0] row, input [3:0] switches,
 output reg send=0, output reg [7:0] data=0, input tx_busy,
 output wire busy, output reg done=0);
 reg [1:0] state=0;
 reg [5:0] index=0;
 reg [7:0] character;
 reg [5:0] last;
 assign busy=state!=0;
 function [7:0] hex;
   input [3:0] n;
   begin hex=n<10 ? "0"+n : "A"+n-10; end
 endfunction
 always @* begin
   character=10; last=0;
   case(kind)
     0: begin
       last=28;
       case(index)
         0: character="F";
         1: character=hex(frame[15:12]);
         2: character=hex(frame[11:8]);
         3: character=hex(frame[7:4]);
         4: character=hex(frame[3:0]);
         5: character=" ";
         6: character="C";
         7: character=hex(crc[31:28]);
         8: character=hex(crc[27:24]);
         9: character=hex(crc[23:20]);
         10: character=hex(crc[19:16]);
         11: character=hex(crc[15:12]);
         12: character=hex(crc[11:8]);
         13: character=hex(crc[7:4]);
         14: character=hex(crc[3:0]);
         15: character=" ";
         16: character="S";
         17: character="0"+score[11:8];
         18: character="0"+score[7:4];
         19: character="0"+score[3:0];
         20: character=" ";
         21: character="L";
         22: character="0"+lives;
         23: character=" ";
         24: character="B";
         25: character="0"+(remaining/10);
         26: character="0"+(remaining%10);
         27: character=8'd13;
         28: character=8'd10;
         default: character=10;
       endcase
     end
     1: begin
       last=9;
       case(index)
         0: character=8'd72;
         1: character=8'd73;
         2: character=8'd84;
         3: character=8'd32;
         4: character="0"+(col/10);
         5: character="0"+(col%10);
         6: character=" ";
         7: character="0"+row;
         8: character=8'd13;
         9: character=8'd10;
         default: character=10;
       endcase
     end
     2: begin
       last=5;
       case(index)
         0: character=8'd77;
         1: character=8'd73;
         2: character=8'd83;
         3: character=8'd83;
         4: character=8'd13;
         5: character=8'd10;
         default: character=10;
       endcase
     end
     3: begin
       last=8;
       case(index)
         0: character=8'd76;
         1: character=8'd69;
         2: character=8'd86;
         3: character=8'd69;
         4: character=8'd76;
         5: character=8'd32;
         6: character="1"+level;
         7: character=8'd13;
         8: character=8'd10;
         default: character=10;
       endcase
     end
     4: begin
       last=9;
       case(index)
         0: character=8'd79;
         1: character=8'd86;
         2: character=8'd69;
         3: character=8'd82;
         4: character=8'd32;
         5: character="0"+score[11:8];
         6: character="0"+score[7:4];
         7: character="0"+score[3:0];
         8: character=8'd13;
         9: character=8'd10;
         default: character=10;
       endcase
     end
     5: begin
       last=7;
       case(index)
         0: character=8'd80;
         1: character=8'd76;
         2: character=8'd65;
         3: character=8'd89;
         4: character=8'd69;
         5: character=8'd82;
         6: character=8'd13;
         7: character=8'd10;
         default: character=10;
       endcase
     end
     6: begin
       last=8;
       case(index)
         0: character=8'd83;
         1: character=8'd87;
         2: character=8'd32;
         3: character=switches[0] ? "1" : "0";
         4: character=switches[1] ? "1" : "0";
         5: character=switches[2] ? "1" : "0";
         6: character=switches[3] ? "1" : "0";
         7: character=8'd13;
         8: character=8'd10;
         default: character=10;
       endcase
     end
     7: begin
       last=8;
       case(index)
         0: character=8'd82;
         1: character=8'd69;
         2: character=8'd83;
         3: character=8'd84;
         4: character=8'd65;
         5: character=8'd82;
         6: character=8'd84;
         7: character=8'd13;
         8: character=8'd10;
         default: character=10;
       endcase
     end
     default: begin character=10; last=0; end
   endcase
 end
 always @(posedge clk) begin
   send<=0; done<=0;
   if(abort_line) begin state<=0; index<=0; end
   else case(state)
     0: if(start) begin index<=0; state<=1; end
     1: if(!tx_busy) begin data<=character; send<=1; state<=2; end
     2: if(tx_busy) state<=3;
     3: if(!tx_busy) begin
       if(index==last) begin done<=1; state<=0; end
       else begin index<=index+1; state<=1; end
     end
   endcase
 end
endmodule
