"""Synthetic screen records only: these tests never start the GUI."""

import contextlib
import io
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import header_summary as summary


def pins(external=None, startup=False):
    entries = []
    for index in range(28):
        for kind in ("GN", "GP"):
            name = f"{kind}{index}"
            level = external if name == "GN0" and external is not None else str(int(kind == "GN"))
            ext = external if name == "GN0" and external is not None else "z"
            measure = "--Hz/--%"
            if kind == "GP" and index < 4:
                hz, duty = ((1000, 25), (2000, 50), (5000, 75), (10000, 10))[index]
                level = "X" if startup else "0"
                measure = "--Hz/--%" if startup else f"{hz:.3f}Hz/{duty:.3f}%"
            entries.append(f"{name}={level} {measure} ext={ext}")
    return " ".join(entries)


def uart(second, external=None):
    counts = " ".join(f"{n * second:08X}" for n in (1000, 2000, 5000, 10000))
    word, echo = ("55555555555554", "0") if external == "0" else ("55555555555555", "1")
    return (f"HEADER {counts} P 000003E8/005F5E10 000007D0/00BEBC20 "
            f"00001388/011E1A30 00002710/002625A0 {word} E{echo}\r\n")


def escaped(text):
    return text.replace("\\", "\\\\").replace("\r", "\\r").replace("\n", "\\n").replace("\t", "\\t")


def board(time, external=None):
    lit = "00000000" if external == "0" else "10000000"
    dark = "11111111" if external == "0" else "01111111"
    return (f"# board\t{time}\tdrawing 1\tLEDs D0..D7 to show lit {lit}"
            f"\tdrawn lit {lit}\tdrawn dark {dark}\tok\n")


def record(external=None):
    first = uart(1, external)
    return (
        "t\trunning\trate\tcycle\n"
        "# script\t0.000\tboard top\n"
        "# script\t0.000\trate real\n"
        "# script\t0.000\trun\n"
        f"# header {pins(external, startup=True)}\n"
        "0.100\t1\t25000000\t10000\n"
        f"# header {pins(external)}\n"
        "3.000\t1\t25000000\t12000000\n"
        + board("3.100", external)
        + f"# uart\t6.000\t25100000\t{escaped(first[:70])}\n"
        + f"# uart\t8.000\t25300000\t{escaped(first[70:])}\n"
        + "9.000\t1\t25000000\t30000000\n"
        "# script\t12.000\trate max\n"
        "14.500\t1\tmax\t40000000\n"
        + board("14.600", external)
        + f"# uart\t20.000\t50300000\t{escaped(uart(2, external))}\n"
        + "21.000\t1\tmax\t60000000\n"
        "# uart\t22.000\t60000001\tHEADER 00000\n"
        "# script\t29.000\tpause\n"
        "29.100\t0\tmax\t70000000\n"
        "# script\t30.000\tquit\n"
        "30.000\t0\tmax\t70000000\n"
    )


