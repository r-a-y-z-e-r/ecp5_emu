"""Hardware-free checks of capture preservation, deficit interpretation and heartbeat."""
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
from analyze import inspect
import run


def record(p=4, g=4, dcca_loss=0):
    words = []
    for i in (524288, 1048576):
        gr = i-64-dcca_loss
        words.extend((i, i-64, 64, gr, (i-p+1)//2, (i-p)//2,
                      (gr-g+1)//2, (gr-g)//2))
    return 'HCD '+''.join(f'{word:08X}' for word in words)


class AnalysisTests(unittest.TestCase):
    def test_distinguishes_history_and_immediate_cases(self):
        for p, g, phrase in [(0, 4, 'supports pre-running'), (4, 4, 'both pay'),
                             (0, 0, 'both pass'), (0, 5, 'supports pre-running')]:
            result = inspect(record(p, g))
            self.assertEqual(result['errors'], [])
            self.assertIn(phrase, result['reading'])
            self.assertEqual([s['G_deficit'] for s in result['snapshots']], [g, g])

    def test_dcca_loss_cannot_be_attributed_to_sync(self):
        result = inspect(record(dcca_loss=1))
        self.assertTrue(result['errors'])
        self.assertIn('invalid measurement', result['reading'])

    def test_deficit_must_remain_constant(self):
        good = record()
        changed = good[:4+64]+record(6, 6)[4+64:]
        self.assertTrue(inspect(changed)['errors'])


class RunnerTests(unittest.TestCase):
    def capture(self, chunks, loads=1, loader_error=None):
        with tempfile.TemporaryDirectory(prefix='hi13-runner-') as tmp:
            log = Path(tmp)/'capture.log'
            order = []
            port = MagicMock()
            port.__enter__.return_value = port
            port.read.side_effect = chunks
            def loader(command, **kwargs):
                self.assertEqual(command, ['openFPGALoader', '-b', 'ulx3s', 'top.bit'])
                order.append('load')
                if loader_error:
                    raise loader_error
            def serial(*args, **kwargs):
                order.append('serial')
                return port
            argv = ['run.py', 'top.bit', 'mock-port', '--output', str(log), '--loads', str(loads), '--seconds', '.1']
            with patch.object(sys, 'argv', argv), patch.dict(sys.modules, {'serial': SimpleNamespace(Serial=serial)}), patch.object(run.subprocess, 'run', side_effect=loader), patch.object(run.time, 'monotonic', side_effect=[0, 0, 1]*loads), contextlib.redirect_stdout(io.StringIO()):
                status = run.main()
            return status, log.read_text(), [p.read_bytes() for p in sorted(Path(tmp).glob('*.bin'))], order

    def test_load_then_serial_and_raw_bytes_preserved(self):
        raw = b'\xff\r\nHBD 01 00000040\r\n'+record().encode()+b'\r\npartial'
        status, log, files, order = self.capture([raw])
        self.assertEqual((status, files, order), (0, [raw], ['load', 'serial']))
        self.assertIn('complete=1', log)

    def test_all_incomplete_loads_are_saved_and_fail(self):
        raw = b'\xff\r\nHBD 01 00000040\r\npartial'
        status, log, files, order = self.capture([raw, b''], loads=2)
        self.assertEqual((status, files, order), (1, [raw, b''], ['load', 'serial']*2))
        self.assertEqual(log.count('ERROR: no complete'), 2)

    def test_loader_failure_never_opens_serial(self):
        status, log, files, order = self.capture([], loader_error=OSError('mock failure'))
        self.assertEqual((status, files, order), (1, [b''], ['load']))

    def test_changing_frozen_payload_fails(self):
        status, log, _, _ = self.capture([(record()+'\r\n'+record(0, 4)+'\r\n').encode()])
        self.assertEqual(status, 1)
        self.assertIn('frozen record changed', log)

    def test_existing_output_is_refused_before_loading(self):
        with tempfile.TemporaryDirectory(prefix='hi13-existing-') as tmp:
            log = Path(tmp)/'capture.log'
            log.write_text('preserve')
            with patch.object(sys, 'argv', ['run.py', 'top.bit', 'mock-port', '--output', str(log)]), patch.object(run.subprocess, 'run') as loader, contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                run.main()
            loader.assert_not_called()
            self.assertEqual(log.read_text(), 'preserve')


class HeartbeatTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which('iverilog') and shutil.which('vvp'), 'requires Icarus Verilog')
    def test_uart_survives_absent_lane_clocks(self):
        # These stubs intentionally do not model ECLKSYNC/CLKDIV startup.
        # They verify that absent test clocks cannot prevent pin-clock UART.
        primitives = '''
module DCCA(input CLKI,CE,output CLKO); assign CLKO=CLKI & CE; endmodule
module ECLKSYNCB(input ECLKI,STOP,output ECLKO); assign ECLKO=0; endmodule
module CLKDIVF #(parameter DIV="2.0")(input CLKI,RST,ALIGNWD,output CDIVX); assign CDIVX=0; endmodule
`timescale 1ns/1ps
module tb;
reg clk=0; wire tx; top dut(clk,tx); always #20 clk=~clk;
integer k,j; reg [7:0] c;
initial begin
 #1;
 for(k=0;k<34;k=k+1) begin
  @(negedge tx); #13020;
  for(j=0;j<8;j=j+1) begin c[j]=tx; #8680; end
  if (!tx) $fatal(1,"bad UART stop");
  $write("%c",c);
 end
 if (dut.pr || dut.pf || dut.qr || dut.qf) $fatal(1,"unexpected lane clocks");
 $finish;
end
initial begin #6000000; $fatal(1,"heartbeat stalled"); end
endmodule
'''
        with tempfile.TemporaryDirectory(prefix='hi13-heartbeat-') as tmp:
            tmp = Path(tmp)
            (tmp/'primitives.v').write_text(primitives)
            shutil.copyfile(Path(__file__).with_name('top.v'), tmp/'top.v')
            subprocess.run(['iverilog', '-g2012', '-s', 'tb', '-o', 'sim', 'primitives.v', 'top.v'], cwd=tmp, check=True)
            output = subprocess.run(['vvp', 'sim'], cwd=tmp, check=True, capture_output=True).stdout
            complete, heartbeats = run.parse(output)
            self.assertFalse(complete)
            self.assertEqual(len(heartbeats), 2, output)
            self.assertGreater(int(heartbeats[1][7:], 16), int(heartbeats[0][7:], 16))


if __name__ == '__main__':
    unittest.main()
