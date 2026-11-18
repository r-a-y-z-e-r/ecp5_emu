// Lexer categories: directives, comments, strings, escaped names, literals, attributes, keywords, operators.
`timescale 1ns/1ps
`define WIDTH 8
/* block comment
   spanning lines */
package types;
  typedef enum logic [1:0] {IDLE=2'b00, RUN=2'b01} state_t;
  typedef struct packed {logic valid; logic [`WIDTH-1:0] data;} packet_t;
endpackage
interface bus_if(input logic clk);
  logic ready;
  modport producer(input clk, output ready);
endinterface
(* keep = "true" *) module sample(input logic clk, rst, output logic [7:0] data);
  import types::*;
  state_t state;
  logic [7:0] \escaped.name ;
  string message = "quoted \"text\" and \\ slash";
  real scale = 1.25e-3;
  time period = 10ns;
  logic [31:0] based = 32'hde_ad_be_ef;
  logic [3:0] unknowns = 4'b10xz;
  always_ff @(posedge clk) begin
    if (rst) state <= IDLE;
    else state <= RUN;
  end
  always_comb begin
    unique case (state)
      IDLE: data = '0;
      default: data = {4'hA, unknowns} + 8'd1;
    endcase
  end
  function automatic int double_it(input int n); return n << 1; endfunction
  initial $display("%s %d", message, double_it(2));
endmodule
