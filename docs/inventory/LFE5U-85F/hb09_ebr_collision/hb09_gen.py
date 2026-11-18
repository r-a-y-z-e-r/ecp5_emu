"""Write hb09.v: what a Lattice ECP5 DP16KD block RAM does when both ports address
the same word in the same clock edge (HB09).
Usage: python3 hb09_gen.py > hb09.v; python3 hb09_gen.py s > hb09s.v

HB09 asks:
  - When one port reads while the other writes the same word, what does the
    reading port output (old word, new word, bit mix, or something else)?
  - When both ports write the same word, what does the array hold afterward?

Eight DP16KD are instantiated directly, all on the 25 MHz clock with CEA = CEB = 1,
OCEA = OCEB = 1, RSTA = RSTB = 0, CS = 0, CSDECODE = 0b000, GSR = DISABLED,
RESETMODE = SYNC:

  EBR  DATA_WIDTH_A  DATA_WIDTH_B  WRITEMODE_A      WRITEMODE_B      REGMODE
  e0   2             2             READBEFOREWRITE  READBEFOREWRITE  NOREG
  e1   1             1             READBEFOREWRITE  READBEFOREWRITE  NOREG
  e2   4             4             READBEFOREWRITE  READBEFOREWRITE  NOREG
  e3   9             9             READBEFOREWRITE  READBEFOREWRITE  NOREG
  e4   18            18            READBEFOREWRITE  READBEFOREWRITE  NOREG
  e5   9             9             NORMAL           NORMAL           NOREG
  e6   9             9             WRITETHROUGH     WRITETHROUGH     NOREG
  e7   9             9             READBEFOREWRITE  READBEFOREWRITE  OUTREG

Traffic (from a 64-bit maximal-length LFSR x^64+x^63+x^61+x^60+1, stepped 32
times per clock from SEED = 0x1D2CACE15EED0001):
  - port A: w_a = lfsr[1:0],  we_a = lfsr[4], d_a = lfsr[23:6]
  - port B: w_b = lfsr[3:2],  we_b = lfsr[5], d_b = lfsr[41:24]
Every EBR receives the same fields (data truncated to its port width). Both
ports map their 2-bit word address w to the same low word-address pins
AD[shift+1:shift] without splitting halves on AD13, so they collide on ~1/4
of clocks. Unused AD pins are 0, except for width 18 (e4) where AD0/AD1 are
the active-high 9-bit write-group enables (BE[1:0]) and are tied to 1'b1 so
full 18-bit writes occur.
INITVAL_00..3F hold a fixed pseudo-random 9-bit pattern per EBR (INIT_MEMS).

Line format (115200 8N1):
  "R" + 16 hex (win_lfsr) + 128 * 31 hex (samples 0..127, sample 0 in MSBs) + CR LF
Each 124-bit sample packs {e0_a[1:0], e0_b[1:0], e1_a[0], e1_b[0], e2_a[3:0],
e2_b[3:0], e3_a[8:0], e3_b[8:0], e4_a[17:0], e4_b[17:0], e5_a[8:0], e5_b[8:0],
e6_a[8:0], e6_b[8:0], e7_a[8:0], e7_b[8:0], 2'b0} (122 data bits + 2 pad bits).

Alignment:
  - Sample k (k = 0..127) is the EBR DOA/DOB output BEFORE rising edge k of the
    window, and win_lfsr is the LFSR state at clock 0 of the window (L_0).
  - At rising edge k, the EBR receives traffic(L_k) where L_k = step^k(win_lfsr).
  - Sample k+1 (k = 0..126) is therefore the EBR output AFTER rising edge k:
    - NOREG: sample k+1 reflects traffic(L_k) directly.
    - OUTREG (OCE=1): sample k+1 captures the pre-edge-k internal latch, and
      sample k+2 (k = 0..125) reflects traffic(L_k).
  - Default (free-running): after sending a line, the next window starts on the
    next clock while the LFSR and EBR traffic run continuously.
  - Variant "s": records the first window starting at clock 0 after configuration
    (when memory equals INITVAL) and resends that same window forever while
    traffic continues running.
"""
import sys

