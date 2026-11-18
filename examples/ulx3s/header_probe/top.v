// Board-safe: every J1/J2 signal is INPUT ONLY. Unknown external attachments
// must never be driven. The PWM channels run internally and report counts.
module top(input clk_25mhz, input [27:0] gp, input [27:0] gn,
           output [7:0] led, output ftdi_rxd);
  wire [3:0] pwm;
  header_logic #(.DRIVE_PWM(0)) logic(.clk(clk_25mhz),.gp(gp),.gn(gn),
    .pwm(pwm),.led(led),.tx(ftdi_rxd));
endmodule
