"""Software checks of record adaptation/startup inference; no hardware access.
Run: python3 -B -m unittest discover -s docs/inventory/LFE5U-85F/hi11_output_delay -p 'test_*.py'
"""
import contextlib
import io
from pathlib import Path
import tempfile
import unittest

from hi11_gen import controls
import hi11_model as model
from hi11s_gen import controls as startup_controls, generate, VALUES, THREE_LANE_IDS


HERE = Path(__file__).resolve().parent


def startup_payload(value, initial_input, initial_output, initial_bottom=None):
    counts = [initial_input, initial_output]
    if initial_bottom is not None:
        counts.append(initial_bottom)
    previous = 0
    samples = []
    for loadn, move, direction in startup_controls():
        if not loadn:
            counts = [value] * len(counts)
        elif previous and not move:
            counts = [min(127, max(0, count + (-1 if direction else 1))) for count in counts]
        previous = move
        flags = [int(count == (0 if direction else 127)) for count in counts]
        bottom = (flags[2] << 6) if initial_bottom is not None else 0
        samples.append(bottom | 0x20 | (loadn << 4) | (move << 3) | (direction << 2) |
                       (flags[0] << 1) | flags[1])
    return ''.join(f'{sample:02X}' for sample in samples)


class Records(unittest.TestCase):
    def test_original_board_records_and_startup_bound(self):
        total = 0
        with contextlib.redirect_stdout(io.StringIO()) as output:
            for path in sorted((HERE / 'records').glob('H11[FBG]-*.txt')):
                total += model.check(path)
        self.assertEqual(total, 90)
        self.assertEqual(output.getvalue().count('initial counter=9..127'), 3)
        self.assertEqual(output.getvalue().count('1991 real samples/path, 0 differ'), 3)
        self.assertEqual(next(i for i, c in enumerate(controls()) if not c[0]), 57)

    def test_measured_two_lane_startup_records(self):
        total = 0
        with contextlib.redirect_stdout(io.StringIO()) as output:
            for path in sorted((HERE / 'records').glob('H11S[01]-*.txt')):
                total += model.check(path)
        self.assertEqual(total, 25)
        self.assertEqual(output.getvalue().count('initial counter=10 '), 6)
        self.assertEqual(output.getvalue().count('initial counter=36 '), 6)
        self.assertEqual(output.getvalue().count('1439 real samples/path, 0 differ'), 6)

    def test_measured_three_lane_boundary_records(self):
        total = 0
        for ident, initial in [('2', 0), ('3', 62), ('4', 0), ('5', 62)]:
            for path in sorted((HERE / 'records').glob(f'H11S{ident}-*.txt')):
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    total += model.check(path)
                lines = output.getvalue().splitlines()
                self.assertIn(f'initial counter={initial} ', lines[1])
                self.assertIn(f'initial counter={initial} ', lines[2])
                self.assertIn('initial counter=0 ', lines[3])
                self.assertIn('0 differ, 0 skipped (3 lanes)', lines[4])
        self.assertEqual(total, 51)

    def test_startup_inference_distinguishes_paths_and_hypotheses(self):
        for ident, value in [('S0', 10), ('S1', 100)]:
            for initial in (0, value, 127):
                with self.subTest(variant=ident, initial=initial):
                    payload = startup_payload(value, 0, initial)
                    _, messages = model.analyse(ident, payload)
                    self.assertIn(f'initial counter={initial} (HI08-compatible)', messages[0])
                    self.assertIn('startup output:', messages[0])
                    self.assertIn('initial counter=0 (HI08-compatible)', messages[1])
                    self.assertIn('side-bank input:', messages[1])
                    self.assertIn('first load sample 609: 1439 real samples/path, 0 differ', messages[2])

    def test_post_load_error_is_still_rejected(self):
        samples = bytearray.fromhex(startup_payload(10, 0, 127))
        samples[900] ^= 1
        with self.assertRaisesRegex(AssertionError, 'NO FIT'):
            model.analyse('S0', samples.hex().upper())

    def test_three_lane_boundaries_report_each_path(self):
        for ident in sorted(THREE_LANE_IDS):
            value = VALUES[ident]
            initial = {1: 0, 63: 62, 64: 0, 127: 62}[value]
            with self.subTest(value=value):
                _, messages = model.analyse('S' + ident, startup_payload(value, initial, initial, 0))
                self.assertIn(f'initial counter={initial} ', messages[0])
                self.assertIn('startup output:', messages[0])
                self.assertIn(f'initial counter={initial} ', messages[1])
                self.assertIn('side-bank input:', messages[1])
                self.assertIn('initial counter=0 ', messages[2])
                self.assertIn('bottom-bank input (FIRE1/R1):', messages[2])
                self.assertIn('0 differ, 0 skipped (3 lanes)', messages[3])

        _, messages = model.analyse('S2', startup_payload(1, 2, 1, 3))
        for message, initial in zip(messages[:3], (1, 2, 3)):
            self.assertIn(f'initial counter={initial} ', message)

    def test_post_load_errors_are_rejected_in_all_three_lanes(self):
        for bit in (0, 1, 6):
            with self.subTest(bit=bit):
                samples = bytearray.fromhex(startup_payload(127, 63, 63, 0))
                samples[900] ^= 1 << bit
                with self.assertRaisesRegex(AssertionError, 'NO FIT'):
                    model.analyse('S5', samples.hex().upper())

    def test_three_lane_frames_use_the_new_prefix_and_old_length(self):
        payload = startup_payload(64, 0, 0, 0)
        checksum, _ = model.analyse('S4', payload)
        frame = f'H11S4{0:08X}{payload}{checksum:02X}\r\n'
        self.assertEqual(len(frame), 4113)
        with tempfile.TemporaryDirectory(prefix='hi11-unit-') as directory:
            path = Path(directory) / 'record.txt'
            path.write_bytes(frame.encode())
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(model.check(path), 1)
            self.assertIn('variant=S4', output.getvalue())
            self.assertIn('bottom-bank input (FIRE1/R1)', output.getvalue())

    def test_startup_frames_and_checksum_sequence_guards(self):
        payload = startup_payload(100, 0, 100)
        checksum, _ = model.analyse('S1', payload)
        def frame(sequence, xor=checksum):
            return f'H11S1{sequence:08X}{payload}{xor:02X}\r\n'
        with tempfile.TemporaryDirectory(prefix='hi11-unit-') as directory:
            path = Path(directory) / 'record.txt'
            path.write_bytes(('partial\r\n' + frame(7) + frame(8) + 'H11S').encode())
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(model.check(path), 2)
            self.assertEqual(len(frame(7)), 4113)
            path.write_bytes(frame(7, checksum ^ 1).encode())
            with self.assertRaisesRegex(AssertionError, 'checksum'):
                model.check(path)
            path.write_bytes((frame(7) + frame(9)).encode())
            with self.assertRaisesRegex(AssertionError, 'sequence'):
                model.check(path)

    def test_generated_sources_reproduce(self):
        for value in VALUES.values():
            self.assertEqual((HERE / f'hi11s{value}.v').read_text(), generate(value))

    def test_series_has_no_personal_home_paths(self):
        for path in HERE.rglob('*'):
            if path.is_file():
                for root in ('Users', 'home'):
                    self.assertNotIn(f'/{root}/'.encode(), path.read_bytes(), str(path))
                self.assertNotIn(b'/private/tmp/' + b'claude-', path.read_bytes(), str(path))


if __name__ == '__main__':
    unittest.main()