# (name, DATA_WIDTH_A, DATA_WIDTH_B, WRITEMODE_A, WRITEMODE_B, REGMODE)
EBRS = [
    ("e0", 2, 2, "READBEFOREWRITE", "READBEFOREWRITE", "NOREG"),
    ("e1", 1, 1, "READBEFOREWRITE", "READBEFOREWRITE", "NOREG"),
    ("e2", 4, 4, "READBEFOREWRITE", "READBEFOREWRITE", "NOREG"),
    ("e3", 9, 9, "READBEFOREWRITE", "READBEFOREWRITE", "NOREG"),
    ("e4", 18, 18, "READBEFOREWRITE", "READBEFOREWRITE", "NOREG"),
    ("e5", 9, 9, "NORMAL", "NORMAL", "NOREG"),
    ("e6", 9, 9, "WRITETHROUGH", "WRITETHROUGH", "NOREG"),
    ("e7", 9, 9, "READBEFOREWRITE", "READBEFOREWRITE", "OUTREG"),
]

RECORD = [
    item
    for name, wa, wb, *_ in EBRS
    for item in ((name, "a", wa), (name, "b", wb))
]
WIDTH = sum(w for *_, w in RECORD)
PAD = -WIDTH % 4
SAMPLE_BITS = WIDTH + PAD
SAMPLES = 128

SEED = 0x1D2CACE15EED0001
LFSR_STEPS = 32
MASK64 = (1 << 64) - 1
SHIFT = {1: 0, 2: 1, 4: 2, 9: 3, 18: 4}


def lfsr_step(x):
    """Advance the 64-bit LFSR by LFSR_STEPS=32 single-bit shifts."""
    for _ in range(LFSR_STEPS):
        fb = ((x >> 63) ^ (x >> 62) ^ (x >> 60) ^ (x >> 59)) & 1
        x = ((x << 1) & MASK64) | fb
    return x


def traffic_fields(lfsr):
    """Return ((w_a, d_a, we_a), (w_b, d_b, we_b)) from a 64-bit LFSR state."""
    return (
        (lfsr & 0x3, (lfsr >> 6) & 0x3FFFF, (lfsr >> 4) & 1),
        ((lfsr >> 2) & 0x3, (lfsr >> 24) & 0x3FFFF, (lfsr >> 5) & 1),
    )


