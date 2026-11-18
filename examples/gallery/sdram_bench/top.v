// Burst-mode SDRAM benchmark: a pattern source, checker, phase sequencer and
// decimal reporter, all plain RTL in the 25 MHz board-clock domain.
module top(input clk_25mhz, input [6:0] btn, input ftdi_txd,
 output ftdi_rxd, output [7:0] led,
 output sdram_clk, sdram_cke, sdram_csn, sdram_rasn, sdram_casn, sdram_wen,
 output [1:0] sdram_ba, output [12:0] sdram_a,
 output [1:0] sdram_dqm, inout [15:0] sdram_d);
 wire clk=clk_25mhz;
 reg [5:0] reset_count=0;
 wire reset=!(&reset_count);
 always @(posedge clk) if(reset) reset_count<=reset_count+1;
 localparam BOUNDARY=0, KEY_WAIT=1, MARKER=2, MARKER_WAIT=3,
  WSTART=4, WWAIT=5, RSTART=6, RWAIT=7, REPORT=8, PRINTING=9;
 reg [3:0] state=BOUNDARY;
 reg full=0, selected_full=1, automatic_full=0;
 reg [31:0] pass=1, errors=0, write_cycles=0, read_cycles=0;
 reg sticky_error=0;
 reg [23:0] first_addr=0;
 reg [15:0] first_expected=0, first_read=0;
 wire [23:0] address;
 // The low and high address bits both affect every row's pattern. The pass
 // seed changes at every pass, including repeated small passes: stale words
 // from the preceding pass therefore fail. This nonzero 16-bit LFSR has no
 // fixed points and repeats after 65535 passes; adjacent seeds always differ.
 reg [15:0] seed=16'hace1;
 wire [15:0] pattern=address[15:0] ^ {address[7:0],address[23:16]} ^
   seed;
 wire ready, phase_done, read_valid;
 wire [15:0] rdata;
 wire [31:0] phase_cycles;
 wire start_phase=(state==WSTART || state==RSTART) && ready;
 burst_controller ram(clk,reset,start_phase,state==WSTART,full,pattern,address,
  read_valid,rdata,ready,phase_done,phase_cycles,
  sdram_clk,sdram_cke,sdram_csn,sdram_rasn,sdram_casn,sdram_wen,
  sdram_ba,sdram_a,sdram_dqm,sdram_d);
 wire busy, available;
 wire [7:0] key;
 wire [6:0] pressed;
 reg take=0;
 reg marker_send=0;
 reg [2:0] marker_pos=0;
 reg [7:0] marker_data=0;
 wire report_send, report_done;
 wire [7:0] report_data;
 gallery_io io(clk,reset,ftdi_txd,ftdi_rxd,btn,pressed,
  marker_send|report_send, marker_send ? marker_data:report_data,
  busy,key,available,take);
 bench_report printer(clk,reset,state==REPORT,pass,full,write_cycles,read_cycles,errors,
  first_addr,first_expected,first_read,busy,report_send,report_data,report_done);
 // Eight equal intervals of the current phase, with seven cumulative LEDs.
 wire [2:0] progress=full ? address[23:21]:address[19:17];
 assign led={sticky_error, (7'b1111111 >> (7-progress))};
 function [7:0] marker_char;
   input [2:0] p;
   begin
     case(p)
       0:marker_char="R";1:marker_char="E";2:marker_char="S";3:marker_char="T";
       4:marker_char="A";5:marker_char="R";6:marker_char="T";default:marker_char=10;
     endcase
   end
 endfunction
 always @(posedge clk) begin
   marker_send<=0; take<=0;
   if(reset) begin state<=BOUNDARY; full<=0; selected_full<=1; automatic_full<=0; end
   else begin
     if(read_valid && rdata!=pattern) begin
       errors<=errors+1; sticky_error<=1;
       if(errors==0) begin first_addr<=address; first_expected<=pattern; first_read<=rdata; end
     end
     case(state)
       BOUNDARY: if(available && !take) begin take<=1; state<=KEY_WAIT; end
         else if(!take) begin
           // The first pass is always 2 MiB; subsequent passes use the
           // selected size. Commands received at this boundary override it.
           full<=automatic_full ? selected_full:1'b0;
           errors<=0; first_addr<=0; first_expected<=0; first_read<=0; state<=WSTART;
         end
       KEY_WAIT: begin
         state<=BOUNDARY;
         // key still names the popped entry here: FIFO take is registered.
         case(key)
           "r": begin
             pass<=1; seed<=16'hace1; errors<=0; sticky_error<=0;
             first_addr<=0; first_expected<=0; first_read<=0;
             write_cycles<=0; read_cycles<=0;
             full<=0; selected_full<=1; automatic_full<=0; marker_pos<=0; state<=MARKER;
           end
           "s": begin selected_full<=0; automatic_full<=1; end
           "f": begin selected_full<=1; automatic_full<=1; end
           default: begin end
         endcase
       end
       // Never reset the transmitter or receive FIFO on r. Finish its
       // current byte before the marker; later queued keys retain order.
       MARKER: if(!busy && !marker_send) begin
         marker_data<=marker_char(marker_pos); marker_send<=1;
         if(marker_pos==7) state<=MARKER_WAIT; else marker_pos<=marker_pos+1;
       end
       MARKER_WAIT: if(!busy && !marker_send) state<=BOUNDARY;
       WSTART: if(ready) state<=WWAIT;
       WWAIT: if(phase_done) begin write_cycles<=phase_cycles; state<=RSTART; end
       RSTART: if(ready) state<=RWAIT;
       RWAIT: if(phase_done) begin read_cycles<=phase_cycles; state<=REPORT; end
       REPORT: state<=PRINTING;
       PRINTING: if(report_done) begin
         pass<=pass+1; automatic_full<=1;
         seed<={seed[14:0],seed[15]^seed[13]^seed[12]^seed[10]}; state<=BOUNDARY;
       end
       default: state<=BOUNDARY;
     endcase
   end
 end
endmodule
