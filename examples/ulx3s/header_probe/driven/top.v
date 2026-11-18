// Drives ONLY GP0..GP3, at DRIVE=4 (top.lpf), and reads their pad buffers.
// Board loads are allowed only while J1/J2 are empty (2026-11-17: only the power cable
// is plugged in).
// SRAM loads only. Every other header signal remains an input with a pull.
module top(input clk_25mhz, input [27:4] gp, inout [3:0] gp_pwm, input [27:0] gn,
           output [7:0] led, output ftdi_rxd);
  parameter DRIVE_PWM = 1;
  wire [3:0] pwm;
  wire [3:0] pad_read;
  BB gp0_pad(.I(pwm[0]),.T(!DRIVE_PWM),.O(pad_read[0]),.B(gp_pwm[0]));
  BB gp1_pad(.I(pwm[1]),.T(!DRIVE_PWM),.O(pad_read[1]),.B(gp_pwm[1]));
  BB gp2_pad(.I(pwm[2]),.T(!DRIVE_PWM),.O(pad_read[2]),.B(gp_pwm[2]));
  BB gp3_pad(.I(pwm[3]),.T(!DRIVE_PWM),.O(pad_read[3]),.B(gp_pwm[3]));
  header_logic #(.DRIVE_PWM(DRIVE_PWM)) logic(.clk(clk_25mhz),.gp({gp,pad_read}),.gn(gn),
    .pwm(pwm),.led(led),.tx(ftdi_rxd));
endmodule
