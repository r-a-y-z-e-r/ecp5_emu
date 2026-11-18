// Four integer DDS voices. See README for the sample-order specification.
module chip_synth(input clk, input reset, input enable, input pause_toggle,
 input [6:0] live_note, output tick, output reg sample_toggle=0, output reg [31:0] sample_number=0,
 output reg [3:0] audio_l=8, output reg [3:0] audio_r=8, output [3:0] audio_v,
 output reg [1535:0] history=0, output reg [4:0] step=0,
 output reg [6:0] display_note=0, output reg paused=0,
 output reg report=0, input report_take, output reg [31:0] report_crc=0);
 assign audio_v=8;
 reg [9:0] divider=0;
 assign tick=enable && divider==999;
 reg [12:0] beat_sample=0, block_sample=0, live_age=0;
 reg [3:0] env_div=0;
 reg [23:0] phase[0:3], increment[0:3];
 reg [7:0] env[0:3];
 reg [1:0] stage[0:3]; // 0 release/idle, 1 attack, 2 decay, 3 sustain
 reg [31:0] crc=32'hffffffff;
 integer i;
 initial for(i=0;i<4;i=i+1) begin phase[i]=0; increment[i]=0; env[i]=0; stage[i]=0; end
 function [31:0] crc_byte;
   input [31:0] c; input [7:0] b;
   reg [31:0] t; integer k;
   begin t=c^{24'b0,b}; for(k=0;k<8;k=k+1) t=(t>>1)^(t[0]?32'hedb88320:0); crc_byte=t; end
 endfunction
 // Packed ROM words are bass, lead, harmony MIDI note numbers; zero is rest.
 function [23:0] score;
   input [4:0] n;
   begin case(n)
     0: score=24'h30483c;
     1: score=24'h004c00;
     2: score=24'h304a40;
     3: score=24'h004300;
     4: score=24'h374543;
     5: score=24'h000000;
     6: score=24'h304a40;
     7: score=24'h004c00;
     8: score=24'h2d4839;
     9: score=24'h004500;
     10: score=24'h2d433c;
     11: score=24'h004000;
     12: score=24'h344340;
     13: score=24'h000000;
     14: score=24'h2d453c;
     15: score=24'h004a00;
     16: score=24'h354d3c;
     17: score=24'h004c00;
     18: score=24'h354841;
     19: score=24'h004500;
     20: score=24'h3c4845;
     21: score=24'h004f00;
     22: score=24'h354c41;
     23: score=24'h000000;
     24: score=24'h2b4a3b;
     25: score=24'h004300;
     26: score=24'h32473e;
     27: score=24'h004a00;
     28: score=24'h374f43;
     29: score=24'h004c00;
     30: score=24'h2b4a3e;
     31: score=24'h000000;
     default: score=0;
   endcase end
 endfunction
 function [23:0] pitch;
   input [6:0] midi;
   begin case(midi)
     43: pitch=24'd65766;
     45: pitch=24'd73820;
     48: pitch=24'd87787;
     50: pitch=24'd98538;
     52: pitch=24'd110605;
     53: pitch=24'd117182;
     55: pitch=24'd131532;
     57: pitch=24'd147640;
     59: pitch=24'd165720;
     60: pitch=24'd175574;
     62: pitch=24'd197075;
     64: pitch=24'd221209;
     65: pitch=24'd234363;
     67: pitch=24'd263064;
     69: pitch=24'd295279;
     71: pitch=24'd331439;
     72: pitch=24'd351148;
     74: pitch=24'd394150;
     76: pitch=24'd442419;
     77: pitch=24'd468726;
     79: pitch=24'd526127;
     81: pitch=24'd590558;
     default: pitch=0;
   endcase end
 endfunction
 function signed [5:0] wave;
   input [1:0] voice; input [4:0] p;
   begin case(voice)
     0: wave=p<16 ? -15+$signed({1'b0,p,1'b0}) : 47-$signed({1'b0,p,1'b0});
     1: wave=p<8 ? 15 : -15;
     2: wave=$signed({1'b0,p})-16;
     default: case(p)
       0,16: wave=0; 1,15: wave=3; 2,14: wave=6; 3,13: wave=8;
       4,12: wave=11; 5,11: wave=12; 6,10: wave=14; 7,8,9: wave=15;
       17,31: wave=-3; 18,30: wave=-6; 19,29: wave=-8;
       20,28: wave=-11; 21,27: wave=-12; 22,26: wave=-14;
       default: wave=-15;
     endcase
   endcase end
 endfunction
 function [7:0] sustain;
   input integer v;
   begin case(v) 0: sustain=144; 1: sustain=128; 2: sustain=96; default: sustain=176; endcase end
 endfunction
 // Prepare the next pair between sample strobes. Voice state changes only
 // once per 1000 clocks, so this pipeline adds no sample or pad-edge latency.
 function [3:0] clamp_code;
   input signed [6:0] c;
   begin if(c<4) clamp_code=4; else if(c>12) clamp_code=12; else clamp_code=c[3:0]; end
 endfunction
 wire signed [17:0] product[0:3];
 genvar v;
 generate for(v=0;v<4;v=v+1) begin: mix_voice
   reg signed [5:0] waveform=0;
   reg [7:0] gain=0;
   reg signed [14:0] multiplied=0;
   assign product[v]={{3{multiplied[14]}},multiplied};
   always @(posedge clk) begin
     waveform<=wave(v,phase[v][23:19]); gain<=env[v];
     multiplied<=waveform*$signed({1'b0,gain});
     if(reset) begin waveform<=0; gain<=0; multiplied<=0; end
   end
 end endgenerate
 reg signed [17:0] left_a=0, left_b=0, right_a=0, right_b=0;
 reg signed [17:0] mix_l=0, mix_r=0, rounded_l=0, rounded_r=0;
 reg [3:0] next_l=8, next_r=8;
 reg [31:0] crc_after_l=0, next_crc=0;
 wire signed [6:0] code_l=7'sd8+$signed(rounded_l[17:11]);
 wire signed [6:0] code_r=7'sd8+$signed(rounded_r[17:11]);
 always @(posedge clk) begin
   // Two-term partial sums, final sum, rounding and limiting are registered.
   left_a<=(product[0]<<<1)+(product[1]<<<1);
   left_b<=product[2]+(product[3]<<<1);
   right_a<=(product[0]<<<1)+product[1];
   right_b<=(product[2]<<<1)+(product[3]<<<1);
   mix_l<=left_a+left_b; mix_r<=right_a+right_b;
   rounded_l<=mix_l+18'sd1024; rounded_r<=mix_r+18'sd1024;
   next_l<=clamp_code(code_l); next_r<=clamp_code(code_r);
   // One CRC byte per register stage; no multiplier/sum feeds CRC logic.
   crc_after_l<=crc_byte(crc,{4'b0,next_l});
   next_crc<=crc_byte(crc_after_l,{4'b0,next_r});
   if(reset) begin
     left_a<=0; left_b<=0; right_a<=0; right_b<=0;
     mix_l<=0; mix_r<=0; rounded_l<=0; rounded_r<=0;
     next_l<=8; next_r<=8; crc_after_l<=0; next_crc<=0;
   end
 end
 wire [23:0] notes=score(step);
 always @(posedge clk) begin
   if(report_take) report<=0;
   if(pause_toggle) paused<=!paused;
   if(reset) begin
     divider<=0; beat_sample<=0; block_sample<=0; env_div<=0; live_age<=0;
     sample_number<=0; sample_toggle<=0; audio_l<=8; audio_r<=8; history<=0; step<=0; display_note<=0;
     paused<=0; report<=0; report_crc<=0; crc<=32'hffffffff;
     for(i=0;i<4;i=i+1) begin phase[i]<=0; increment[i]<=0; env[i]<=0; stage[i]<=0; end
   end else if(enable) begin
     if(divider!=999) divider<=divider+1;
     else begin
       divider<=0; env_div<=env_div+1; sample_toggle<=!sample_toggle; sample_number<=sample_number+1;
       audio_l<=next_l; audio_r<=next_r; history<={history[1519:0],mix_l[15:0]};
       if(block_sample==3124) begin
         block_sample<=0; crc<=32'hffffffff; report_crc<=~next_crc; report<=1;
       end else begin block_sample<=block_sample+1; crc<=next_crc; end
       for(i=0;i<4;i=i+1) begin
         phase[i]<=phase[i]+increment[i];
         if(env_div==0) case(stage[i])
           0: env[i]<=env[i]>2 ? env[i]-2 : 0;
           1: if(env[i]>=251) begin env[i]<=255; stage[i]<=2; end else env[i]<=env[i]+4;
           2: if(env[i]<=sustain(i)+1) begin env[i]<=sustain(i); stage[i]<=3; end else env[i]<=env[i]-1;
           3: env[i]<=sustain(i);
         endcase
       end
       if(!paused) begin
         if(beat_sample==0) begin
           for(i=0;i<3;i=i+1) begin
             if(((notes>>((2-i)*8))&255)!=0) begin
               phase[i]<=0; increment[i]<=pitch(notes>>((2-i)*8)); env[i]<=0; stage[i]<=1;
             end else stage[i]<=0;
           end
           display_note<=notes[14:8];
         end
         if(beat_sample==4687) for(i=0;i<3;i=i+1) stage[i]<=0;
         if(beat_sample==6249) begin beat_sample<=0; step<=step+1; end
         else beat_sample<=beat_sample+1;
       end
       // A live key is a 1-sample strobe, captured only on tick by the top.
       if(live_age!=0) begin
         live_age<=live_age-1;
         if(live_age==1563) stage[3]<=0;
       end
       if(live_note!=0) begin
         phase[3]<=0; increment[3]<=pitch(live_note); env[3]<=0; stage[3]<=1;
         live_age<=6250; display_note<=live_note;
       end
     end
   end
 end
endmodule
