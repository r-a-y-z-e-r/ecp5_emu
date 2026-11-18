#!/usr/bin/env python3
"""Regression checks for required models and window-record summaries; no GUI run."""
import contextlib
import io
from pathlib import Path
import sys
import tempfile
import unittest
sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'examples/gallery'))
import check
import gui_run

SDRAM = (b'peripheral: sdram init=yes reads=1 writes=1 timing_violations=0 '
         b'unknown_samples=0 refresh_shortfalls=0\n')
OLED = (b'peripheral: oled frames=151 crc32=89ABCDEF commands=944 data_bytes=1855488 '
        b'errors=0 unknown_commands=0 display_on=yes\n')
UART = b'RESTART\r\nF0000 C89ABCDEF S000 L3 B62\r\n'

def picture(n, crc='89ABCDEF', frames=None):
    frames = n if frames is None else frames
    return f'# oled picture {n} on frames {frames} data crc {crc} picture crc 11223344 at 0,0 384x256'

class Tooling(unittest.TestCase):
    def test_compare_marker_and_prefix(self):
        self.assertTrue(check.compare(UART, b'old' + UART + b'next', 'RESTART')[0])
        self.assertFalse(check.compare(UART, UART[:-1], 'RESTART')[0])

    def test_sdram_default_and_missing(self):
        self.assertTrue(check.peripheral_status({}, SDRAM, b'')[0])
        self.assertFalse(check.peripheral_status({}, b'', b'')[0])
        self.assertFalse(check.peripheral_status({}, OLED, UART)[0])

    def test_sdram_errors(self):
        for old, new in [(b'init=yes', b'init=no'), (b'timing_violations=0', b'timing_violations=1'),
                         (b'unknown_samples=0', b'unknown_samples=1'),
                         (b'refresh_shortfalls=0', b'refresh_shortfalls=1')]:
            self.assertFalse(check.sdram_status(SDRAM.replace(old, new))[0])

    def test_required_models_only(self):
        config = {'peripherals': ['oled']}
        self.assertTrue(check.peripheral_status(config, OLED + SDRAM.replace(b'init=yes', b'init=no'), UART)[0])
        self.assertFalse(check.peripheral_status({'peripherals': ['sdram', 'oled']}, OLED, UART)[0])
        self.assertFalse(check.peripheral_status({'peripherals': ['typo']}, OLED, UART)[0])

    def test_oled_requires_completed_healthy_report(self):
        self.assertTrue(check.oled_status(OLED, UART)[0])
        self.assertFalse(check.oled_status(b'', UART)[0])
        for old, new in [(b'frames=151', b'frames=0'), (b'frames=151', b'frames=oops'),
                         (b'errors=0', b'errors=1'), (b'unknown_commands=0', b'unknown_commands=1'),
                         (b'display_on=yes', b'display_on=no'), (b'crc32=89ABCDEF', b'crc32=00000000')]:
            self.assertFalse(check.oled_status(OLED.replace(old, new), UART)[0])
        self.assertFalse(check.oled_status(OLED, b'F0000 C89ABCDE')[0])

    def test_window_only_last_exception_allowed(self):
        self.assertTrue(gui_run.oled_pictures([picture(1), picture(2)], UART)[2])
        pictures, bad, ok = gui_run.oled_pictures([picture(1), picture(2, 'FFFFFFFF')], UART)
        self.assertTrue(ok)
        self.assertEqual(bad, [1])
        self.assertEqual([p[1] for p in pictures], [1, 2])
        self.assertFalse(gui_run.oled_pictures([picture(1, 'FFFFFFFF'), picture(2)], UART)[2])
        self.assertFalse(gui_run.oled_pictures([picture(1, 'FFFFFFFF'), picture(2, 'FFFFFFFF')], UART)[2])
        self.assertTrue(gui_run.oled_pictures([], UART)[2])

    def test_window_ignores_only_pictures_without_completed_frames(self):
        startup = [picture(10, '00000000', frames=0), picture(11, '00000000', frames=0)]
        pictures, bad, ok = gui_run.oled_pictures(startup + [picture(12)], UART)
        self.assertTrue(ok)
        self.assertEqual(len(pictures), 3)
        self.assertEqual(bad, [])
        self.assertTrue(gui_run.oled_pictures(startup, b'')[2])
        # Ignoring startup must not relax the exception for the actual last paint.
        _, bad, ok = gui_run.oled_pictures(startup + [picture(12, 'FFFFFFFF')], UART)
        self.assertTrue(ok)
        self.assertEqual(bad, [2])
        self.assertFalse(gui_run.oled_pictures(startup + [picture(12, 'FFFFFFFF'), picture(13)], UART)[2])
        self.assertFalse(gui_run.oled_pictures([picture(12, 'FFFFFFFF')] + startup, UART)[2])
        # Zero is also a possible real CRC: only the frame count decides exemption.
        self.assertFalse(gui_run.oled_pictures([picture(12, '00000000'), picture(13)], UART)[2])

    def test_summary_preserves_hdmi_and_reports_oled_controls(self):
        # Exercise the actual summary, including its existing HDMI CRC matching.
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root / 'target/gallery/fixture').mkdir(parents=True)
            f = root / 'examples/gallery/fixture'
            (f / 'expected').mkdir(parents=True)
            (f / 'expected/uart.txt').write_bytes(UART)
            (f / 'check.json').write_text('{"marker":"RESTART"}')
            trace = root / 'record.tsv'
            trace.write_text('# uart\t0\t0\tRESTART\\r\\nF0000 C89ABCDEF S000 L3 B62\\r\\n'
                             'SW 0100\\r\\nPLAYER\\r\\n\n'
                             '# monitor frame 0 96x64 crc 89ABCDEF\n' +
                             picture(0, '00000000') + '\n' + picture(1) + '\n' +
                             picture(2, 'FFFFFFFF') + '\n')
            old_check, old_gui = check.ROOT, gui_run.ROOT
            try:
                check.ROOT = gui_run.ROOT = root
                text = io.StringIO()
                with contextlib.redirect_stdout(text):
                    self.assertTrue(gui_run.summarize('fixture', trace))
                for wanted in ['1 CRCs equal by frame number', 'SW 0100, PLAYER',
                               '3 pictures painted, frame counts 0..2', '1 before the first completed frame',
                               '1 of 2 painted data CRCs', 'last picture allowed']:
                    self.assertIn(wanted, text.getvalue())
                self.assertEqual((root / 'target/gallery/fixture/gui_oled.txt').read_text(),
                                 '1 1 89ABCDEF printed\n2 2 FFFFFFFF unprinted\n')
            finally:
                check.ROOT, gui_run.ROOT = old_check, old_gui

if __name__ == '__main__':
    unittest.main()
