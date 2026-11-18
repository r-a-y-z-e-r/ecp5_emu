// Orchard / three hand-drawn masks. One fixed-point step between OLED fills.
module bricks_game(input clk, input reset, input step, input player_start,
 input launch, input left, input right, input [3:0] options,
 output reg done=0, output reg human=0, output reg [71:0] bricks=0,
 output reg [6:0] remaining=0, output reg [11:0] score=0,
 output reg [1:0] lives=3, output reg [1:0] level=0,
 output reg [6:0] paddle=48, output [11:0] bx, by,
 output reg [3:0] events=0, output reg [3:0] hit_col=0,
 output reg [2:0] hit_row=0, output reg waiting=0,
 output wire wide, output wire theme);
 // Keep signed arithmetic internal: Yosys 0.70 rejects signed ANSI ports.
 reg signed [11:0] ball_x=768, ball_y=832;
 assign bx=ball_x; assign by=ball_y;
 // Positions are signed Q8.4, velocities signed Q4.4 pixels per picture.
 reg signed [7:0] vx=8, vy=-18;
 reg signed [11:0] nx=0, ny=0;
 reg signed [7:0] dx=0, dy=0;
 reg [15:0] lfsr=16'h6d29;
 reg signed [6:0] aim_error=0;
 reg [3:0] returns=0;
 reg [6:0] pause_count=0;
 reg [1:0] state=0;
 reg [6:0] scan=0;
 reg [3:0] col=0;
 reg [2:0] row=0;
 reg collided=0;
 assign wide=human && options[1];
 assign theme=human && options[2];
 wire signed [7:0] speed_y=(human && options[0]) ? 8'sd27 : 8'sd18;
 wire signed [11:0] target=$signed(ball_x) + ($signed(aim_error) <<< 4);
 wire signed [11:0] paddle_q=$signed({1'b0,paddle,4'b0});
 wire signed [11:0] half_q=wide ? 12'sd192 : 12'sd128;
 // Renderer covers [centre-half, centre+half): both widths reach 0..95.
 // All four limits are even, like 48 +/- 2k. Saturation also handles an
 // odd centre or a width change without crossing or missing an endpoint.
 wire [6:0] paddle_min=wide ? 7'd12 : 7'd8;
 wire [6:0] paddle_max=7'd96-paddle_min;
 wire [6:0] paddle_bounded=paddle<paddle_min ? paddle_min :
                           paddle>paddle_max ? paddle_max : paddle;
 wire [6:0] paddle_left=paddle_bounded<paddle_min+7'd2 ? paddle_min : paddle_bounded-7'd2;
 wire [6:0] paddle_right=paddle_bounded>paddle_max-7'd2 ? paddle_max : paddle_bounded+7'd2;
 wire [6:0] human_paddle_next=left && !right ? paddle_left :
                              right && !left ? paddle_right : paddle_bounded;
 wire signed [11:0] human_paddle_q=$signed({1'b0,human_paddle_next,4'b0});
 wire signed [11:0] brick_left=$signed({1'b0,col,7'b0});
 wire signed [11:0] brick_top=12'sd192 + $signed({3'b0,row,6'b0});
 function [71:0] layout;
   input [1:0] n;
   begin case(n)
     // Each row reads left-to-right in ascending bit order.
     0: layout=72'hfffdbdbff7fe6f6fff;
     1: layout=72'hc6339c6c63639c63c6;
     default: layout=72'h79e39cc6339ce79e79;
   endcase end
 endfunction
 function [6:0] count;
   input [71:0] mask;
   integer k;
   begin count=0; for(k=0;k<72;k=k+1) count=count+mask[k]; end
 endfunction
 task fresh;
   begin
     bricks<=layout(0); remaining<=count(layout(0)); score<=0;
     lives<=3; level<=0; paddle<=48; ball_x<=768; ball_y<=832;
     vx<=8; vy<=-18; lfsr<=16'h6d29; aim_error<=0; returns<=0;
     pause_count<=0; waiting<=0; events<=0; scan<=0; row<=0; col<=0;
     nx<=0; ny<=0; dx<=0; dy<=0; collided<=0; state<=0;
   end
 endtask
 always @(posedge clk) begin
   done<=0;
   if(reset) begin fresh; human<=0; end
   else if(player_start) begin fresh; human<=1; waiting<=1; ball_y<=896; done<=1; end
   else case(state)
     0: if(step) begin
       events<=0;
       // Width changes must re-clamp even a stationary game-over paddle.
       paddle<=paddle_bounded;
       // OVER is latched by lives=0. Only player_start or reset may leave
       // human game-over; neither the LFSR nor movement advances this picture.
       if(human && lives==0) done<=1;
       else begin
         lfsr<={lfsr[14:0],lfsr[15]^lfsr[13]^lfsr[12]^lfsr[10]};
         // Clamped, speed-limited pursuit. Only human play reads options/buttons.
         if(human) begin
           paddle<=human_paddle_next;
         end else begin
           if(target < paddle_q-16) paddle<=paddle_left;
           if(target > paddle_q+16) paddle<=paddle_right;
         end
         if(pause_count!=0) begin
           pause_count<=pause_count-1;
           if(pause_count==1) begin fresh; human<=0; end
           done<=1;
         end else if(waiting) begin
           // Use this update's paddle position, not its previous-frame centre.
           // The human ball's bottom row 57 rests above paddle row 58.
           ball_x<=human ? human_paddle_q : paddle_q;
           ball_y<=human ? 12'sd896 : 12'sd832;
           if(!human || launch) begin waiting<=0; vx<=8; vy<=-speed_y; end
           done<=1;
         end else if(remaining==0) begin
           if(level==2) begin level<=0; bricks<=layout(0); remaining<=count(layout(0)); end
           else begin level<=level+1; bricks<=layout(level+1); remaining<=count(layout(level+1)); end
           ball_x<=human ? human_paddle_q : paddle_q;
           ball_y<=human ? 12'sd896 : 12'sd832;
           waiting<=1; events[2]<=1; done<=1;
         end else begin
           nx<=ball_x+vx; ny<=ball_y+vy; dx<=vx; dy<=vy;
           state<=1;
         end
       end
     end
     1: begin
       // Reflect against centre bounds (one-pixel ball radius).
       if(nx<16) begin nx<=32-nx; dx<=-dx; end
       if(nx>1504) begin nx<=3008-nx; dx<=-dx; end
       if(ny<176) begin ny<=352-ny; dy<=speed_y; end
       scan<=0; row<=0; col<=0; collided<=0; state<=2;
     end
     2: begin
       // Visit one brick each clock, stopping after the first overlap.
       if(!collided && bricks[scan] && nx+16>=brick_left && nx-16<brick_left+112 &&
          ny+16>=brick_top && ny-16<brick_top+48) begin
         bricks[scan]<=0; remaining<=remaining-1; collided<=1;
         hit_col<=col; hit_row<=row; events[0]<=1;
         if(score[3:0]==9) begin
           score[3:0]<=0;
           if(score[7:4]==9) begin score[7:4]<=0; score[11:8]<=score[11:8]==9 ? 0 : score[11:8]+1; end
           else score[7:4]<=score[7:4]+1;
         end else score[3:0]<=score[3:0]+1;
         if(ball_y+16<=brick_top || ball_y-16>=brick_top+48) begin
           dy<=-dy; ny<=ball_y;
         end else begin dx<=-dx; nx<=ball_x; end
       end
       if(scan==71) state<=3;
       else begin
         scan<=scan+1;
         if(col==11) begin col<=0; row<=row+1; end else col<=col+1;
       end
     end
     3: begin
       ball_x<=nx; ball_y<=ny; vx<=dx; vy<=dy;
       // Same half-open x interval as render.v, including fractional positions.
       if(dy>0 && ball_y<912 && ny>=912 && nx>=paddle_q-half_q && nx<paddle_q+half_q) begin
         ball_y<=912; vy<=-speed_y; returns<=returns+1;
         // Five launch angles, chosen by the ball's offset from paddle centre.
         if(nx<paddle_q-64) vx<=-16;
         else if(nx<paddle_q-16) vx<=-8;
         else if(nx>paddle_q+64) vx<=16;
         else if(nx>paddle_q+16) vx<=8;
         else vx<=4;
         // Two accurate returns, then a deliberately fallible LFSR pursuit.
         if(returns==1) begin
           // Aim the large error inward: clamping an outward error at a
           // wall can now catch the ball there. Keep attract's MISS coverage.
           if(ball_x>=12'sd768) aim_error<=-7'sd18-$signed({5'b0,lfsr[1:0]});
           else aim_error<=7'sd18+$signed({5'b0,lfsr[1:0]});
         end
         else aim_error<=$signed({1'b0,lfsr[2:0]})-3;
       end
       if(ny>1008) begin
         events[1]<=1; waiting<=1; ball_x<=paddle_q;
         ball_y<=human ? 12'sd896 : 12'sd832; returns<=0; aim_error<=0;
         if(!(human && options[3])) begin
           lives<=lives-1;
           if(lives==1) begin events[3]<=1; pause_count<=human ? 0 : 100; waiting<=0; end
         end
       end
       done<=1; state<=0;
     end
   endcase
 end
endmodule
