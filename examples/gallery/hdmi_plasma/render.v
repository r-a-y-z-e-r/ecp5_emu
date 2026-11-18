// Eight pixel-clock stages; one pixel accepted every clock, no pixel RAM.
// Signed values stay inside the module (avoids signed-port tool issues).
module sine_lut(input clk, input [9:0] phase, output reg [8:0] value = 0);
  (* rom_style="block" *) reg [8:0] table_data[0:1023];
  initial $readmemh("sine.hex", table_data);
  always @(posedge clk) value <= table_data[phase];
endmodule

module beam_effect(input clk, input [9:0] x, y, input [15:0] tick,
  input effect, input [1:0] palette, output reg [23:0] rgb = 0,
  output de, hs, vs, last_pixel, raster_end);
  // Frame constants settle during blanking, never change in active video.
  wire [8:0] sin_bits, cos_bits;
  sine_lut spin_sin(clk, tick[9:0], sin_bits);
  sine_lut spin_cos(clk, tick[9:0] + 10'd256, cos_bits);
  wire signed [8:0] sn = $signed(sin_bits), cs = $signed(cos_bits);
  wire signed [8:0] zoom_wave = sn >>> 2, warp = sn >>> 1;
  wire signed [10:0] zoom = 11'sd192 + {{2{zoom_wave[8]}},zoom_wave};
  wire [9:0] phase_warp = {warp[8],warp};
  // Zoom deliberately uses S(T), not S(2T), for one coherent breathing turn.

  // S1: centred coordinates and three linear phases (all phases mod 1024).
  reg signed [10:0] u1 = 0, v1 = 0;
  reg [9:0] a1 = 0, b1 = 0, c1 = 0;
  always @(posedge clk) begin
    u1 <= $signed({1'b0,x}) - 11'sd320;
    v1 <= $signed({1'b0,y}) - 11'sd240;
    a1 <= (x + (x << 1)) + (tick[9:0] << 1);
    b1 <= (y << 2) - (tick[9:0] + (tick[9:0] << 1));
    c1 <= ((x + y) << 1) + tick[9:0] + phase_warp;
  end
  // S2: six independent signed products, suitable for MULT18X18D.
  reg [21:0] uu2 = 0, vv2 = 0;
  reg signed [19:0] uc2 = 0, vs2 = 0, us2 = 0, vc2 = 0;
  reg [9:0] a2 = 0, b2 = 0, c2 = 0;
  always @(posedge clk) begin
    uu2 <= u1 * u1; vv2 <= v1 * v1;
    uc2 <= u1 * cs; vs2 <= v1 * sn;
    us2 <= u1 * sn; vc2 <= v1 * cs;
    a2 <= a1; b2 <= b1; c2 <= c1;
  end
  // S3: squared-radius phase and Q8 rotation reduction. Widen before sums.
  wire signed [20:0] rot_u = {uc2[19],uc2} - $signed({vs2[19],vs2});
  wire signed [20:0] rot_v = {us2[19],us2} + $signed({vc2[19],vc2});
  reg signed [11:0] u3 = 0, v3 = 0;
  reg [9:0] a3 = 0, b3 = 0, c3 = 0, d3 = 0;
  wire [21:0] radius_squared = uu2 + vv2;
  always @(posedge clk) begin
    u3 <= rot_u[19:8]; v3 <= rot_v[19:8];
    a3 <= a2; b3 <= b2; c3 <= c2;
    d3 <= radius_squared[16:7] - {tick[8:0],1'b0};
  end
  // S4: synchronous sine reads, plus two zoom products (eight DSPs total
  // intended; actual mapped resources require a synthesis/build).
  wire [8:0] wa, wb, wc, wd;
  sine_lut wave_a(clk, a3, wa);
  sine_lut wave_b(clk, b3, wb);
  sine_lut wave_c(clk, c3, wc);
  sine_lut wave_d(clk, d3, wd);
  reg signed [22:0] uz4 = 0, vz4 = 0;
  always @(posedge clk) begin uz4 <= u3 * zoom; vz4 <= v3 * zoom; end
  // S5: four-wave sum; rotating/zooming lattice in wrapping pixel units.
  reg signed [11:0] sum5 = 0;
  reg [7:0] tile_u5 = 0, tile_v5 = 0;
  always @(posedge clk) begin
    sum5 <= {{3{wa[8]}},wa} + {{3{wb[8]}},wb} +
            {{3{wc[8]}},wc} + {{3{wd[8]}},wd};
    tile_u5 <= uz4[15:8] + tick[9:2];
    tile_v5 <= vz4[15:8] - tick[10:3];
  end
  // S6: scalar field to palette index. Cut-glass facets, dark gutters and
  // a thin luminous diamond distinguish the second effect from plasma.
  wire [5:0] fu = tile_u5[5:0], fv = tile_v5[5:0];
  wire [5:0] tri_u = fu[5] ? 6'd63 - fu : fu;
  wire [5:0] tri_v = fv[5] ? 6'd63 - fv : fv;
  wire [5:0] abs_u = fu >= 32 ? fu - 6'd32 : 6'd32 - fu;
  wire [5:0] abs_v = fv >= 32 ? fv - 6'd32 : 6'd32 - fv;
  wire [6:0] diamond = {1'b0,abs_u} + {1'b0,abs_v};
  wire tile_parity = tile_u5[6] ^ tile_v5[6];
  wire signed [11:0] biased_sum = sum5 + 12'sd1020;
  wire [7:0] plasma_index = biased_sum[10:3] + tick[9:2];
  wire [7:0] tile_index = ({2'b0,tri_u} << 2) + {2'b0,tri_u} +
    ({2'b0,tri_v} << 1) + {2'b0,tri_v} +
    (tile_parity ? 8'd64 : 8'd0) + tick[9:2];
  reg [9:0] palette_address6 = 0;
  reg dark6 = 0, shine6 = 0;
  always @(posedge clk) begin
    palette_address6 <= {palette, effect ? tile_index : plasma_index};
    dark6 <= effect && (fu < 2 || fv < 2);
    shine6 <= effect && diamond >= 29 && diamond <= 31;
  end
  // S7: RGB palette ROM. Entries 768..1023 are defined too (safe startup).
  (* rom_style="block" *) reg [23:0] colors[0:1023];
  initial $readmemh("palette.hex", colors);
  reg [23:0] color7 = 0;
  reg dark7 = 0, shine7 = 0;
  always @(posedge clk) begin
    color7 <= colors[palette_address6]; dark7 <= dark6; shine7 <= shine6;
  end
  // S8: channel-wise shading, without carry between RGB channels.
  always @(posedge clk) begin
    if (dark7) rgb <= {3'b0,color7[23:19],3'b0,color7[15:11],3'b0,color7[7:3]};
    else if (shine7) rgb <= {
      (8'd112 + {1'b0,color7[23:17]}),
      (8'd112 + {1'b0,color7[15:9]}),
      (8'd112 + {1'b0,color7[7:1]})};
    else rgb <= color7;
  end
  // Delay *all* raster signals by the same eight stages as RGB. In
  // particular CRC end tags cannot run ahead of the pipeline's last pixel.
  reg [7:0] dep = 0, hsp = 8'hff, vsp = 8'hff, lastp = 0, endp = 0;
  always @(posedge clk) begin
    dep <= {dep[6:0],x < 640 && y < 480};
    hsp <= {hsp[6:0],!(x >= 656 && x < 752)};
    vsp <= {vsp[6:0],!(y >= 490 && y < 492)};
    lastp <= {lastp[6:0],x == 639 && y == 479};
    endp <= {endp[6:0],x == 799 && y == 524};
  end
  assign de = dep[7], hs = hsp[7], vs = vsp[7];
  assign last_pixel = lastp[7], raster_end = endp[7];
endmodule
