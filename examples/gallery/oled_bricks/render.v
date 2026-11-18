// Pure combinational pixel=f(x,y,state); no image RAM, no downloaded glyphs.
module bricks_render(input [6:0] x, input [5:0] y,
 input [71:0] bricks, input [11:0] score, input [1:0] lives, level,
 input [6:0] paddle, input [11:0] bx, by,
 input wide, theme, human, waiting, output reg [15:0] pixel);
 function [14:0] digit;
   input [3:0] n;
   begin case(n)
     // Five rows of three bits, drawn here for this design.
     0: digit=15'b111_101_101_101_111;
     1: digit=15'b010_110_010_010_111;
     2: digit=15'b110_001_010_100_111;
     3: digit=15'b110_001_010_001_110;
     4: digit=15'b101_101_111_001_001;
     5: digit=15'b111_100_110_001_110;
     6: digit=15'b011_100_111_101_111;
     7: digit=15'b111_001_010_010_010;
     8: digit=15'b111_101_010_101_111;
     9: digit=15'b111_101_111_001_110;
     default: digit=0;
   endcase end
 endfunction
 // Original 3x5 FIRE1 lettering, shown only when a human can serve.
 function [14:0] serve_glyph;
   input [2:0] n;
   begin case(n)
     0: serve_glyph=15'b111_100_110_100_100; // F
     1: serve_glyph=15'b111_010_010_010_111; // I
     2: serve_glyph=15'b110_101_110_101_101; // R
     3: serve_glyph=15'b111_100_110_100_111; // E
     default: serve_glyph=digit(1);
   endcase end
 endfunction
 reg [3:0] d;
 reg [14:0] glyph;
 reg [14:0] hint_glyph;
 reg [4:0] hint_x;
 reg [3:0] hint_row;
 reg [6:0] index;
 reg [15:0] brick_color;
 wire [6:0] ball_x=bx[10:4];
 wire [5:0] ball_y=by[9:4];
 wire [6:0] half_width=wide ? 12 : 8;
 always @* begin
   // Plum night, sage, clay and pale gold; a second ink/copper theme.
   pixel=theme ? 16'h0823 : 16'h1005;
   d=0; glyph=0; index=0; hint_glyph=0; hint_x=0; hint_row=0;
   brick_color=16'h858e;
   case(y[4:2])
     3,6: brick_color=theme ? 16'hb3c8 : 16'hb28f;
     4,7: brick_color=theme ? 16'h6c74 : 16'h858e;
     default: brick_color=theme ? 16'hdd68 : 16'hd58d;
   endcase
   if(y==8 || y==63) pixel=theme ? 16'h21c9 : 16'h30ca;
   if(y>=12 && y<36 && x[2:0]!=7 && y[1:0]!=3) begin
     index=((y-12)>>2)*12+(x>>3);
     if(bricks[index]) pixel=(y[1:0]==0) ? 16'heeb8 : brick_color;
   end
   // Score 000..999, little diamond life counters, level and pilot chevron.
   if(y>=1 && y<=5) begin
     if(x>=3 && x<=5) d=score[11:8];
     else if(x>=7 && x<=9) d=score[7:4];
     else if(x>=11 && x<=13) d=score[3:0];
     else if(x>=87 && x<=89) d={2'b0,level}+1;
     glyph=digit(d);
     if(((x>=3 && x<=13 && x[1:0]!=2) || (x>=87 && x<=89)) &&
        glyph[(5-y)*3+(2-((x-3)&3))]) pixel=16'hf735;
   end
   if(y>=2 && y<=4 && x>=20 && x<20+{lives,2'b00} &&
      ((y==3 && x[1:0]!=3) || (y!=3 && x[1:0]==1))) pixel=16'hd58d;
   if(!human && x>=76 && x<=80 && (y==3 || (y==2 && x==79) || (y==4 && x==79))) pixel=16'h858e;
   if(human && x>=77 && x<=79 && y>=2 && y<=4) pixel=16'hb28f;
   if(y>=58 && y<=60 && x>=paddle-half_width && x<paddle+half_width &&
      !(y==60 && (x==paddle-half_width || x==paddle+half_width-1)))
     pixel=theme ? 16'hdd68 : 16'h9e34;
   if((x==ball_x && y>=ball_y-1 && y<=ball_y+1) ||
      (y==ball_y && x>=ball_x-1 && x<=ball_x+1)) pixel=16'hff9c;
   if(!human && waiting && y==54 && x>=paddle-2 && x<=paddle+2) pixel=16'h858e;
   if(human && waiting && lives!=0 && x>=38 && x<57 && y>=43 && y<=47) begin
     hint_x=x-7'd38;
     hint_glyph=serve_glyph(hint_x[4:2]);
     case(y)
       43: hint_row=12;
       44: hint_row=9;
       45: hint_row=6;
       46: hint_row=3;
       default: hint_row=0;
     endcase
     if(hint_x[1:0]<3 && hint_glyph[hint_row+4'd2-{2'b0,hint_x[1:0]}]) pixel=16'heeb8;
   end
 end
endmodule