class HeaderSummaryTests(unittest.TestCase):
    def read(self, text):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "record.tsv"
            path.write_text(text, encoding="utf-8")
            return summary.read_record(path)

    def checks(self, text, external=None):
        return summary.judge(self.read(text), external)

    def assert_pass(self, checks):
        self.assertTrue(all(passed for _, passed, _ in checks), checks)

    def assert_failed(self, checks, name):
        self.assertTrue(any(name in label and not passed for label, passed, _ in checks), checks)

    def test_pulls_and_carried_header_notes_pass_in_both_phases(self):
        checks = self.checks(record())
        self.assert_pass(checks)
        self.assertEqual(sum(label.endswith("52 inputs") for label, _, _ in checks), 2)

    def test_external_zero_changes_panel_uart_word_and_led(self):
        self.assert_pass(self.checks(record("0"), "GN0=0"))

    def test_external_one_requires_an_explicit_external_setting(self):
        self.assert_pass(self.checks(record("1"), "GN0=1"))
        self.assert_failed(self.checks(record(), "GN0=1"), "52 inputs")

    def test_constant_correct_led_drawing_carries_into_both_rate_phases(self):
        for external in (None, "0"):
            with self.subTest(external=external):
                data = self.read(record(external))
                data.boards = [(0.0, data.boards[0][1])]
                checks = summary.judge(data, "GN0=0" if external else None)
                self.assert_pass(checks)
                led = [(label, passed, detail) for label, passed, detail in checks
                       if label.endswith("LED0 echo")]
                self.assertEqual(len(led), 2)
                for _, _, detail in led:
                    self.assertIn("1 visible correct drawings, 0 wrong", detail)

    def test_constant_wrong_led_drawing_is_wrong_not_missing_evidence(self):
        data = self.read(record())  # Header GN0=1 and UART E1.
        data.boards = [(0.0, self.read(record("0")).boards[0][1])]
        checks = summary.judge(data)
        self.assertEqual(
            [(label, passed, detail) for label, passed, detail in checks
             if label.endswith("LED0 echo")],
            [(f"{rate} LED0 echo", False, "0 visible correct drawings, 1 wrong; GN0=1")
             for rate in ("real", "max")])
        self.assert_pass([check for check in checks if not check[0].endswith("LED0 echo")])

    def test_later_wrong_drawing_is_checked_alongside_carried_correct_drawing(self):
        data = self.read(record())
        data.boards = [(0.0, data.boards[0][1]),
                       (15.0, self.read(record("0")).boards[0][1])]
        checks = summary.judge(data)
        led = {label: (passed, detail) for label, passed, detail in checks
               if label.endswith("LED0 echo")}
        self.assertEqual(led["real LED0 echo"],
                         (True, "1 visible correct drawings, 0 wrong; GN0=1"))
        self.assertEqual(led["max LED0 echo"],
                         (False, "1 visible correct drawings, 1 wrong; GN0=1"))

    def test_latest_drawing_at_or_before_settled_start_is_counted_once(self):
        for time in (0.0, 1.0, summary.SETTLE_SECONDS):
            with self.subTest(time=time):
                data = self.read(record())
                data.boards = [(0.0, self.read(record("0")).boards[0][1]),
                               (time, data.boards[0][1])]
                checks = summary.judge(data)
                self.assert_pass(checks)
                for label, _, detail in checks:
                    if label.endswith("LED0 echo"):
                        self.assertIn("1 visible correct drawings, 0 wrong", detail)

    def test_frequency_and_duty_outside_tolerance_fail(self):
        text = record().replace("5000.000Hz/75.000%", "4900.000Hz/77.000%")
        self.assert_failed(self.checks(text), "GP2")

    def test_tolerance_allows_small_window_endpoint_errors(self):
        text = record().replace("5000.000Hz/75.000%", "5025.000Hz/75.500%")
        self.assert_pass(self.checks(text))

    def test_wrong_pull_or_external_metadata_fails(self):
        text = record().replace("GN7=1 --Hz/--% ext=z", "GN7=0 --Hz/--% ext=z")
        self.assert_failed(self.checks(text), "52 inputs")
        text = record("0").replace("GN0=0 --Hz/--% ext=0", "GN0=0 --Hz/--% ext=z")
        self.assert_failed(self.checks(text, "GN0=0"), "52 inputs")

    def test_missing_and_duplicate_pins_are_rejected(self):
        for replacement in ("", "GN26=1 --Hz/--% ext=z "):
            with self.subTest(replacement=replacement), self.assertRaises(ValueError):
                self.read(record().replace("GN27=1 --Hz/--% ext=z ", replacement))

    def test_missing_board_and_wrong_gpu_led_fail(self):
        text = "\n".join(line for line in record().splitlines() if not line.startswith("# board")) + "\n"
        checks = self.checks(text)
        self.assert_failed(checks, "3D board")
        self.assert_failed(checks, "LED0 echo")
        for label, _, detail in checks:
            if label.endswith("LED0 echo"):
                self.assertIn("no # board drawing", detail)
        text = record("0").replace("drawn dark 11111111", "drawn dark 01111111")
        self.assert_failed(self.checks(text, "GN0=0"), "LED0 echo")

    def test_uart_fragments_are_joined_but_unterminated_tail_is_ignored(self):
        data = self.read(record())
        self.assertEqual(data.uart, (uart(1) + uart(2) + "HEADER 00000").encode())
        checks = summary.judge(data)
        self.assertEqual(sum(label.endswith("format") for label, _, _ in checks), 2)
        self.assert_pass(checks)

    def test_no_complete_uart_line_fails(self):
        data = self.read(record())
        data.uart = uart(1).encode()[:-1]
        self.assert_failed(summary.judge(data), "UART complete lines")

    def test_corrupt_uart_counter_pad_count_word_and_echo_fail(self):
        for before, after, check in (
            ("000003E8 000007D0", "000003E9 000007D0", "counters"),
            ("000003E8/005F5E10", "000003E8/005F5E11", "pad counts"),
            ("55555555555555 E1", "55555555555554 E0", "inputs/echo"),
        ):
            with self.subTest(check=check):
                data = self.read(record())
                data.uart = data.uart.replace(before.encode(), after.encode())
                self.assert_failed(summary.judge(data), check)

    def test_only_one_rate_or_no_settled_frames_fails(self):
        self.assert_failed(self.checks(record().replace("rate max", "rate real")), "rate phases")
        self.assert_failed(self.checks(record().replace("14.500\t1", "12.500\t1").replace("21.000\t1", "13.000\t1")), "max running")

    def test_invalid_escape_nonfinite_time_and_truncated_record_fail(self):
        for text in (record().replace("HEADER 00000\n", "\\q\n"),
                     record().replace("3.000\t1", "nan\t1"), record().rstrip("\n")):
            with self.subTest(text=text[-80:]), self.assertRaises(ValueError):
                self.read(text)
        self.assertEqual(summary.uart_bytes(r"\x48\t\\\r\n"), b"H\t\\\r\n")

    def test_script_uses_only_the_existing_timed_commands(self):
        script = Path(__file__).with_name("header_probe.script").read_text(encoding="utf-8")
        commands = [line.split("#", 1)[0].split() for line in script.splitlines()]
        commands = [words for words in commands if words]
        self.assertEqual(commands, [
            ["0", "board", "top"], ["0", "rate", "real"], ["0", "run"],
            ["12", "rate", "max"], ["29", "pause"], ["30", "quit"],
        ])

    def test_cli_help_and_success_and_failure_exit_codes(self):
        script = Path(__file__).with_name("header_summary.py")
        help_run = subprocess.run([sys.executable, str(script), "--help"], capture_output=True, text=True)
        self.assertEqual(help_run.returncode, 0)
        self.assertIn("--external", help_run.stdout)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "record.tsv"
            path.write_text(record("0"), encoding="utf-8")
            for external, expected in (("GN0=0", 0), ("GN0=1", 1)):
                with self.subTest(external=external), contextlib.redirect_stdout(io.StringIO()) as out:
                    self.assertEqual(summary.main([str(path), "--external", external]), expected)
                    self.assertIn("PASS" if expected == 0 else "FAIL", out.getvalue())


if __name__ == "__main__":
    unittest.main()
