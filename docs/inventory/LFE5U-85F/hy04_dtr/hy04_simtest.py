#!/usr/bin/env python3
"""Exercise generated RTL, pulse widths, RAM replay and actual 115200 UART framing.

Requires iverilog/vvp. All generated files go into a temporary directory outside
the repository. The synthetic sensor is a transport fixture, not a DTR model.
"""
import json
import io
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

from hy04_analyse import SCHEDULE, analyse, parse, record
from hy04_gen import generate
import run as collector

FIRST_START = 2_500_000
REPLAY_INTERVAL = 625_000


BENCH = r'''
`timescale 1ns/1ps
module DTR(input STARTPULSE, output DTROUT0, DTROUT1, DTROUT2, DTROUT3,
           DTROUT4, DTROUT5, DTROUT6, DTROUT7);
  reg [7:0] code=8'h40;
  assign {DTROUT7,DTROUT6,DTROUT5,DTROUT4,DTROUT3,DTROUT2,DTROUT1,DTROUT0}=code;
  // Startup changes must survive in immutable RAM while later captures differ.
  initial begin #127; code=8'h41; #80; code=8'h43; end
  // Deliberately split updates, with a phase away from observer edges.
  // This fixture ignores starts while busy; held high triggers only once.
  initial forever begin
    @(posedge STARTPULSE);
    #70007; code=code^8'h03;
    #40; code=code^8'h04;
  end
endmodule

module tb;
  reg clk=0;
  always #20 clk=~clk;
  wire tx;
  top dut(clk, tx);
  reg [7:0] golden_previous=0;
  integer golden_seq=-1, pulses=0, falls=0, offset=0;
  reg was_high=0;
  always @(posedge clk) begin
    if ((dut.state==dut.BOOT && dut.tick==2) || dut.state==dut.PREP) begin
      golden_seq=dut.seq;
      golden_previous=dut.synced;
      $display("BASE %08X %02X", dut.seq, dut.synced);
    end
    if (dut.state==dut.CAPTURE && dut.synced!=golden_previous) begin
      $display("GOLD %08X %04X %02X", dut.seq, {1'b0,dut.index}, dut.synced);
      golden_previous=dut.synced;
    end
  end
  always @(negedge clk) begin
    if (golden_seq!=dut.seq) begin pulses=0; falls=0; end
    // Reset counts at PREP, before the new capture's first pulse.
    if (dut.state==dut.PREP) begin pulses=0; falls=0; end
    offset=dut.tick-1-dut.capture_tick;
    if (dut.startpulse && !was_high) begin
      if (dut.tick-1<dut.FIRST_START) $fatal(1,"STARTPULSE before startup guard");
      if (offset != 16+pulses*dut.gap) $fatal(1,"incorrect pulse rising offset");
      pulses=pulses+1;
    end
    if (!dut.startpulse && was_high) begin
      if (offset != 16+falls*dut.gap+dut.width) $fatal(1,"incorrect pulse width");
      falls=falls+1;
    end
    was_high=dut.startpulse;
    if (dut.state==dut.HEADER && !dut.replay && (pulses!=dut.count || falls!=dut.count))
      $fatal(1,"incorrect number of complete pulses");
  end
  // Decode the wire, rather than peeking at transmitter payload registers.
  reg [7:0] value;
  reg [8*128-1:0] line=0;
  integer bit_index, z_count=0;
  reg [47:0] line_tick=0;
  initial forever begin
    @(negedge tx);
    #4340;
    if (tx!==0) $fatal(1,"invalid UART start bit");
    for (bit_index=0;bit_index<8;bit_index=bit_index+1) begin
      #8680; value[bit_index]=tx;
    end
    #8680;
    if (tx!==1) $fatal(1,"invalid UART stop bit");
    if (value==10) begin
      $display("UART %012X %0s",line_tick,line);
      if (line[8*23-1:8*20]==24'h59345a) z_count=z_count+1;
      line=0;
      if (z_count==2) begin $display("SIM_DONE"); $finish; end
    end else if (value!=13) begin
      if (line==0) line_tick=dut.tick;
      line={line[8*127-1:0],value};
    end
    #4340;
  end
  initial begin #1000000000; $fatal(1,"simulation timeout"); end
endmodule
'''


