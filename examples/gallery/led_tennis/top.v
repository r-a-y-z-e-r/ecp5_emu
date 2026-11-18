// Eight lamps are enough for timing, risk, scoring and a deterministic rival.
// D7 is physically leftmost. A serve launches from D6 or D1; the receiver
// has one full step interval at its end lamp in which to return the ball.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output reg [7:0] led);
 wire clk=clk_25mhz;
 reg [5:0] reset_count=0;
 wire reset=!(&reset_count);
 always @(posedge clk) if (reset) reset_count<=reset_count+1;
 reg send=0,take=0;
 reg [7:0] data=0;
 wire busy,available;
 wire [7:0] key;
 wire [6:0] pressed;
 gallery_io io(clk,reset,ftdi_txd,ftdi_rxd,btn,pressed,send,data,busy,key,available,take);
 localparam LOG=0, SERVE=1, PLAY=2, FLASH=3, WIN=4, POINT=5, RESTART_WAIT=6;
 localparam M_RESTART=0,M_ATTRACT=1,M_HUMAN=2,M_TWO=3,M_SERVE=4,
   M_HIT=5,M_FAULT=6,M_POINT=7,M_GAME=8;
 reg [2:0] state=LOG, after_log=SERVE;
 reg [3:0] message=M_ATTRACT;
 reg [5:0] pos=0;
 reg human=0,two=0,server=0,side=0; // side/server: 0 left, 1 right
 reg [2:0] ball=6;
 reg toward_right=1;
 reg [3:0] score_l=0,score_r=0;
 reg [7:0] rally=0;
 reg [22:0] step_time=5000000,timer=0;
 reg [23:0] ceremony=0;
 reg [31:0] lfsr=32'h716ac123;
 reg [6:0] pending=0;
 reg hit_l=0,hit_r=0;
 wire [22:0] shortened=step_time-(step_time>>4)-(step_time>>6);
 // The computer returns the ball with a chance out of 256 that falls as the
 // ball speeds up: 242/256 (95%) at the human game's 200 ms step,
 // 194/256 (76%) at attract mode's 80 ms, 178/256 (70%) at the 40 ms floor.
 // Rallies therefore end by themselves, and attract mode is an exhibition
 // with fallible players rather than an endless perfect rally.
 wire [7:0] probability=8'd170+{1'b0,step_time[22:19],3'b0};
 wire cpu_success=lfsr[7:0]<probability;
 wire [31:0] next_lfsr={lfsr[30:0],lfsr[31]^lfsr[21]^lfsr[1]^lfsr[0]};
 wire left_press=hit_l || pending[1] || pending[5];
 wire right_press=hit_r || pending[2] || pending[6];
 wire [3:0] hundreds=rally/100;
 wire [3:0] tens=(rally%100)/10;
 wire [3:0] units=rally%10;
 wire [1:0] digits=rally>=100 ? 3 : rally>=10 ? 2 : 1;
 wire [7:0] side_char=side ? "R":"L";
 reg [7:0] log_char;
 always @* begin
   log_char=10;
   case(message)
     M_RESTART: case(pos)
       0:log_char="R";1:log_char="E";2:log_char="S";3:log_char="T";
       4:log_char="A";5:log_char="R";6:log_char="T";
     endcase
     M_ATTRACT: case(pos)
       0:log_char="A";1:log_char="T";2:log_char="T";3:log_char="R";
       4:log_char="A";5:log_char="C";6:log_char="T";
     endcase
     M_HUMAN: case(pos)
       0:log_char="H";1:log_char="U";2:log_char="M";3:log_char="A";4:log_char="N";
       5:log_char=" ";6:log_char="L";7:log_char=" ";8:log_char="v";9:log_char="s";
       10:log_char=" ";11:log_char="C";12:log_char="P";13:log_char="U";
       14:log_char=" ";15:log_char="R";
     endcase
     M_TWO: case(pos)
       0:log_char="H";1:log_char="U";2:log_char="M";3:log_char="A";4:log_char="N";
       5:log_char=" ";6:log_char="L";7:log_char=" ";8:log_char="v";9:log_char="s";
       10:log_char=" ";11:log_char="H";12:log_char="U";13:log_char="M";14:log_char="A";
       15:log_char="N";16:log_char=" ";17:log_char="R";
     endcase
     M_SERVE: case(pos)
       0:log_char="S";1:log_char="E";2:log_char="R";3:log_char="V";
       4:log_char="E";5:log_char=" ";6:log_char=side_char;
     endcase
     M_HIT: case(pos)
       0:log_char="H";1:log_char="I";2:log_char="T";3:log_char=" ";
       4:log_char=side_char;5:log_char=" ";
       6:log_char="0"+(digits==3 ? hundreds : digits==2 ? tens : units);
       7:if (digits>=2) log_char="0"+(digits==3 ? tens : units);
       8:if (digits==3) log_char="0"+units;
     endcase
     M_FAULT: case(pos)
       0:log_char="F";1:log_char="A";2:log_char="U";3:log_char="L";
       4:log_char="T";5:log_char=" ";6:log_char=side_char;
     endcase
     M_POINT: case(pos)
       0:log_char="P";1:log_char="O";2:log_char="I";3:log_char="N";4:log_char="T";
       5:log_char=" ";6:log_char=side_char;7:log_char=" ";
       8:log_char="0"+score_l;9:log_char=":";10:log_char="0"+score_r;
     endcase
     M_GAME: case(pos)
       0:log_char="G";1:log_char="A";2:log_char="M";3:log_char="E";
       4:log_char=" ";5:log_char=side_char;6:log_char=" ";
       7:log_char="0"+(side ? score_r:score_l);8:log_char=":";
       9:log_char="0"+(side ? score_l:score_r);
     endcase
   endcase
 end
 // The short event logger applies backpressure to the game. Timers stop for
 // its few milliseconds; every event is preserved rather than overwritten.
 always @* begin
   led=8'b10000000 >> (7-ball);
   if (state==FLASH) led=ceremony[19] ? 8'hff:0;
   if (state==WIN) led=ceremony[21] ? (side ? 8'h0f:8'hf0):0;
 end
 task log_event;
   input [3:0] kind;
   input [2:0] continuation;
   begin message<=kind; after_log<=continuation; state<=LOG; pos<=0; end
 endtask
 task return_ball;
   input player;
   begin
     side<=player; toward_right<=!player; timer<=0;
     if (rally!=255) rally<=rally+1;
     step_time<=shortened<1000000 ? 1000000:shortened;
     log_event(M_HIT,PLAY);
   end
 endtask
 task new_game;
   input mode_two;
   begin
     human<=1; two<=mode_two; score_l<=0; score_r<=0; server<=0;
     ball<=6; toward_right<=1; rally<=0; timer<=0; step_time<=5000000;
     pending<=0; hit_l<=0; hit_r<=0;
     log_event(mode_two ? M_TWO:M_HUMAN,SERVE);
   end
 endtask
 always @(posedge clk) begin
   send<=0; take<=0;
   pending<=pending | (pressed & 7'b1111110);
   if (reset) begin state<=LOG; message<=M_ATTRACT; pos<=0; after_log<=SERVE; end
   else if (state==RESTART_WAIT) begin
     // The serial block is left running so the current byte gets its stop bit.
     if (!busy && !send) begin
       human<=0; two<=0; server<=0; side<=0; ball<=6; toward_right<=1;
       score_l<=0; score_r<=0; rally<=0; step_time<=5000000; timer<=0;
       ceremony<=0; lfsr<=32'h716ac123; pending<=0; hit_l<=0; hit_r<=0;
       log_event(M_RESTART,WIN); // WIN is intercepted below for this message.
     end
   end else if (state!=LOG && available && !take) begin
     take<=1;
     case(key)
       "r": state<=RESTART_WAIT;
       "n": new_game(0);
       "2": new_game(1);
       "a": if (!human) new_game(0); else hit_l<=1;
       "l": if (!human) new_game(0); else hit_r<=1;
     endcase
   end else if (state!=LOG && !human && pending!=0) new_game(0);
   else case(state)
     LOG: if (!busy && !send) begin
       data<=log_char; send<=1;
       if (log_char==10) begin
         pos<=0;
         if (message==M_RESTART) log_event(M_ATTRACT,SERVE);
         else state<=after_log;
       end else pos<=pos+1;
     end
     SERVE: begin
       side<=server; ball<=server ? 1:6; toward_right<=!server;
       // A human game starts at 200 ms per lamp; attract mode, an exhibition,
       // at 80 ms, so it is lively on the board and watchable in the emulator.
       timer<=0; step_time<=human ? 5000000:2000000;
       rally<=0; hit_l<=0; hit_r<=0; pending<=0;
       log_event(M_SERVE,PLAY);
     end
     PLAY: begin
       // The left player owns D7..D4, the right D3..D0. A press on the
       // opposite half is ignored. An early press on one's own half faults.
       if (human && left_press) begin
         hit_l<=0; pending[1]<=0; pending[5]<=0;
         if (ball==7) return_ball(0);
         else if (ball>=4) begin side<=0; log_event(M_FAULT,POINT); end
       end else if (human && two && right_press) begin
         hit_r<=0; pending[2]<=0; pending[6]<=0;
         if (ball==0) return_ball(1);
         else if (ball<=3) begin side<=1; log_event(M_FAULT,POINT); end
       end else if (timer==step_time-1) begin
         timer<=0;
         if ((toward_right && ball==0) || (!toward_right && ball==7)) begin
           lfsr<=next_lfsr;
           if ((!human || (toward_right && !two)) && cpu_success)
             return_ball(toward_right);
           else begin side<=toward_right; state<=POINT; end
         end else ball<=toward_right ? ball-1:ball+1;
       end else timer<=timer+1;
     end
     POINT: begin
       // side records the player who missed/faulted; invert it for the scorer.
       side<=!side;
       if (side) score_l<=score_l+1; else score_r<=score_r+1;
       ceremony<=0; log_event(M_POINT,FLASH);
     end
     FLASH: begin
       ceremony<=ceremony+1;
       if (ceremony==1249999) begin
         ceremony<=0;
         if (score_l==5 || score_r==5) log_event(M_GAME,WIN);
         else begin server<=!server; state<=SERVE; end
       end
     end
     WIN: begin
       ceremony<=ceremony+1;
       if (ceremony==9999999) begin
         human<=0; two<=0; score_l<=0; score_r<=0; server<=0;
         pending<=0; hit_l<=0; hit_r<=0;
         log_event(M_ATTRACT,SERVE);
       end
     end
     default: state<=RESTART_WAIT;
   endcase
 end
endmodule
