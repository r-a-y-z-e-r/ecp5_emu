// Conway's Game of Life on a W x H torus: every cell is a flip-flop, and
// every clock is a generation of all the cells at once. A live cell with two
// or three live neighbours lives on, a dead one with three comes alive, and
// the rest die. The grid starts from a fixed random pattern; holding FIRE1
// (btn[1]) loads it again. Left alone, such a grid settles in a few hundred
// generations into still lifes and blinkers; so every 16th generation a few
// random cells, about one in 64, turn over, and it keeps changing. The LEDs
// show the population (the live cells, counted) divided by 8. About 10 000
// LUTs: a large design for the map.
module top(input clk_25mhz, input [6:0] btn, output [7:0] led);
  localparam W = 27, H = 27;

  // A fixed random pattern, about a quarter of the cells alive (xorshift32).
  function [W*H-1:0] seed(input integer unused);
    integer i;
    reg [31:0] x;
    begin
      x = 32'h2545f491;
      for (i = 0; i < W*H; i = i + 1) begin
        x = x ^ (x << 13);
        x = x ^ (x >> 17);
        x = x ^ (x << 5);
        seed[i] = x[3] & x[11];
      end
    end
  endfunction
  localparam [W*H-1:0] SEED = seed(0);

  reg [1:0] sync = 0;
  always @(posedge clk_25mhz) sync <= {sync[0], btn[1]};

  // Noise: a 32-bit LFSR (taps 32, 22, 2, 1), a step every generation, and
  // a turn of cells every 16th generation.
  reg [31:0] noise = 32'hace12468;
  always @(posedge clk_25mhz)
    noise <= {noise[30:0], noise[31] ^ noise[21] ^ noise[1] ^ noise[0]};
  reg [3:0] generation = 0;
  always @(posedge clk_25mhz) generation <= generation + 1;
  wire sprinkle = generation == 0;

  reg [W*H-1:0] cell = SEED;
  wire [W*H-1:0] next;
  genvar r, c;
  generate
    for (r = 0; r < H; r = r + 1) begin : row
      for (c = 0; c < W; c = c + 1) begin : col
        // The eight neighbours, round the edges of the torus.
        localparam U = ((r + H - 1) % H) * W, M = r * W, D = ((r + 1) % H) * W;
        localparam L = (c + W - 1) % W, R = (c + 1) % W;
        wire [7:0] n = {cell[U + L], cell[U + c], cell[U + R], cell[M + L],
                        cell[M + R], cell[D + L], cell[D + c], cell[D + R]};
        wire [3:0] live = n[0] + n[1] + n[2] + n[3] + n[4] + n[5] + n[6] + n[7];
        // The cell turns over at a sprinkle when six bits of the noise, its
        // own six (7 is odd, so they differ), are all 1: one in 64.
        localparam I = M + c;
        wire turn = sprinkle && noise[(I * 5) % 32] && noise[(I * 5 + 7) % 32]
                    && noise[(I * 5 + 14) % 32] && noise[(I * 5 + 21) % 32]
                    && noise[(I * 5 + 28) % 32] && noise[(I * 5 + 35) % 32];
        assign next[M + c] = (live == 3 || (live == 2 && cell[M + c])) ^ turn;
      end
    end
  endgenerate
  always @(posedge clk_25mhz) cell <= sync[1] ? SEED : next;

  // The population: the live cells of each row counted, then the rows.
  integer i, j;
  reg [15:0] count;
  reg [7:0] in_row;
  always @* begin
    count = 0;
    for (i = 0; i < H; i = i + 1) begin
      in_row = 0;
      for (j = 0; j < W; j = j + 1) in_row = in_row + cell[i * W + j];
      count = count + in_row;
    end
  end
  reg [15:0] population = 0;
  always @(posedge clk_25mhz) population <= count;
  assign led = population[10:3];
endmodule