def runner_selftest():
    """Mock FTDI access: the loader must finish before sleep/open/read."""
    events = []
    initial, done = record("I", 0x40, 1, 1), record("Z", 1, 2)
    chunks = iter([(done+"\r\n").encode(), (initial+"\r\n"+done+"\r\n").encode()])
    class Port:
        def __init__(self, *args, **kwargs):
            events.append("open")

        def __enter__(self):
            return self

        def __exit__(self, *args):
            events.append("close")

        def reset_input_buffer(self):
            events.append("clear")

        def read(self, size):
            events.append("read")
            return next(chunks)

    def load(*args, **kwargs):
        events.append("load")
        return SimpleNamespace(returncode=0, stdout="DONE\n", stderr="")

    def settle(seconds):
        assert seconds == 2.0
        events.append("wait")

    out = io.StringIO()
    with patch.dict("sys.modules", {"serial": SimpleNamespace(Serial=Port)}), \
         patch("sys.argv", ["run.py", "fixture.bit", "Y4", "1", "90"]), \
         patch.object(collector.subprocess, "run", side_effect=load), \
         patch.object(collector.time, "sleep", side_effect=settle), \
         patch("sys.stdout", out), patch("sys.stderr", io.StringIO()):
        collector.main()
    assert events == ["load", "wait", "open", "clear", "read", "read", "close"]
    assert out.getvalue().splitlines() == [done, initial, done]
    # A failed load must not open UART or wait for it.
    with patch.dict("sys.modules", {"serial": SimpleNamespace(Serial=Port)}), \
         patch("sys.argv", ["run.py", "fixture.bit", "Y4", "1", "90"]), \
         patch.object(collector.subprocess, "run", return_value=SimpleNamespace(
             returncode=5, stdout="", stderr="unable to claim usb device\n")), \
         patch.object(collector.time, "sleep") as sleep, \
         patch("sys.stderr", io.StringIO()):
        before = list(events)
        try:
            collector.main()
        except SystemExit as exc:
            assert exc.code == 5
        else:
            raise AssertionError("failed loader accepted")
        assert events == before
        sleep.assert_not_called()
    print("HY04 runner mock: PASS (load, wait 2 s, open UART, chronological output, wait for I before Z; no UART after failed load)")


def main():
    assert "FIRST_START=75000000, REPLAY_INTERVAL=12500000" in generate()
    runner_selftest()
    with tempfile.TemporaryDirectory(prefix="hy04-sim-", dir="/tmp") as tmp:
        root = Path(tmp)
        (root/"hy04.v").write_text(generate(rounds=1, readings=2,
                                         period_cycles=25_000, uart_delay_cycles=0,
                                         first_start_cycles=FIRST_START,
                                         replay_cycles=REPLAY_INTERVAL))
        (root/"tb.v").write_text(BENCH)
        subprocess.run(["iverilog", "-g2012", "-s", "tb", "-o", str(root/"sim"),
                        str(root/"hy04.v"), str(root/"tb.v")], check=True)
        result = subprocess.run(["vvp", str(root/"sim")], capture_output=True, text=True, check=True)
        assert "SIM_DONE" in result.stdout
        lines, late_lines, golden, bases = [], [], {}, {}
        for line in result.stdout.splitlines():
            if line.startswith("UART "):
                _, timestamp, payload = line.split(maxsplit=2)
                lines.append(payload)
                if int(timestamp, 16) >= FIRST_START*2//3:
                    late_lines.append(payload)
            elif line.startswith("BASE "):
                _, seq, base = line.split()
                bases[int(seq, 16)] = int(base, 16)
            elif line.startswith("GOLD "):
                _, seq, offset, value = line.split()
                golden.setdefault(int(seq, 16), []).append((int(offset, 16), int(value, 16)))
        run, = parse("\n".join(lines))
        assert run.done and run.initial == 0x40
        assert run.startup_replays >= 12
        assert run.windows[0].changes and run.windows[0].final == 0x43
        assert [w.kind for w in run.windows] == list(range(9)) + [9, 9]
        for w in run.windows:
            assert w.base == bases[w.seq]
            assert w.changes == golden.get(w.seq, []), (w.seq, w.changes, golden.get(w.seq))
            assert (w.width, w.gap, w.count, w.end) == SCHEDULE[w.kind]
        assert analyse(run)["timing_by_width_cycles"]["1"]["shapes"]["possible_bus_skew_or_intermediate_codes"]
        late, = parse("partial UART line\n" + "\n".join(late_lines))
        assert late.done and analyse(late)["complete"]
        assert late.initial == run.initial and late.windows == run.windows
        assert late.startup_replays < run.startup_replays
        late_i = next(line for line in late_lines if line.startswith("Y4I"))
        assert int(late_i[5:17], 16) < FIRST_START
        # Attaching after completion still recovers the exact startup capture.
        final_i = max(i for i, line in enumerate(lines) if line.startswith("Y4I"))
        completed, = parse("\n".join(lines[final_i:]))
        assert completed.done and completed.windows == [run.windows[0]]
        assert not analyse(completed)["complete"]
        print("HY04 RTL simulation: PASS (startup guard, immutable startup replay, late-reader recovery, all pulse schedules/widths, every buffered change, checksums, UART framing and repeated end marker)")
        print(json.dumps({"windows": len(run.windows), "UART_lines": len(lines),
                          "startup_replays": run.startup_replays,
                          "late_reader_replays": late.startup_replays,
                          "changes": sum(len(w.changes) for w in run.windows)}))


if __name__ == "__main__":
    main()
