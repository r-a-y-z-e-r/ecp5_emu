`timescale 1ns/1ps
// Observer/serialization test only. This is not a behavioral PLL model.
module fault_tb;
  reg clk=0, clkop=0, locked=0;
  integer mode, out, n;
  reg [1023:0] filename;
  reg [7:0] byte_value;
  wire tx;
  hc08_reporter #(.STARTUP_LAST(3)) dut(clk, clkop, locked, tx);
  always #20 clk=~clk;
  initial begin
    if (!$value$plusargs("MODE=%d",mode)) $fatal;
    if (!$value$plusargs("OUT=%s",filename)) $fatal;
    out=$fopen(filename,"wb");
    if (mode==1 || mode==2) forever #5 clkop=~clkop;
    else if (mode==3) forever #8 clkop=~clkop;
  end
  initial begin
    #15000000;
    if (mode==2) locked=1;
    #65000000;
    $fclose(out); $finish;
  end
  // UART 217 pin clocks/bit; sample at each data bit's center.
  initial begin
    #1; // Ignore the time-zero X -> initialized-idle resolution.
    forever begin
      @(negedge tx);
      #13020;
      for (n=0;n<8;n=n+1) begin
        byte_value[n]=tx; #8680;
      end
      if (!tx) $fatal(1,"bad stop bit");
      $fwrite(out,"%c",byte_value);
    end
  end
endmodule
