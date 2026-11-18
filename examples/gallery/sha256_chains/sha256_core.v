// Original RTL transcription of FIPS 180-4 sections 4.1.2, 4.2.2, 6.2.2.
// No CPU, multipliers, unrolling, midstate, or cached schedule words.
module sha256_core(input clk, input reset, input start,
 input [511:0] block, input [255:0] iv,
 output reg [255:0] digest = 0, output reg done = 0);
 reg [31:0] a=0,b=0,c=0,d=0,e=0,f=0,g=0,h=0;
 reg [255:0] base=0;
 reg [31:0] w[0:15];
 integer i;
 initial for(i=0;i<16;i=i+1) w[i]=0;
 reg [5:0] round=0;
 reg running=0, finish=0;
 function [31:0] rotr;
   input [31:0] x; input integer n;
   begin rotr=(x>>n)|(x<<(32-n)); end
 endfunction
 // Uppercase Sigma mixes three rotations; lowercase sigma includes a shift.
 function [31:0] big0;
   input [31:0] x; begin big0=rotr(x,2)^rotr(x,13)^rotr(x,22); end
 endfunction
 function [31:0] big1;
   input [31:0] x; begin big1=rotr(x,6)^rotr(x,11)^rotr(x,25); end
 endfunction
 function [31:0] small0;
   input [31:0] x; begin small0=rotr(x,7)^rotr(x,18)^(x>>3); end
 endfunction
 function [31:0] small1;
   input [31:0] x; begin small1=rotr(x,17)^rotr(x,19)^(x>>10); end
 endfunction
 // K[t]: first 32 fractional bits of cube roots of the first 64 primes.
 // A combinational case maps to LUT logic, not an EBR.
 function [31:0] constant;
   input [5:0] t;
   begin case(t)
     6'd0: constant=32'h428a2f98;
     6'd1: constant=32'h71374491;
     6'd2: constant=32'hb5c0fbcf;
     6'd3: constant=32'he9b5dba5;
     6'd4: constant=32'h3956c25b;
     6'd5: constant=32'h59f111f1;
     6'd6: constant=32'h923f82a4;
     6'd7: constant=32'hab1c5ed5;
     6'd8: constant=32'hd807aa98;
     6'd9: constant=32'h12835b01;
     6'd10: constant=32'h243185be;
     6'd11: constant=32'h550c7dc3;
     6'd12: constant=32'h72be5d74;
     6'd13: constant=32'h80deb1fe;
     6'd14: constant=32'h9bdc06a7;
     6'd15: constant=32'hc19bf174;
     6'd16: constant=32'he49b69c1;
     6'd17: constant=32'hefbe4786;
     6'd18: constant=32'h0fc19dc6;
     6'd19: constant=32'h240ca1cc;
     6'd20: constant=32'h2de92c6f;
     6'd21: constant=32'h4a7484aa;
     6'd22: constant=32'h5cb0a9dc;
     6'd23: constant=32'h76f988da;
     6'd24: constant=32'h983e5152;
     6'd25: constant=32'ha831c66d;
     6'd26: constant=32'hb00327c8;
     6'd27: constant=32'hbf597fc7;
     6'd28: constant=32'hc6e00bf3;
     6'd29: constant=32'hd5a79147;
     6'd30: constant=32'h06ca6351;
     6'd31: constant=32'h14292967;
     6'd32: constant=32'h27b70a85;
     6'd33: constant=32'h2e1b2138;
     6'd34: constant=32'h4d2c6dfc;
     6'd35: constant=32'h53380d13;
     6'd36: constant=32'h650a7354;
     6'd37: constant=32'h766a0abb;
     6'd38: constant=32'h81c2c92e;
     6'd39: constant=32'h92722c85;
     6'd40: constant=32'ha2bfe8a1;
     6'd41: constant=32'ha81a664b;
     6'd42: constant=32'hc24b8b70;
     6'd43: constant=32'hc76c51a3;
     6'd44: constant=32'hd192e819;
     6'd45: constant=32'hd6990624;
     6'd46: constant=32'hf40e3585;
     6'd47: constant=32'h106aa070;
     6'd48: constant=32'h19a4c116;
     6'd49: constant=32'h1e376c08;
     6'd50: constant=32'h2748774c;
     6'd51: constant=32'h34b0bcb5;
     6'd52: constant=32'h391c0cb3;
     6'd53: constant=32'h4ed8aa4a;
     6'd54: constant=32'h5b9cca4f;
     6'd55: constant=32'h682e6ff3;
     6'd56: constant=32'h748f82ee;
     6'd57: constant=32'h78a5636f;
     6'd58: constant=32'h84c87814;
     6'd59: constant=32'h8cc70208;
     6'd60: constant=32'h90befffa;
     6'd61: constant=32'ha4506ceb;
     6'd62: constant=32'hbef9a3f7;
     6'd63: constant=32'hc67178f2;
   endcase end
 endfunction
 // Ch selects f or g using e; Maj takes the majority of a, b, c.
 wire [31:0] ch=(e&f)^((~e)&g);
 wire [31:0] maj=(a&b)^(a&c)^(b&c);
 // All additions are modulo 2^32. Yosys maps them to LUTs and carry chains.
 wire [31:0] t1=h+big1(e)+ch+constant(round)+w[0];
 wire [31:0] t2=big0(a)+maj;
 always @(posedge clk) begin
   done<=0;
   if(reset) begin running<=0; finish<=0; end
   else if(start) begin
     {a,b,c,d,e,f,g,h}<=iv; base<=iv; round<=0; running<=1; finish<=0;
     for(i=0;i<16;i=i+1) w[i]<=block[511-32*i-:32];
   end else if(running) begin
     h<=g; g<=f; f<=e; e<=d+t1; d<=c; c<=b; b<=a; a<=t1+t2;
     // Sliding schedule: before round t, w[j] is W[t+j]. The recurrence
     // appends W[t+16] = sigma1(W[t+14])+W[t+9]+sigma0(W[t+1])+W[t].
     for(i=0;i<15;i=i+1) w[i]<=w[i+1];
     w[15]<=small1(w[14])+w[9]+small0(w[1])+w[0];
     if(round==63) begin running<=0; finish<=1; end
     else round<=round+1;
   end else if(finish) begin
     digest<={base[255:224]+a,base[223:192]+b,base[191:160]+c,base[159:128]+d,
              base[127:96]+e,base[95:64]+f,base[63:32]+g,base[31:0]+h};
     finish<=0; done<=1;
   end
 end
endmodule
