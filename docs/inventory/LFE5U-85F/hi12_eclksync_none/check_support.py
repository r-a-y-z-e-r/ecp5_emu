"""Check UART diagnostics and recorded HI12 counts without hardware access.
Usage: python3 -B check_support.py
"""
import contextlib
import io
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch
sys.dont_write_bytecode = True
from hi12_gen import generate
from hi12_model import decode, inspect as inspect_record, startup_candidates
import run


class CaptureTests(unittest.TestCase):
    def capture(self, chunks, loads=1, loader_error=None, ticks=None):
        with tempfile.TemporaryDirectory(prefix='hi12-runner-') as tmp:
            log = Path(tmp)/'capture.log'
            order = []
            port = MagicMock()
            port.__enter__.return_value = port
            port.read.side_effect = chunks
            def loader(*args, **kwargs):
                order.append('load')
                self.assertEqual(args[0][:3], ['openFPGALoader', '-b', 'ulx3s'])
                if loader_error:
                    raise loader_error
            def serial(*args, **kwargs):
                order.append('serial')
                return port
            argv = ['run.py', 'top.bit', 'mock-port', '--output', str(log),
                    '--loads', str(loads), '--seconds', '0.1']
            with patch.object(sys, 'argv', argv), patch.dict(sys.modules, {'serial': SimpleNamespace(Serial=serial)}), patch.object(run.subprocess, 'run', side_effect=loader), patch.object(run.time, 'monotonic', side_effect=ticks or [0, 0, 1]*loads), contextlib.redirect_stdout(io.StringIO()):
                status = run.main()
            return status, log.read_text(), [p.read_bytes() for p in sorted(Path(tmp).glob('*.bin'))], order

    def test_heartbeat_and_partial_bytes_survive_all_failed_loads(self):
        raw = b'\xff\r\nHB0 02 00001000\r\nHC0 truncated'
        status, log, files, order = self.capture([raw, raw], loads=2)
        self.assertEqual((status, files, order), (1, [raw, raw], ['load', 'serial']*2))
        self.assertEqual(log.count('ERROR: no complete'), 2)
        self.assertIn('LOCK=0 OBSERVER_SEEN=1', log)

    def test_silent_load_keeps_an_empty_raw_file(self):
        status, log, files, _ = self.capture([b''])
        self.assertEqual((status, files), (1, [b'']))
        self.assertIn('bytes=0', log)

    def test_complete_record_is_saved_without_filtering_raw_bytes(self):
        hc = b'HC0 '+b'0'*176
        raw = b'HB0 03 00000100\r\n'+hc+b'\r\npartial'
        status, log, files, _ = self.capture([raw])
        self.assertEqual((status, files), (0, [raw]))
        self.assertIn(hc.decode(), log)
        self.assertIn('complete=1', log)

    def test_loader_failure_keeps_raw_file_and_never_opens_serial(self):
        status, log, files, order = self.capture([], loader_error=OSError('mock loader failure'))
        self.assertEqual((status, files, order), (1, [b''], ['load']))
        self.assertIn('mock loader failure', log)

    def test_read_failure_keeps_already_received_bytes(self):
        raw = b'HB0 02 00001000\r\npartial'
        status, log, files, _ = self.capture([raw, OSError('mock read failure')], ticks=[0, 0, 0])
        self.assertEqual((status, files), (1, [raw]))
        self.assertIn('mock read failure', log)

    def test_existing_output_is_preserved_before_any_load(self):
        with tempfile.TemporaryDirectory(prefix='hi12-runner-') as tmp:
            log = Path(tmp)/'capture.log'
            log.write_text('preserve')
            with patch.object(sys, 'argv', ['run.py', 'top.bit', 'mock-port', '--output', str(log)]), patch.object(run.subprocess, 'run') as loader, contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                run.main()
            loader.assert_not_called()
            self.assertEqual(log.read_text(), 'preserve')


