// Decimal UART report. One shared restoring divider (34 clocks) computes
// tenths of MiB/s; one shift/add-3 converter prints all decimal fields.
// No CPU, multipliers, or combinational wide division.
module bench_report(input clk, input reset, input start,
 input [31:0] pass, input full_size, input [31:0] write_cycles, read_cycles, errors,
 input [23:0] first_addr, input [15:0] first_expected, first_read,
 input busy, output reg send=0, output reg [7:0] data=0, output reg done=0);
 localparam IDLE=0, DIVIDE=1, FIELD=2, TEXT=3, CONVERT=4, DIGITS=5, DOT=6, FINISH=7;
 reg [3:0] state=IDLE;
 reg [4:0] field=0;
 reg [3:0] pos=0, digit=0;
 reg [5:0] count=0;
 reg which=0;
 reg [33:0] quotient=0;
 reg [32:0] remainder=0;
 reg [31:0] denominator=0, write_bw=0, read_bw=0, binary=0;
 reg [39:0] decimal=0;
 reg leading=1;
 wire [32:0] trial={remainder[31:0],quotient[33]};
 wire subtract=trial>={1'b0,denominator};
 wire [33:0] next_quotient={quotient[32:0],subtract};
 reg [39:0] adjusted;
 integer i;
 always @* begin
   adjusted=decimal;
   for(i=0;i<10;i=i+1)
     if(decimal[i*4+:4]>=5) adjusted[i*4+:4]=decimal[i*4+:4]+3;
 end
 wire fraction=field==7 || field==11;
 function [7:0] literal;
   input [4:0] f; input [3:0] p;
   begin
     literal=0;
     case(f)
     0: case(p)
       0: literal=8'd80;
       1: literal=8'd65;
       2: literal=8'd83;
       3: literal=8'd83;
       4: literal=8'd32;
       default: literal=0; endcase
     2: case(p)
       0: literal=8'd32;
       1: literal=8'd83;
       2: literal=8'd73;
       3: literal=8'd90;
       4: literal=8'd69;
       5: literal=8'd32;
       default: literal=0; endcase
     4: case(p)
       0: literal=8'd32;
       1: literal=8'd87;
       2: literal=8'd82;
       3: literal=8'd73;
       4: literal=8'd84;
       5: literal=8'd69;
       6: literal=8'd32;
       default: literal=0; endcase
     6: case(p)
       0: literal=8'd32;
       default: literal=0; endcase
     8: case(p)
       0: literal=8'd32;
       1: literal=8'd82;
       2: literal=8'd69;
       3: literal=8'd65;
       4: literal=8'd68;
       5: literal=8'd32;
       default: literal=0; endcase
     10: case(p)
       0: literal=8'd32;
       default: literal=0; endcase
     12: case(p)
       0: literal=8'd32;
       1: literal=8'd69;
       2: literal=8'd82;
       3: literal=8'd82;
       4: literal=8'd79;
       5: literal=8'd82;
       6: literal=8'd83;
       7: literal=8'd32;
       default: literal=0; endcase
     14: case(p)
       0: literal=8'd10;
       default: literal=0; endcase
     15: case(p)
       0: literal=8'd70;
       1: literal=8'd73;
       2: literal=8'd82;
       3: literal=8'd83;
       4: literal=8'd84;
       5: literal=8'd32;
       6: literal=8'd69;
       7: literal=8'd82;
       8: literal=8'd82;
       9: literal=8'd79;
       10: literal=8'd82;
       11: literal=8'd32;
       default: literal=0; endcase
     17: case(p)
       0: literal=8'd32;
       default: literal=0; endcase
     19: case(p)
       0: literal=8'd32;
       default: literal=0; endcase
     21: case(p)
       0: literal=8'd10;
       default: literal=0; endcase
       default: literal=0;
     endcase
   end
 endfunction
 always @(posedge clk) begin
   send<=0; done<=0;
   if(reset) state<=IDLE;
   else case(state)
     IDLE: if(start) begin
       quotient<=full_size ? 34'd8000000000:34'd500000000;
       denominator<=write_cycles; remainder<=0; count<=0; which<=0; state<=DIVIDE;
     end
     DIVIDE: begin
       quotient<=next_quotient;
       remainder<=subtract ? trial-{1'b0,denominator}:trial;
       if(count==33) begin
         if(!which) begin
           write_bw<=next_quotient[31:0]; which<=1; count<=0; remainder<=0;
           quotient<=full_size ? 34'd8000000000:34'd500000000; denominator<=read_cycles;
         end else begin read_bw<=next_quotient[31:0]; field<=0; state<=FIELD; end
       end else count<=count+1;
     end
     FIELD: begin
       pos<=0;
       case(field)
         1: binary<=pass;
         3: binary<=full_size ? 32:2;
         5: binary<=write_cycles;
         7: binary<=write_bw;
         9: binary<=read_cycles;
         11: binary<=read_bw;
         13: binary<=errors;
         16: binary<={8'd0,first_addr};
         18: binary<={16'd0,first_expected};
         20: binary<={16'd0,first_read};
         default: binary<=0;
       endcase
       if(field==1 || field==3 || field==5 || field==7 || field==9 ||
          field==11 || field==13 || field==16 || field==18 || field==20) begin
         decimal<=0; count<=0; state<=CONVERT;
       end else state<=TEXT;
     end
     TEXT: if(!busy && !send) begin
       if(literal(field,pos)!=0) begin data<=literal(field,pos); send<=1; pos<=pos+1; end
       else if(field==21 || (field==14 && errors==0)) state<=FINISH;
       else begin field<=field+1; state<=FIELD; end
     end
     CONVERT: begin
       decimal<={adjusted[38:0],binary[31]}; binary<={binary[30:0],1'b0};
       if(count==31) begin digit<=9; leading<=1; state<=DIGITS; end
       else count<=count+1;
     end
     DIGITS: if(!busy && !send) begin
       if(decimal[digit*4+:4]!=0 || !leading || digit==0 || (fraction && digit==1)) begin
         data<="0"+{4'd0,decimal[digit*4+:4]}; send<=1; leading<=0;
       end
       if(digit==0) begin field<=field+1; state<=FIELD; end
       else begin digit<=digit-1; if(fraction && digit==1) state<=DOT; end
     end
     DOT: if(!busy && !send) begin data<="."; send<=1; state<=DIGITS; end
     FINISH: if(!busy && !send) begin done<=1; state<=IDLE; end
     default: state<=IDLE;
   endcase
 end
endmodule