def cell_coords(width, addr2, bit_idx):
    """Map (width, 2-bit word address, bit index) to (word9, bit9) per EBR_ADDRESSING.md."""
    if width < 9:
        n = 8 // width
        return addr2 // n, (addr2 % n) * width + bit_idx
    return addr2 * (width // 9) + bit_idx // 9, bit_idx % 9


def _make_init_mems():
    mems = {}
    for idx, (name, *_) in enumerate(EBRS):
        state = (0x4842_3039 + idx * 0x9E37_79B9_7F4A_7C15) & MASK64
        words = []
        for _ in range(2048):
            state = (state * 6364136223846793005 + 1442695040888963407) & MASK64
            words.append((state >> 28) & 0x1FF)
        mems[name] = tuple(words)
    return mems


INIT_MEMS = _make_init_mems()


def _initval_params(words):
    """Pack 2048 9-bit words into 64 320-bit INITVAL parameters (16 20-bit chunks each)."""
    out = []
    for k in range(64):
        val = 0
        for j in range(16):
            w0 = words[32 * k + 2 * j]
            w1 = words[32 * k + 2 * j + 1]
            val |= ((w1 << 9) | w0) << (20 * j)
        out.append(f".INITVAL_{k:02X}(320'h{val:080X})")
    return out


def _addr_connections(prefix, bus_name, width):
    shift = SHIFT[width]
    conns = []
    for pin in range(14):
        if shift <= pin < shift + 2:
            sig = f"{bus_name}[{pin - shift}]"
        elif width == 18 and pin < 2:
            # In DATA_WIDTH=18, AD0/AD1 carry the active-high 9-bit write enables (BE[1:0]).
            sig = "1'b1"
        else:
            sig = "1'b0"
        conns.append(f".{prefix}{pin}({sig})")
    return conns


def _data_connections(prefix, bus_name, width):
    return [
        f".{prefix}{i}({bus_name}[{i}])" if i < width else f".{prefix}{i}(1'b0)"
        for i in range(18)
    ]


def _ebr_verilog(name, wa, wb, wma, wmb, regmode):
    ports = _data_connections("DIA", "d_a", wa) + _data_connections("DIB", "d_b", wb)
    ports += _addr_connections("ADA", "w_a", wa) + _addr_connections("ADB", "w_b", wb)
    ports += [f".DOA{i}({name}_a[{i}])" for i in range(18)]
    ports += [f".DOB{i}({name}_b[{i}])" for i in range(18)]
    ports += [
        ".CLKA(clk_25mhz)", ".CLKB(clk_25mhz)",
        ".CEA(1'b1)", ".CEB(1'b1)",
        ".OCEA(1'b1)", ".OCEB(1'b1)",
        ".WEA(we_a)", ".WEB(we_b)",
        ".RSTA(1'b0)", ".RSTB(1'b0)",
        ".CSA0(1'b0)", ".CSA1(1'b0)", ".CSA2(1'b0)",
        ".CSB0(1'b0)", ".CSB1(1'b0)", ".CSB2(1'b0)",
    ]
    params = [
        f".DATA_WIDTH_A({wa})", f".DATA_WIDTH_B({wb})",
        f'.WRITEMODE_A("{wma}")', f'.WRITEMODE_B("{wmb}")',
        f'.REGMODE_A("{regmode}")', f'.REGMODE_B("{regmode}")',
        '.CSDECODE_A("0b000")', '.CSDECODE_B("0b000")',
        '.GSR("DISABLED")', '.RESETMODE("SYNC")',
    ] + _initval_params(INIT_MEMS[name])
    return (
        f"  wire [17:0] {name}_a, {name}_b;\n"
        f"  DP16KD #(\n    " + ",\n    ".join(params) + f"\n  ) {name} (\n    "
        + ",\n    ".join(ports) + ");\n"
    )


def generate(once=False):
    hex_samples = SAMPLES * (SAMPLE_BITS // 4)
    total_bits = SAMPLES * SAMPLE_BITS
    ci_last = 1 + 16 + hex_samples + 1  # index of LF (CR is ci_last - 1)
    ci_bits = ci_last.bit_length()
    rows = "".join(
        f"//   {n:<3} {wa:<12} {wb:<12} {wma:<15} {wmb:<15} {rm}\n"
        for n, wa, wb, wma, wmb, rm in EBRS
    )
    fields = ", ".join(f"{n}_{p}[{w - 1}:0]" for n, p, w in RECORD)
    ebr_blocks = "".join(_ebr_verilog(*e) for e in EBRS)
    next_state = "SEND" if once else "REC"
    variant_note = (
        "// Variant s: records only the first window from clock 0 and resends it.\n"
        if once
        else "// Free-running: records a fresh 128-sample window after each line is sent.\n"
    )
    return f"""// HB09{"s" if once else ""}: DP16KD same-clock port collision behaviour.
// Generated by hb09_gen.py{" s" if once else ""}.
//   EBR DATA_WIDTH_A DATA_WIDTH_B WRITEMODE_A     WRITEMODE_B     REGMODE
{rows}// Sample ({SAMPLE_BITS} bits) = {{{", ".join(f"{n}.DO{p.upper()}[{w - 1}:0]" for n, p, w in RECORD)}, {PAD}'b0}}
// captured before each rising 25 MHz edge; win_lfsr is the LFSR state at
// clock 0 of the window (sample k+1 reflects traffic(step^k(win_lfsr)) for NOREG,
// and sample k+2 reflects traffic(step^k(win_lfsr)) for OUTREG).
// Line "R<16 hex win_lfsr><{hex_samples} hex samples>" CR LF at 115200 8N1.
{variant_note}module top(input clk_25mhz, output ftdi_rxd);
  function [63:0] step_lfsr(input [63:0] x);
    reg [63:0] t;
    integer i;
    begin
      t = x;
      for (i = 0; i < {LFSR_STEPS}; i = i + 1)
        t = {{t[62:0], t[63] ^ t[62] ^ t[60] ^ t[59]}};
      step_lfsr = t;
    end
  endfunction

  reg [63:0] lfsr = 64'h{SEED:016X};
  wire [1:0] w_a = lfsr[1:0];
  wire [1:0] w_b = lfsr[3:2];
  wire we_a = lfsr[4];
  wire we_b = lfsr[5];
  wire [17:0] d_a = lfsr[23:6];
  wire [17:0] d_b = lfsr[41:24];

{ebr_blocks}  wire [{SAMPLE_BITS - 1}:0] s = {{{fields}, {PAD}'b0}};

  reg [63:0] win_lfsr = 64'h{SEED:016X};
  reg [6:0] u = 0;
  reg [{total_bits - 1}:0] rec = 0;
  localparam REC = 0, SEND = 1;
  reg state = REC;

  function [7:0] hex(input [3:0] v);
    hex = v < 10 ? 8'h30 + v : 8'h37 + v;
  endfunction

  reg [{ci_bits - 1}:0] ci = 0;
  reg [7:0] ch;
  always @* begin
    if (ci == 0) ch = "R";
    else if (ci < 17) ch = hex(win_lfsr[63 - 4 * (ci - 1) -: 4]);
    else if (ci < {17 + hex_samples}) ch = hex(rec[{total_bits - 1} -: 4]);
    else if (ci == {ci_last - 1}) ch = 8'h0d;
    else ch = 8'h0a;
  end

  reg tx = 1, busy = 0;
  reg [7:0] baud = 0;
  reg [3:0] bitn = 0;
  reg [9:0] shift = 10'h3ff;

  always @(posedge clk_25mhz) begin
    lfsr <= step_lfsr(lfsr);
    case (state)
      REC: begin
        if (u == 0) win_lfsr <= lfsr;
        rec <= {{rec[{total_bits - SAMPLE_BITS - 1}:0], s}};
        u <= u + 1;
        if (u == {SAMPLES - 1}) begin
          state <= SEND;
          u <= 0;
        end
      end
      SEND:
        if (!busy) begin
          shift <= {{1'b1, ch, 1'b0}};
          busy <= 1;
          baud <= 0;
          bitn <= 0;
        end else if (baud == 216) begin
          baud <= 0;
          tx <= shift[0];
          shift <= {{1'b1, shift[9:1]}};
          bitn <= bitn + 1;
          if (bitn == 9) begin
            busy <= 0;
            if (ci >= 17 && ci < {17 + hex_samples})
              rec <= {{rec[{total_bits - 5}:0], rec[{total_bits - 1} -: 4]}};
            if (ci == {ci_last}) begin
              ci <= 0;
              state <= {next_state};
            end else ci <= ci + 1;
          end
        end else baud <= baud + 1;
    endcase
  end

  assign ftdi_rxd = tx;
endmodule
"""


if __name__ == "__main__":
    variant = sys.argv[1] if len(sys.argv) > 1 else ""
    if variant not in ("", "s"):
        raise SystemExit("Usage: python3 hb09_gen.py [s]")
    sys.stdout.write(generate(once=(variant == "s")))
