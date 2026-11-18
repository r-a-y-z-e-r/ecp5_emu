// Chip synth: original music, plain RTL, one 25 MHz domain, no DIP inputs.
module top(input clk_25mhz, input ftdi_txd, input [6:0] btn,
 output ftdi_rxd, output [7:0] led, output [3:0] audio_l,audio_r,audio_v,
 output oled_clk,oled_mosi,oled_dc,oled_csn,oled_resn);
 wire clk=clk_25mhz;
 wire [7:0] key,tx_data;
 wire available,tx_busy,tx_send,overflow;
 wire take_key;
 synth_io io(clk,ftdi_txd,ftdi_rxd,btn,tx_send,tx_data,tx_busy,key,available,take_key,overflow);
 function [6:0] key_note;
   input [7:0] k;
   begin case(k) "a":key_note=72; "s":key_note=74; "d":key_note=76;
     "f":key_note=77; "g":key_note=79; "h":key_note=81; default:key_note=0; endcase end
 endfunction
 // RUN, DRAIN, MARKER, WAIT_MARKER. Only incoming r may enter DRAIN.
 reg [1:0] mode=0;
 reg reset=0;
 wire enabled=mode==0;
 wire sample_toggle;
 wire tick,paused,audio_valid,oled_valid,oled_idle;
 wire [1535:0] history;
 wire [4:0] step;
 wire [6:0] note;
 wire [15:0] frame;
 wire [31:0] audio_crc,oled_crc;
 reg note_valid=0;
 reg [6:0] note_report=0;
 reg [31:0] note_sample=0;
 wire [31:0] sample_number;
 wire text_busy,text_done;
 reg start=0,sending=0;
 reg [2:0] kind=0;
 reg [31:0] text_crc=0;
 reg [15:0] text_frame=0;
 wire audio_take=text_done && sending && kind==0;
 wire oled_take=text_done && sending && kind==1;
 wire note_take=text_done && sending && kind==2;
 // Commands are consumed on sample boundaries; NOTE payload stays until sent.
 assign take_key=enabled && available && tick && !note_valid;
 wire [6:0] live_note=take_key?key_note(key):0;
 chip_synth synth(clk,reset,enabled,take_key && key=="p",live_note,tick,sample_toggle,sample_number,
   audio_l,audio_r,audio_v,history,step,note,paused,audio_valid,audio_take,audio_crc);
 synth_oled oled(clk,reset,enabled,history,step,note,paused,
   oled_clk,oled_mosi,oled_dc,oled_csn,oled_resn,oled_idle,oled_valid,oled_take,frame,oled_crc);
 synth_text text(clk,start,kind,text_frame,text_crc,note_report,
   tx_send,tx_data,tx_busy,text_busy,text_done);
 reg fault_sent=0;
 assign led={overflow,paused,step[4:0],sample_toggle};
 always @(posedge clk) begin
   reset<=0; start<=0;
   if(note_take) note_valid<=0;
   if(take_key && live_note!=0) begin note_valid<=1; note_report<=live_note; note_sample<=sample_number; end
   if(take_key && key=="r") mode<=1;
   if(text_done && sending) begin
     sending<=0;
     if(kind==3) mode<=0;
   end
   if(!sending && !text_busy && !start) begin
     if(audio_valid) begin kind<=0; text_crc<=audio_crc; start<=1; sending<=1; end
     else if(oled_valid) begin kind<=1; text_crc<=oled_crc; text_frame<=frame; start<=1; sending<=1; end
     else if(note_valid) begin kind<=2; text_crc<=note_sample; start<=1; sending<=1; end
     else if(overflow && !fault_sent) begin kind<=4; start<=1; sending<=1; fault_sent<=1; end
     else if(mode==1 && oled_idle) begin reset<=1; mode<=2; end
     else if(mode==2) begin kind<=3; start<=1; sending<=1; mode<=3; end
   end
 end
endmodule
