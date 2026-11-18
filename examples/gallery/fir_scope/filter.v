// Two implementations of exactly the same signed dot product. Both retain
// all 54 accumulator bits; the display truncates only after comparison.
// No signed PORTS (see dsp_mac.v): sign is on the wires inside and in top.v.
module filters(input clk,input reset,input sample_tick,input [17:0] sample,
               output reg valid=0,output reg [17:0] parallel_out=0,
               output reg [17:0] serial_out=0,output reg [31:0] mismatches=0);
  reg signed [17:0] coeff[0:30];
  initial $readmemh("coeff.hex",coeff);
  reg signed [17:0] delay[0:30];
  integer k;
  initial for(k=0;k<31;k=k+1) delay[k]=0;
  always @(posedge clk) begin
    if(reset) for(k=0;k<31;k=k+1) delay[k]<=0;
    else if(sample_tick) begin
      delay[0]<=sample;
      for(k=1;k<31;k=k+1) delay[k]<=delay[k-1];
    end
  end
  // A balanced five-level fabric adder tree. Every product has a distinct
  // delay sample, even though the coefficient sequence is symmetric.
  reg signed [35:0] product[0:31];
  reg signed [53:0] level1[0:15],level2[0:7],level3[0:3],level4[0:1];
  reg signed [53:0] parallel_sum=0;
  genvar g;
  generate for(g=0;g<31;g=g+1) begin:mults
    initial product[g]=0;
    always @(posedge clk) product[g]<=delay[g]*coeff[g];
  end endgenerate
  initial product[31]=0;
  integer a;
  initial begin
    for(a=0;a<16;a=a+1) level1[a]=0;
    for(a=0;a<8;a=a+1) level2[a]=0;
    for(a=0;a<4;a=a+1) level3[a]=0;
    for(a=0;a<2;a=a+1) level4[a]=0;
  end
  always @(posedge clk) begin
    for(a=0;a<16;a=a+1) level1[a]<=$signed(product[2*a])+$signed(product[2*a+1]);
    for(a=0;a<8;a=a+1) level2[a]<=level1[2*a]+level1[2*a+1];
    for(a=0;a<4;a=a+1) level3[a]<=level2[2*a]+level2[2*a+1];
    for(a=0;a<2;a=a+1) level4[a]<=level3[2*a]+level3[2*a+1];
    parallel_sum<=level4[0]+level4[1];
  end
  reg busy=0;
  reg [4:0] tap=0;
  wire signed [53:0] serial_sum;
  // rst clears the accumulator at the sample tick, one clock before tap 0.
  dsp_mac mac(clk,busy,sample_tick,delay[tap],coeff[tap],serial_sum);
  reg [5:0] age=63;
  always @(posedge clk) begin
    valid<=0;
    if(reset) begin
      busy<=0;tap<=0;age<=63;parallel_out<=0;serial_out<=0;mismatches<=0;
    end else begin
      if(sample_tick) begin busy<=1;tap<=0;age<=0;end
      else if(age!=63) age<=age+1;
      if(busy) begin
        if(tap==30) busy<=0; else tap<=tap+1;
      end
      if(age==34) begin
        valid<=1;parallel_out<=parallel_sum>>>16;serial_out<=serial_sum>>>16;
        if(parallel_sum!=serial_sum) mismatches<=mismatches+1;
      end
    end
  end
endmodule
