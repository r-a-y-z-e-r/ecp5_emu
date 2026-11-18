// No CPU. Three concurrent CDC protocols, and a board-clock UART reporter.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output [7:0] led);
 wire clk=clk_25mhz;
 wire clk40,clk33,clkosc,locked;
 gallery_pll clocks(clk,clk40,clk33,locked);
 OSCG #(.DIV(8)) oscillator(.OSC(clkosc));

 // UART and debouncers survive application restarts, including the byte
 // already in the transmitter. Only power-on resets the serial plumbing.
 reg [5:0] power_count=0;
 wire power_reset=!(&power_count);
 always @(posedge clk) if (power_reset) power_count<=power_count+1'b1;
 reg send=0,take=0;
 reg [7:0] data=0;
 wire busy,available;
 wire [7:0] key;
 wire [6:0] pressed;
 gallery_io io(clk,power_reset,ftdi_txd,ftdi_rxd,btn,pressed,send,data,busy,key,available,take);
 wire key_event=available && !take;
 wire restart=(key_event && key=="r") || pressed[1];
 wire frequency_key=key_event && key=="f";
 always @(posedge clk) take<=available && !take;

 reg reset_request=1;
 reg [5:0] reset_hold=63;
 wire lock25;
 cdc_sync lock_sync(clk,power_reset,locked,lock25);
 always @(posedge clk) begin
   if (restart) begin reset_request<=1; reset_hold<=63; end
   else if (reset_hold!=0) reset_hold<=reset_hold-1'b1;
   else if (lock25) reset_request<=0;
 end
 wire reset,rst40,rst33,rstosc;
 cdc_reset reset25(clk,reset_request,reset);
 cdc_reset reset40(clk40,reset_request,rst40);
 cdc_reset reset33(clk33,reset_request,rst33);
 cdc_reset resetosc(clkosc,reset_request,rstosc);

 reg run=0;
 reg [31:0] pass_number=1;
 wire fready40,fready33,hready,pready;
 wire fdone,pdone,hdone;
 wire [31:0] fwords,ferrors,fcrc,hwords,herrors,hcrc,psent,pcounted;
 fifo_lesson stream(clk40,rst40,clk33,rst33,run,pass_number,
   fready40,fready33,fdone,fwords,ferrors,fcrc);
 handshake_lesson hs(clkosc,rstosc,clk,reset,run,pass_number,
   hready,hdone,hwords,herrors,hcrc);
 pulse_lesson pulses(clk,reset,clk40,rst40,run,pready,psent,pcounted,pdone);
 wire [3:0] ready;
 cdc_sync #(4) ready_sync(clk,reset,{pready,hready,fready33,fready40},ready);
 wire fifo_done,pulse_done;
 cdc_sync fifo_done_sync(clk,reset,fdone,fifo_done);
 cdc_sync pulse_done_sync(clk,reset,pdone,pulse_done);

 wire [27:0] gray40,gray33,grayosc,g40,g33,go;
 cdc_meter meter40(clk40,rst40,gray40,led[1]);
 cdc_meter meter33(clk33,rst33,gray33,led[2]);
 cdc_meter meterosc(clkosc,rstosc,grayosc,led[3]);
 cdc_sync #(28) sample40(clk,reset,gray40,g40);
 cdc_sync #(28) sample33(clk,reset,gray33,g33);
 cdc_sync #(28) sampleosc(clk,reset,grayosc,go);
 reg [22:0] heartbeat=0;
 always @(posedge clk or posedge reset)
   if (reset) heartbeat<=0; else heartbeat<=heartbeat+1'b1;
 assign led[0]=heartbeat[22];
 assign led[4]=locked;
 assign led[6:5]=0;
 reg sticky_error=0;
 assign led[7]=sticky_error;
 function [27:0] ungray;
   input [27:0] gray;
   integer k;
   begin
     ungray[27]=gray[27];
     for (k=26;k>=0;k=k-1) ungray[k]=ungray[k+1]^gray[k];
   end
 endfunction
 reg measuring=0,freq_pending=0;
 reg [24:0] window=0;
 reg [27:0] base40=0,base33=0,baseosc=0;
 reg [31:0] freq40=0,freq33=0,freqosc=0;
 wire freq_printed;
 always @(posedge clk or posedge reset)
   if (reset) begin
     measuring<=0; freq_pending<=0; window<=0;
     base40<=0; base33<=0; baseosc<=0; freq40<=0; freq33<=0; freqosc<=0;
   end else begin
     if (freq_printed) freq_pending<=0;
     if (frequency_key && !measuring && !freq_pending) begin
       base40<=ungray(g40); base33<=ungray(g33); baseosc<=ungray(go);
       window<=24999999; measuring<=1;
     end else if (measuring) begin
       if (window==0) begin
         freq40<={4'b0,(ungray(g40)-base40)};
         freq33<={4'b0,(ungray(g33)-base33)};
         freqosc<={4'b0,(ungray(go)-baseosc)};
         measuring<=0; freq_pending<=1;
       end else window<=window-1'b1;
     end
   end

 localparam WAIT_READY=0,MARKER=1,START=2,WORK=3,REPORT=4,FREQUENCY=5;
 reg [2:0] state=WAIT_READY;
 reg [7:0] pos=0;
 reg marker_pending=0;
 wire slot=!busy && !send;
 wire marker_printed=state==MARKER && slot && pos==7;
 assign freq_printed=state==FREQUENCY && slot && pos==31;
 always @(posedge clk) begin
   if (restart) marker_pending<=1;
   else if (marker_printed) marker_pending<=0;
 end
 reg [31:0] report_words=0,report_errors=0,report_crc=0,
   hs_words=0,hs_errors=0,hs_crc=0,report_sent=0,report_counted=0;
 function [7:0] hexchar;
   input [3:0] n;
   begin hexchar=n<10 ? 8'd48+{4'b0,n} : 8'd55+{4'b0,n}; end
 endfunction
 function [7:0] restart_char;
   input [7:0] p;
   begin case(p)
     0:restart_char="R";1:restart_char="E";2:restart_char="S";3:restart_char="T";
     4:restart_char="A";5:restart_char="R";6:restart_char="T";default:restart_char=10;
   endcase end
 endfunction
 // Fixed-width hexadecimal reports: every numeric field is eight digits.
 function [7:0] pass_char;
   input [7:0] p;
   begin case(p)
     0: pass_char=80;
     1: pass_char=65;
     2: pass_char=83;
     3: pass_char=83;
     4: pass_char=32;
     5: pass_char=hexchar(pass_number[28+:4]);
     6: pass_char=hexchar(pass_number[24+:4]);
     7: pass_char=hexchar(pass_number[20+:4]);
     8: pass_char=hexchar(pass_number[16+:4]);
     9: pass_char=hexchar(pass_number[12+:4]);
     10: pass_char=hexchar(pass_number[8+:4]);
     11: pass_char=hexchar(pass_number[4+:4]);
     12: pass_char=hexchar(pass_number[0+:4]);
     13: pass_char=32;
     14: pass_char=70;
     15: pass_char=73;
     16: pass_char=70;
     17: pass_char=79;
     18: pass_char=32;
     19: pass_char=hexchar(report_words[28+:4]);
     20: pass_char=hexchar(report_words[24+:4]);
     21: pass_char=hexchar(report_words[20+:4]);
     22: pass_char=hexchar(report_words[16+:4]);
     23: pass_char=hexchar(report_words[12+:4]);
     24: pass_char=hexchar(report_words[8+:4]);
     25: pass_char=hexchar(report_words[4+:4]);
     26: pass_char=hexchar(report_words[0+:4]);
     27: pass_char=32;
     28: pass_char=69;
     29: pass_char=82;
     30: pass_char=82;
     31: pass_char=32;
     32: pass_char=hexchar(report_errors[28+:4]);
     33: pass_char=hexchar(report_errors[24+:4]);
     34: pass_char=hexchar(report_errors[20+:4]);
     35: pass_char=hexchar(report_errors[16+:4]);
     36: pass_char=hexchar(report_errors[12+:4]);
     37: pass_char=hexchar(report_errors[8+:4]);
     38: pass_char=hexchar(report_errors[4+:4]);
     39: pass_char=hexchar(report_errors[0+:4]);
     40: pass_char=32;
     41: pass_char=67;
     42: pass_char=82;
     43: pass_char=67;
     44: pass_char=32;
     45: pass_char=hexchar(report_crc[28+:4]);
     46: pass_char=hexchar(report_crc[24+:4]);
     47: pass_char=hexchar(report_crc[20+:4]);
     48: pass_char=hexchar(report_crc[16+:4]);
     49: pass_char=hexchar(report_crc[12+:4]);
     50: pass_char=hexchar(report_crc[8+:4]);
     51: pass_char=hexchar(report_crc[4+:4]);
     52: pass_char=hexchar(report_crc[0+:4]);
     53: pass_char=32;
     54: pass_char=72;
     55: pass_char=83;
     56: pass_char=32;
     57: pass_char=hexchar(hs_words[28+:4]);
     58: pass_char=hexchar(hs_words[24+:4]);
     59: pass_char=hexchar(hs_words[20+:4]);
     60: pass_char=hexchar(hs_words[16+:4]);
     61: pass_char=hexchar(hs_words[12+:4]);
     62: pass_char=hexchar(hs_words[8+:4]);
     63: pass_char=hexchar(hs_words[4+:4]);
     64: pass_char=hexchar(hs_words[0+:4]);
     65: pass_char=32;
     66: pass_char=69;
     67: pass_char=82;
     68: pass_char=82;
     69: pass_char=32;
     70: pass_char=hexchar(hs_errors[28+:4]);
     71: pass_char=hexchar(hs_errors[24+:4]);
     72: pass_char=hexchar(hs_errors[20+:4]);
     73: pass_char=hexchar(hs_errors[16+:4]);
     74: pass_char=hexchar(hs_errors[12+:4]);
     75: pass_char=hexchar(hs_errors[8+:4]);
     76: pass_char=hexchar(hs_errors[4+:4]);
     77: pass_char=hexchar(hs_errors[0+:4]);
     78: pass_char=32;
     79: pass_char=67;
     80: pass_char=82;
     81: pass_char=67;
     82: pass_char=32;
     83: pass_char=hexchar(hs_crc[28+:4]);
     84: pass_char=hexchar(hs_crc[24+:4]);
     85: pass_char=hexchar(hs_crc[20+:4]);
     86: pass_char=hexchar(hs_crc[16+:4]);
     87: pass_char=hexchar(hs_crc[12+:4]);
     88: pass_char=hexchar(hs_crc[8+:4]);
     89: pass_char=hexchar(hs_crc[4+:4]);
     90: pass_char=hexchar(hs_crc[0+:4]);
     91: pass_char=32;
     92: pass_char=80;
     93: pass_char=85;
     94: pass_char=76;
     95: pass_char=83;
     96: pass_char=69;
     97: pass_char=32;
     98: pass_char=hexchar(report_sent[28+:4]);
     99: pass_char=hexchar(report_sent[24+:4]);
     100: pass_char=hexchar(report_sent[20+:4]);
     101: pass_char=hexchar(report_sent[16+:4]);
     102: pass_char=hexchar(report_sent[12+:4]);
     103: pass_char=hexchar(report_sent[8+:4]);
     104: pass_char=hexchar(report_sent[4+:4]);
     105: pass_char=hexchar(report_sent[0+:4]);
     106: pass_char=32;
     107: pass_char=hexchar(report_counted[28+:4]);
     108: pass_char=hexchar(report_counted[24+:4]);
     109: pass_char=hexchar(report_counted[20+:4]);
     110: pass_char=hexchar(report_counted[16+:4]);
     111: pass_char=hexchar(report_counted[12+:4]);
     112: pass_char=hexchar(report_counted[8+:4]);
     113: pass_char=hexchar(report_counted[4+:4]);
     114: pass_char=hexchar(report_counted[0+:4]);
     115: pass_char=10;
     default: pass_char=10;
   endcase end
 endfunction
 function [7:0] freq_char;
   input [7:0] p;
   begin case(p)
     0: freq_char=70;
     1: freq_char=82;
     2: freq_char=69;
     3: freq_char=81;
     4: freq_char=32;
     5: freq_char=hexchar(freq40[28+:4]);
     6: freq_char=hexchar(freq40[24+:4]);
     7: freq_char=hexchar(freq40[20+:4]);
     8: freq_char=hexchar(freq40[16+:4]);
     9: freq_char=hexchar(freq40[12+:4]);
     10: freq_char=hexchar(freq40[8+:4]);
     11: freq_char=hexchar(freq40[4+:4]);
     12: freq_char=hexchar(freq40[0+:4]);
     13: freq_char=32;
     14: freq_char=hexchar(freq33[28+:4]);
     15: freq_char=hexchar(freq33[24+:4]);
     16: freq_char=hexchar(freq33[20+:4]);
     17: freq_char=hexchar(freq33[16+:4]);
     18: freq_char=hexchar(freq33[12+:4]);
     19: freq_char=hexchar(freq33[8+:4]);
     20: freq_char=hexchar(freq33[4+:4]);
     21: freq_char=hexchar(freq33[0+:4]);
     22: freq_char=32;
     23: freq_char=hexchar(freqosc[28+:4]);
     24: freq_char=hexchar(freqosc[24+:4]);
     25: freq_char=hexchar(freqosc[20+:4]);
     26: freq_char=hexchar(freqosc[16+:4]);
     27: freq_char=hexchar(freqosc[12+:4]);
     28: freq_char=hexchar(freqosc[8+:4]);
     29: freq_char=hexchar(freqosc[4+:4]);
     30: freq_char=hexchar(freqosc[0+:4]);
     31: freq_char=10;
     default: freq_char=10;
   endcase end
 endfunction

 always @(posedge clk) begin
   send<=0;
   if (reset || restart) begin
     run<=0; pass_number<=1; state<=WAIT_READY; pos<=0; sticky_error<=0;
     report_words<=0; report_errors<=0; report_crc<=0;
     hs_words<=0; hs_errors<=0; hs_crc<=0; report_sent<=0; report_counted<=0;
   end else case(state)
     WAIT_READY: if (&ready) begin
       pos<=0;
       if (marker_pending) state<=MARKER; else state<=START;
     end
     MARKER: if (slot) begin
       data<=restart_char(pos); send<=1;
       if (pos==7) begin pos<=0; state<=START; end else pos<=pos+1'b1;
     end
     START: begin run<=1; state<=WORK; end
     WORK: if (fifo_done && pulse_done && hdone) begin
       // Bundled result buses have been held since BEFORE done crossed
       // two flops. Lowering run acknowledges their capture, four-phase.
       report_words<=fwords; report_errors<=ferrors; report_crc<=~fcrc;
       hs_words<=hwords; hs_errors<=herrors; hs_crc<=~hcrc;
       report_sent<=psent; report_counted<=pcounted;
       if (ferrors!=0 || herrors!=0 || psent!=pcounted) sticky_error<=1;
       run<=0; pos<=0; state<=REPORT;
     end
     REPORT: if (slot) begin
       data<=pass_char(pos); send<=1;
       if (pos==115) begin
         pos<=0;
         if (freq_pending) state<=FREQUENCY;
         else begin pass_number<=pass_number+1'b1; state<=WAIT_READY; end
       end else pos<=pos+1'b1;
     end
     FREQUENCY: if (slot) begin
       data<=freq_char(pos); send<=1;
       if (pos==31) begin
         pos<=0; pass_number<=pass_number+1'b1; state<=WAIT_READY;
       end else pos<=pos+1'b1;
     end
     default: state<=WAIT_READY;
   endcase
 end
endmodule