# Deliberately idealized primitives: these only exercise the UART when the
# observer is absent/unlocked. They do not validate PLL feedback or analog bits.
PRIMITIVES = r'''
`timescale 1ns/1ps
module EHXPLLL #(parameter CLKI_DIV=1,CLKFB_DIV=4,CLKOP_DIV=6,
 CLKOP_CPHASE=2,CLKOP_FPHASE=0,CLKOP_ENABLE="ENABLED",FEEDBK_PATH="CLKOP",
 PLLRST_ENA="DISABLED",INTFB_WAKE="DISABLED",STDBY_ENABLE="DISABLED",
 DPHASE_SOURCE="DISABLED",OUTDIVIDER_MUXA="DIVA",OUTDIVIDER_MUXB="DIVB",
 OUTDIVIDER_MUXC="DIVC",OUTDIVIDER_MUXD="DIVD")
(input CLKI,CLKFB,RST,STDBY,PHASESEL0,PHASESEL1,PHASEDIR,PHASESTEP,
 PHASELOADREG,PLLWAKESYNC,ENCLKOP,ENCLKOS,ENCLKOS2,ENCLKOS3,
 output reg CLKOP=0,output reg LOCK=0);
initial LOCK=!$test$plusargs("NO_LOCK") && !$test$plusargs("NO_OBSERVER");
always #5 if (!$test$plusargs("NO_OBSERVER")) CLKOP=~CLKOP;
endmodule
module DCCA(input CLKI,CE,output CLKO); assign CLKO=CLKI & CE; endmodule
module ECLKSYNCB(input ECLKI,STOP,output ECLKO);
reg [3:0] stages=15;
always @(negedge ECLKI) if ($time>0) stages<={stages[2:0],STOP};
assign ECLKO=ECLKI & ~stages[3];
endmodule
module CLKDIVF #(parameter DIV="2.0") (input CLKI,RST,ALIGNWD,output reg CDIVX=0);
always @(posedge CLKI) CDIVX<=~CDIVX;
endmodule
module tb;
reg clk=0; wire tx; top dut(clk,tx); always #20 clk=~clk;
integer k,j; reg [7:0] c;
initial begin
  for(k=0;k<34;k=k+1) begin
    @(negedge tx); #13020;
    for(j=0;j<8;j=j+1) begin c[j]=tx; #8680; end
    if (!tx) $fatal(1,"bad UART stop");
    $write("%c",c);
  end
  if (dut.finished) $fatal(1,"failure injection unexpectedly completed");
  $finish;
end
initial begin #6000000; $fatal(1,"no heartbeat while observer failed"); end
endmodule
'''


class HeartbeatTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('iverilog') and shutil.which('vvp'), 'requires Icarus Verilog')
    def test_uart_survives_unlocked_and_missing_observer(self):
        with tempfile.TemporaryDirectory(prefix='hi12-heartbeat-') as tmp:
            tmp = Path(tmp)
            (tmp/'primitives.v').write_text(PRIMITIVES)
            for variant in ('none_default', 'sync_2'):
                (tmp/'top.v').write_text(generate(variant))
                subprocess.run(['iverilog', '-g2012', '-s', 'tb', '-o', 'sim', 'primitives.v', 'top.v'], cwd=tmp, check=True)
                for fault, expected in [('NO_LOCK', '02'), ('NO_OBSERVER', '00')]:
                    with self.subTest(variant=variant, fault=fault):
                        output = subprocess.run(['vvp', 'sim', '+'+fault], cwd=tmp, check=True, capture_output=True).stdout
                        complete, heartbeats = run.parse(output)
                        self.assertEqual(complete, [])
                        self.assertEqual(len(heartbeats), 2)
                        self.assertEqual(heartbeats[-1][4:6], expected)
                        self.assertGreater(int(heartbeats[-1][7:], 16), int(heartbeats[0][7:], 16))


class BoardCaptureTests(unittest.TestCase):
    def records(self):
        files = sorted((Path(__file__).parent/'run_2').glob('*.bin'))
        self.assertEqual(len(files), 15)
        for path in files:
            records, heartbeats = run.parse(path.read_bytes())
            self.assertGreater(len(records), 100, path.name)
            self.assertFalse(heartbeats, path.name)
            self.assertEqual(len(set(records)), 1, path.name)
            yield path, records[0]

    def test_all_fifteen_raw_captures_fit_a_zero_deficit_and_measured_phase(self):
        for path, record in self.records():
            with self.subTest(path=path.name):
                result = inspect_record(record)
                self.assertIn(0, result['candidates'])
                self.assertEqual(result['offsets'], [0])
                self.assertEqual(result['intervals'], [14 if result['seven'] else 8])
                self.assertEqual(result['widths'], {0: [6], 1: [8]} if result['seven'] else {0: [4], 1: [4]})

    def test_fractional_counts_preserve_control_boot_phase_and_uncertainty(self):
        records = {record for path, record in self.records() if path.name.startswith('hi12-sync_35.')}
        self.assertEqual(len(records), 3)
        results = [inspect_record(record) for record in records]
        self.assertTrue(all(0 in r['candidates'] for r in results))
        self.assertTrue(any(1 in r['candidates'] for r in results))
        self.assertTrue(all(4 not in r['candidates'] for r in results))

    def test_lost_cycles_between_snapshots_cannot_hide_as_initial_phase(self):
        for _, record in self.records():
            variant, snapshots, _ = decode(record)
            i, p, f = snapshots[1]
            snapshots[1] = (i, p-10, f-10)
            self.assertFalse(startup_candidates(snapshots, variant in (2, 4)))


if __name__ == '__main__':
    unittest.main()
