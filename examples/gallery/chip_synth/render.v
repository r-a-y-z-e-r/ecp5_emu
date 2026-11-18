// Original 3x5 glyphs and connected lime trace on a midnight RGB565 grid.
module scope_render(input clk, input [6:0] x, input [5:0] y,
 input [5:0] trace_here, trace_low, trace_high,
 input [4:0] step, input [6:0] note, input paused, output reg [15:0] pixel);
 function [14:0] glyph;
   input [7:0] c;
   begin case(c)
     "0":glyph=15'b111_101_101_101_111; "1":glyph=15'b010_110_010_010_111;
     "2":glyph=15'b110_001_111_100_111; "3":glyph=15'b110_001_010_001_110;
     "4":glyph=15'b101_101_111_001_001; "5":glyph=15'b111_100_110_001_110;
     "6":glyph=15'b011_100_111_101_111; "7":glyph=15'b111_001_010_010_010;
     "8":glyph=15'b111_101_111_101_111; "9":glyph=15'b111_101_111_001_110;
     "N":glyph=15'b101_111_111_111_101; "T":glyph=15'b111_010_010_010_010;
     "S":glyph=15'b011_100_010_001_110; "P":glyph=15'b110_101_110_100_100;
     default:glyph=0;
   endcase end
 endfunction
 // Decimal conversion and glyph lookup settle while the window commands
 // are sent; neither is on the pixel -> SPI/CRC path.
 reg [7:0] hundreds=0, tens=0, ones=0, step_tens=0, step_ones=0;
 reg [14:0] note_h=0, note_t=0, note_o=0, step_t=0, step_o=0, pause_glyph=0;
 reg [6:0] bar_end=13;
 always @(posedge clk) begin
   hundreds<="0"+note/100; tens<="0"+(note/10)%10; ones<="0"+note%10;
   step_tens<="0"+step/10; step_ones<="0"+step%10;
   note_h<=glyph(hundreds); note_t<=glyph(tens); note_o<=glyph(ones);
   step_t<=glyph(step_tens); step_o<=glyph(step_ones);
   pause_glyph<=paused ? glyph("P") : 15'b0;
   bar_end<=7'd13+{4'b0,step[2:0]}*7'd10;
 end
 wire [6:0] text_x=x-7'd3;
 wire [5:0] text_y=y-6'd3;
 reg [2:0] font_row;
 reg [14:0] bits;
 always @* begin
   pixel=16'h0863;
   if(y>=13) begin
     if(y==38 || x==0 || x==95 || y==13 || y==63) pixel=16'h2269;
     else if((x%16==0) && (y%4==0)) pixel=16'h1946;
     if(y>=trace_low && y<=trace_high) pixel=16'hbfe9;
     if(y==trace_here) pixel=16'heff4;
   end
   // Nnnn  T120  Sxx and a small pause mark, same original 3x5 font.
   bits=0; font_row=0;
   case(text_x[6:2])
     0:bits=glyph("N"); 1:bits=note_h; 2:bits=note_t; 3:bits=note_o;
     6:bits=glyph("T"); 7:bits=glyph("1"); 8:bits=glyph("2"); 9:bits=glyph("0");
     12:bits=glyph("S"); 13:bits=step_t; 14:bits=step_o;
     18:bits=pause_glyph;
     default:bits=0;
   endcase
   case(text_y[2:0])
     0:font_row=bits[14:12]; 1:font_row=bits[11:9]; 2:font_row=bits[8:6];
     3:font_row=bits[5:3]; 4:font_row=bits[2:0]; default:font_row=0;
   endcase
   if(x>=3 && x<79 && y>=3 && y<8 && text_x[1:0]<3 &&
       font_row[2-text_x[1:0]]) pixel=16'hfcd3;
   if(y==10 && x>=3 && x<bar_end) pixel=16'h4bd7;
 end
endmodule

