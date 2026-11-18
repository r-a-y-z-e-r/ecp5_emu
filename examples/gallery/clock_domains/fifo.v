// 1024 x 18 physical words. Each entry owns TWO complete INIT9 rows;
// different entries can never collide even under the emulator's row rule.
module async_fifo(input wclk, input wrst, input put, input [15:0] data,
 output reg full=0, input rclk, input rrst, input get,
 output [15:0] q, output reg empty=1);
 reg [10:0] wbin=0, wgray=0, rbin=0, rgray=0;
 wire [10:0] rgray_w, wgray_r;
 cdc_sync #(11) rp(wclk,wrst,rgray,rgray_w);
 cdc_sync #(11) wp(rclk,rrst,wgray,wgray_r);
 wire write=put && !full, read=get && !empty;
 wire [10:0] wn=wbin+write, rn=rbin+read;
 wire [10:0] wg=wn^(wn>>1), rg=rn^(rn>>1);
 always @(posedge wclk or posedge wrst)
   if (wrst) begin wbin<=0; wgray<=0; full<=0; end
   else begin
     wbin<=wn; wgray<=wg;
     full<=wg=={~rgray_w[10:9],rgray_w[8:0]};
   end
 always @(posedge rclk or posedge rrst)
   if (rrst) begin rbin<=0; rgray<=0; empty<=1; end
   else begin rbin<=rn; rgray<=rg; empty<=rg==wgray_r; end

 // The read CE is essential: no speculative read at the empty pointer.
 // Yosys maps this simple dual-clock RAM to one DP16KD, width 18/18.
 (* ram_style="block" *) reg [17:0] memory[0:1023];
 reg [17:0] read_data=0;
 always @(posedge wclk) if (write) memory[wbin[9:0]]<={2'b0,data};
 always @(posedge rclk) if (read) read_data<=memory[rbin[9:0]];
 assign q=read_data[15:0];
endmodule

module fifo_lesson(input clk40, input rst40, input clk33, input rst33,
 input run, input [31:0] pass_number, output ready40, output ready33,
 output reg done=0, output reg [31:0] words=0, output reg [31:0] errors=0, output reg [31:0] crc=32'hffffffff);
 // Reflected Ethernet CRC32, initial all ones, complement at the end.
// Bytes arrive least significant first; bits arrive least significant first.
function [31:0] crc_word;
 input [31:0] crc, value;
 input integer bits;
 reg [31:0] c;
 integer k;
 begin
   c=crc;
   for (k=0;k<bits;k=k+1)
     c=(c>>1)^(((c[0]^value[k])!=0) ? 32'hedb88320 : 32'b0);
   crc_word=c;
 end
endfunction
function [31:0] lfsr_next;
 input [31:0] value;
 begin lfsr_next={value[30:0],value[31]^value[21]^value[1]^value[0]}; end
endfunction

 localparam N=262144;
 wire run40,run33;
 cdc_sync go40(clk40,rst40,run,run40);
 cdc_sync go33(clk33,rst33,run,run33);
 assign ready40=!rst40 && !run40;
 assign ready33=!rst33 && !run33;
 reg [18:0] written=0, issued=0;
 reg [31:0] source=0, reference=0;
 reg [13:0] pace=0;
 wire full,empty;
 // 8192 clocks of traffic, then 8192 of silence. Traffic fills the
 // FIFO; silence lets the consumer drain it, irrespective of phase.
 wire put=run40 && written<N && !pace[13];
 wire get=run33 && issued<N && !empty;
 wire [15:0] q;
 async_fifo fifo(clk40,rst40,put,source[15:0],full,
                 clk33,rst33,get,q,empty);
 reg valid=0;
 // These flags are testbench coverage only, deliberately absent from UART.
 reg saw_full=0, saw_empty=0;
 always @(posedge clk40 or posedge rst40)
   if (rst40) begin written<=0; source<=0; pace<=0; saw_full<=0; end
   else if (!run40) begin
     written<=0; source<=32'h1aceb00c^pass_number; pace<=0; saw_full<=0;
   end else begin
     pace<=pace+1'b1;
     if (full) saw_full<=1;
     if (put && !full) begin written<=written+1'b1; source<=lfsr_next(source); end
   end
 always @(posedge clk33 or posedge rst33)
   if (rst33) begin
     issued<=0; words<=0; errors<=0; crc<=32'hffffffff;
     reference<=0; valid<=0; done<=0; saw_empty<=0;
   end else if (!run33) begin
     issued<=0; words<=0; errors<=0; crc<=32'hffffffff;
     reference<=32'h1aceb00c^pass_number; valid<=0; done<=0; saw_empty<=0;
   end else begin
     valid<=get;
     if (get) issued<=issued+1'b1;
     if (empty && words>=4096 && words<N) saw_empty<=1;
     if (valid) begin
       words<=words+1'b1;
       if (q!=reference[15:0]) errors<=errors+1'b1;
       reference<=lfsr_next(reference);
       crc<=crc_word(crc,{16'b0,q},16);
       if (words==N-1) done<=1;
     end
   end
 `ifdef CDC_SIM
 always @(posedge clk33) if (done) begin
   assert(words==N && errors==0 && saw_empty)
     else $fatal(1,"FIFO count/content/empty coverage");
 end
 always @(posedge clk40) if (run40 && written==N)
   assert(saw_full) else $fatal(1,"FIFO never filled");
 `endif
endmodule
