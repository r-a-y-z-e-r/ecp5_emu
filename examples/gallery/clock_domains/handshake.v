// Four phases: req up, ack up, req down, ack down. The bundled data is
// set before req and held until ack has returned low. No bus synchroniser.
module handshake_lesson(input osc, input orst, input clk, input reset,
 input run, input [31:0] pass_number, output ready,
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

 localparam M=4096;
 wire go,ack_osc,req_clk;
 reg req=0,ack=0;
 reg [31:0] payload=0, source=0, reference=0;
 reg [12:0] sent=0;
 reg [1:0] state=0;
 cdc_sync start(osc,orst,run,go);
 cdc_sync back(osc,orst,ack,ack_osc);
 cdc_sync forward(clk,reset,req,req_clk);
 assign ready=!orst && !go;
 always @(posedge osc or posedge orst)
   if (orst) begin req<=0; payload<=0; source<=0; sent<=0; state<=0; end
   else if (!go) begin
     req<=0; payload<=0; source<=32'hc001c0de^pass_number; sent<=0; state<=0;
   end else case(state)
     0: if (sent<M) begin payload<=source; state<=1; end
     1: begin req<=1; state<=2; end
     2: if (ack_osc) begin req<=0; state<=3; end
     3: if (!ack_osc) begin
       sent<=sent+1'b1; source<=lfsr_next(source); state<=0;
     end
   endcase
 always @(posedge clk or posedge reset)
   if (reset) begin ack<=0; words<=0; errors<=0; crc<=32'hffffffff; reference<=0; done<=0; end
   else if (!run) begin
     ack<=0; words<=0; errors<=0; crc<=32'hffffffff;
     reference<=32'hc001c0de^pass_number; done<=0;
   end else begin
     if (req_clk && !ack) begin
       ack<=1; words<=words+1'b1;
       if (payload!=reference) errors<=errors+1'b1;
       reference<=lfsr_next(reference); crc<=crc_word(crc,payload,32);
     end
     if (!req_clk && ack) ack<=0;
     if (words==M && !req_clk && !ack) done<=1;
   end
endmodule

// The request toggle is also the pulse's sequence number. Ack must match
// before another toggle is launched. Count is a held result bus: done is
// synchronised back, and run going low acknowledges receipt of the result.
module pulse_lesson(input clk, input reset, input clk40, input rst40,
 input run, output ready, output reg [31:0] sent=0,
 output reg [31:0] counted=0, output reg done=0);
 localparam P=4096;
 reg request=0,ack=0;
 wire ack_clk,request40,go;
 cdc_sync forward(clk40,rst40,request,request40);
 cdc_sync back(clk,reset,ack,ack_clk);
 cdc_sync start(clk40,rst40,run,go);
 assign ready=!rst40 && !go;
 always @(posedge clk or posedge reset)
   if (reset) begin request<=0; sent<=0; end
   else if (!run) begin request<=0; sent<=0; end
   else if (sent<P && ack_clk==request) begin request<=!request; sent<=sent+1'b1; end
 always @(posedge clk40 or posedge rst40)
   if (rst40) begin ack<=0; counted<=0; done<=0; end
   else if (!go) begin ack<=0; counted<=0; done<=0; end
   else begin
     if (request40!=ack) begin counted<=counted+1'b1; ack<=request40; end
     if (counted==P) done<=1;
   end
endmodule