// Compute floor(22*abs(sample)/peak) once per column. The complete 96-row
// cache is ready 202 clocks after capture, inside the unchanged 210 clocks
// occupied by the six window commands. One sample enters the pipeline per
// clock; each quotient stage contains one 21-bit subtract and a borrow mux.
module scope_trace_cache(input clk, input reset, input capture,
 input [1535:0] history, input [6:0] next_column,
 output [5:0] row_zero, row_next, output reg ready=0);
 reg [1535:0] snapshot=0;
 (* ram_style="distributed" *) reg [5:0] rows[0:95];
 integer r;
 initial for(r=0;r<96;r=r+1) rows[r]=38;
 assign row_zero=rows[0];
 assign row_next=rows[next_column];
 reg scanning=0, read_valid=0, read_last=0, mag_valid=0, mag_last=0;
 reg [6:0] scan_index=0;
 reg signed [15:0] scan_word=0;
 reg [15:0] scan_magnitude=0, peak=0;
 wire [15:0] next_peak=scan_magnitude>peak ? scan_magnitude : peak;
 reg filling=0, word_valid=0, magnitude_valid=0, product_valid=0;
 reg [6:0] source_index=95, write_index=0, word_index=0, magnitude_index=0, product_index=0;
 reg signed [15:0] trace_word=0;
 reg [15:0] trace_magnitude=0;
 reg magnitude_negative=0, product_negative=0;
 reg [20:0] numerator=0;
 wire [20:0] remainder[0:5];
 wire [4:0] quotient[0:5];
 wire [6:0] column[0:5];
 wire negative[0:5], valid[0:5];
 assign remainder[0]=numerator; assign quotient[0]=0;
 assign column[0]=product_index; assign negative[0]=product_negative; assign valid[0]=product_valid;
 genvar k;
 generate for(k=0;k<5;k=k+1) begin: divide
   reg [20:0] rem_q=0;
   reg [4:0] quotient_q=0;
   reg [6:0] column_q=0;
   reg negative_q=0, valid_q=0;
   wire [20:0] divisor={5'b0,peak} << (4-k);
   wire [21:0] difference={1'b0,remainder[k]}-{1'b0,divisor};
   assign remainder[k+1]=rem_q; assign quotient[k+1]=quotient_q;
   assign column[k+1]=column_q; assign negative[k+1]=negative_q; assign valid[k+1]=valid_q;
   always @(posedge clk) begin
     valid_q<=valid[k];
     if(valid[k]) begin
       rem_q<=difference[21] ? remainder[k] : difference[20:0];
       quotient_q<=quotient[k] | (difference[21] ? 5'd0 : (5'b10000 >> k));
       column_q<=column[k]; negative_q<=negative[k];
     end
     if(reset || capture) valid_q<=0;
   end
 end endgenerate
 always @(posedge clk) begin
   // Snapshot read -> absolute value -> peak compare: three short paths.
   read_valid<=scanning;
   if(scanning) begin
     scan_word<=snapshot[{scan_index,4'b0} +: 16];
     read_last<=scan_index==95;
     if(scan_index==95) scanning<=0; else scan_index<=scan_index+1;
   end
   mag_valid<=read_valid; mag_last<=read_last;
   if(read_valid) scan_magnitude<=scan_word[15] ? -scan_word : scan_word;
   if(mag_valid) begin
     peak<=next_peak;
     if(mag_last) begin filling<=1; source_index<=95; write_index<=0; end
   end
   // Snapshot mux, absolute value and constant multiply are separate stages.
   word_valid<=filling;
   if(filling) begin
     trace_word<=snapshot[{source_index,4'b0} +: 16]; word_index<=write_index;
     if(write_index==95) filling<=0;
     else begin write_index<=write_index+1; source_index<=source_index-1; end
   end
   magnitude_valid<=word_valid;
   if(word_valid) begin
     trace_magnitude<=trace_word[15] ? -trace_word : trace_word;
     magnitude_negative<=trace_word[15]; magnitude_index<=word_index;
   end
   product_valid<=magnitude_valid;
   if(magnitude_valid) begin
     numerator<={5'b0,trace_magnitude}*21'd22;
     product_index<=magnitude_index; product_negative<=magnitude_negative;
   end
   if(valid[5]) begin
     rows[column[5]]<=peak==0 ? 6'd38 : negative[5] ?
       6'd38+{1'b0,quotient[5]} : 6'd38-{1'b0,quotient[5]};
     if(column[5]==95) ready<=1;
   end
   if(capture) begin
     snapshot<=history; ready<=0; scanning<=1; scan_index<=0; peak<=0;
     read_valid<=0; mag_valid<=0; filling<=0; word_valid<=0;
     magnitude_valid<=0; product_valid<=0;
   end
   if(reset) begin
     ready<=0; scanning<=0; read_valid<=0; mag_valid<=0;
     filling<=0; word_valid<=0; magnitude_valid<=0; product_valid<=0;
   end
 end
endmodule
