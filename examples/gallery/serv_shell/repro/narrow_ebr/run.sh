#!/bin/sh
# Regression check for the narrow READBEFOREWRITE EBR gap found in serv_shell.
# Yosys 0.70 synth_ecp5 maps SERV's serv_rf_ram (576 x 2 bits, width=2) to a DP16KD
# with DATA_WIDTH_A/B=2 and WRITEMODE_A/B=READBEFOREWRITE. Commit 847a9ef fixed
# fast mode's refusal after board series HB11. Same-clock cross-port reads
# return the old word under READBEFOREWRITE (HB09), supported by 04268bb in
# the event kernel and 8f3673e in fast mode. This reproducer now runs as a
# regression check.
set -eu
root=$(cd "$(dirname "$0")/../../../../.." && pwd)
work="$root/target/gallery/serv_shell_narrow_ebr"
rm -rf "$work" && mkdir -p "$work/src"
cp "$root/examples/gallery/serv_shell/serv/serv_rf_ram.v" "$work/src/"
cat > "$work/src/top.v" <<'EOF'
module top(input wire clk_25mhz, input wire [6:0] btn, output wire [7:0] led);
  reg [9:0] waddr = 0, raddr = 0;
  always @(posedge clk_25mhz) begin
    waddr <= waddr + 1'b1;
    raddr <= raddr + 1'b1;
  end
  wire [1:0] rdata;
  serv_rf_ram #(.width(2), .csr_regs(4)) rf(
    .i_clk(clk_25mhz), .i_waddr(waddr), .i_wdata(btn[1:0]), .i_wen(btn[2]),
    .i_raddr(raddr), .i_ren(1'b1), .o_rdata(rdata)
  );
  assign led = {6'd0, rdata};
endmodule
EOF
cp "$root/examples/gallery/serv_shell/top.lpf" "$work/src/top.lpf"
"$root/examples/ulx3s/build.sh" "$work/src" "$work"
"$root/target/release/ecp5_emu" emulate "$work/top.config" --names "$work/top_routed.json" \
  --fast --startup configured --scope observed --clock net:clk_25mhz \
  --period-ns 40 --cycles 100 --probe net:led
