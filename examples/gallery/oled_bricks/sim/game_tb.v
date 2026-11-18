// Only the separate Verilator unit model defines ORCHARD_GAME_TEST. The
// project/synthesis sources contain no injection ports, defines or test hooks.
`ifdef ORCHARD_GAME_TEST
module game_tb(input clk, input reset, input step, input player_start,
 input launch, input left, input right, input [3:0] options,
 input inject, input [11:0] test_x, test_y, input [7:0] test_vx, test_vy,
 input [6:0] test_paddle, input test_waiting,
 output done, human, waiting, output [6:0] paddle,
 output [11:0] bx, by, output [1:0] lives, output [3:0] events,
 output [7:0] velocity_y);
 wire [71:0] bricks;
 wire [6:0] remaining;
 wire [11:0] score;
 wire [1:0] level;
 wire [3:0] col;
 wire [2:0] row;
 wire wide, theme;
 bricks_game dut(.clk(clk),.reset(reset),.step(step),.player_start(player_start),
   .launch(launch),.left(left),.right(right),.options(options),.done(done),
   .human(human),.bricks(bricks),.remaining(remaining),.score(score),.lives(lives),
   .level(level),.paddle(paddle),.bx(bx),.by(by),.events(events),
   .hit_col(col),.hit_row(row),.waiting(waiting),.wide(wide),.theme(theme));
 assign velocity_y=dut.vy;
 // Deposit between active clock edges, never replace the collision logic.
 always @(negedge clk) if(inject) begin
   dut.ball_x=test_x; dut.ball_y=test_y;
   dut.vx=test_vx; dut.vy=test_vy;
   dut.paddle=test_paddle; dut.waiting=test_waiting;
   dut.state=0; dut.pause_count=0; dut.lives=3;
 end
endmodule
`endif
