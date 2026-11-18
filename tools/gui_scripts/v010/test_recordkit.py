"""Synthetic-record checks only: these tests never launch a GUI or reserve the screen."""
import argparse
from contextlib import nullcontext, redirect_stdout
import importlib.util
import io
import json
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("gui_record", pathlib.Path(__file__).resolve().parents[2] / "gui_record.py")
gui = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gui)
HEADER = "t\tframe_ms\trunning\trate\tvalues\tmhz\tlag\tlit_missed\tvalue_missed\n"
FRAME = "1\t16\t0\t4\t0\t0\t0/0/0,0/0/0,0/0/0,0/0/0,0/0/0\t0\t0\n"


class Records(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name).resolve()
        self.addCleanup(self.temp.cleanup)
        self.addCleanup(patch.stopall)
        patch.object(gui, "RECORDS", self.root).start()
        patch.object(gui, "screen_lock_path", return_value=self.root / "screen.lock").start()

    def bundle(self, window="", project="", output=""):
        (self.root / "test.tsv").write_text(HEADER + window)
        (self.root / "test.project.tsv").write_text("".join(f"{i}.0\t{line}\n" for i, line in enumerate(project.splitlines())))
        (self.root / "test.out").write_text(output + "\n# window exit 0\n")
        entry = {"label": "test"}
        if project:
            entry["project"] = "fixture"
        return gui.record_bundle(entry)

    def verdict(self, expected, rule, **records):
        ok, measured = gui.judge(self.bundle(**records), rule)
        self.assertEqual(ok, expected, measured)
        self.assertTrue(measured)

    def test_note_counts_pass_and_fail(self):
        rule = {"check": "note", "pattern": r"^# header\b", "min": 2}
        self.verdict(True, rule, window="# header open\n# header close\n")
        self.verdict(False, rule, window="# header open\n")

    def test_numeric_bounds_pass_and_fail(self):
        rule = {"check": "numeric", "source": "project", "pattern": r"frame_p95_ms=(?P<value>\S+)", "bound": 16.7}
        self.verdict(True, rule, project="# editor frames frame_p95_ms=16.7")
        for value in ["16.71", "nan", "inf"]:
            self.verdict(False, rule, project=f"# editor frames frame_p95_ms={value}")
        self.verdict(False, rule, project="# editor frames missing")

    def test_privacy_both_records_and_home_boundaries(self):
        rule = {"check": "no_home"}
        self.verdict(True, rule, window="# design ~/work/top.config\n", project="open rtl/top.v:1 language=verilog")
        self.verdict(False, {"check": "no_home", "source": "window"}, project="open /Users/visitor/private/top.v")
        for path in ["/Users/visitor/work/top.v", "/home/visitor/top.v"]:
            self.verdict(False, rule, project=f"load failed: {path}")
            self.verdict(False, rule, window=f"# design {path}\n")

    def test_errors_in_record_and_console(self):
        rule = {"check": "no_error"}
        self.verdict(True, rule, output="load: finished\n")
        missing_exit = self.bundle()
        missing_exit["output"] = ""
        self.assertFalse(gui.judge(missing_exit, rule)[0])
        for event in ["chip error bad", "# script error bad", "build failed 1", "load failed bad"]:
            self.verdict(False, rule, project=event)
        for event in ["thread 'main' panicked at app.rs", "live: failed", "# timeout after 300s", "# window exit 9"]:
            self.verdict(False, rule, output=event)

    @staticmethod
    def layouts():
        return ("# script 0 view 0.85 0.5 19\n"
                "# map-layout report=off central=608,57,1492,794 shelf=- map=608,57,1492,794 resized=0 before_centre=300,200 before_cell=40 centre=300,200 cell=40\n"
                "# layout window x 0..1500 y 0..950 board x 0..600 y 0..700 die map x 608..1492 y 57..794\n"
                "# script 2 critical on\n"
                "# map-layout report=on central=608,57,1492,794 shelf=1152,57,1492,794 map=608,57,1144,794 resized=1 before_centre=300,200 before_cell=40 centre=300,200 cell=40\n"
                "# layout window x 0..1500 y 0..950 board x 0..600 y 0..700 die map x 608..1144 y 57..794\n"
                "# script 4 critical off\n"
                "# map-layout report=off central=608,57,1492,794 shelf=- map=608,57,1492,794 resized=1 before_centre=300,200 before_cell=40 centre=300,200 cell=40\n")

    def test_layout_full_width_gap_camera_and_missing_evidence(self):
        rule = {"check": "layout"}
        self.verdict(True, rule, window=self.layouts())
        for broken in [self.layouts().replace("shelf=1152", "shelf=1155"),
                       self.layouts().replace("map=608,57,1492,794", "map=608,57,1400,794"),
                       self.layouts().replace("centre=300,200 cell=40\n# layout window x 0..1500 y 0..950 board x 0..600 y 0..700 die map x 608..1144", "centre=301,200 cell=40\n# layout window x 0..1500 y 0..950 board x 0..600 y 0..700 die map x 608..1144"),
                       "# layout die map x 608..1144 y 57..794\n"]:
            self.verdict(False, rule, window=broken)

    def test_toolbar_sizes_groups_fit_and_missing_evidence(self):
        rule = {"check": "toolbar"}
        notes = ("# toolbar size=1500,950 row=8,8,1492,38 navigation=8,8,200,38 build=- transport=220,8,430,38 time=450,8,800,38 view=1100,8,1492,38 compact=0\n"
                 "# toolbar size=1100,800 row=8,8,1092,38 navigation=8,8,150,38 build=170,8,290,38 transport=310,8,500,38 time=520,8,780,38 view=940,8,1092,38 compact=1\n")
        self.verdict(True, rule, window=notes)
        for broken in [notes.replace("transport=310", "transport=280"),
                       notes.replace("view=940,8,1092", "view=940,8,1100"),
                       notes.replace("navigation=8,8,150,38", "navigation=8,8,150,50"),
                       notes.replace("build=170,8,290,38", "build=nan,8,290,38"),
                       notes.replace("view=940,8,1092,38", "view=-"),
                       notes.replace("compact=1", "compact=0"),
                       notes + notes.splitlines()[-1] + "\n",
                       notes.splitlines()[0], "# toolbar size=1100,800\n", ""]:
            self.verdict(False, rule, window=broken)

    @staticmethod
    def values():
        return ("# script 0 pause\n# value top.v:6:13 count = count[7:0] = 0x00 (0) @ 1\n"
                "# script 8 run\n# value top.v:6:13 count = count[7:0] = 0x01 (1) @ 2\n")

    def test_live_numbers_cycles_and_inline_are_required(self):
        rule = {"check": "live", "names": ["count"], "inline": True}
        inline = "# editor frames phase=1 file=top.v theme=dark inline=on kind=idle n=2 inline_values=2"
        self.verdict(True, rule, window=self.values(), project=inline)
        self.verdict(False, rule, window=self.values(), project=inline.replace("inline_values=2", "inline_values=0"))
        self.verdict(False, rule, window=self.values().replace("0x01 (1) @ 2", "0x00 (0) @ 1"), project=inline)
        self.verdict(False, rule, window=self.values().replace("0x01 (1)", "?"), project=inline)
        self.verdict(False, {"check": "live", "names": ["missing"]}, window=self.values())
        self.verdict(False, rule, window=self.values().replace("# script 8 run", "# script 8 pause"), project=inline)

    @staticmethod
    def syntax():
        lines = [f"open sample.{ext}:1 language={language}" for ext, language in
                 [("v", "verilog"), ("sv", "systemverilog"), ("vhd", "vhdl"), ("lpf", "lpf"), ("toml", "toml"), ("txt", "plain")]]
        lines += ["# editor frames phase=0 file=picorv32_plain.txt theme=dark inline=off kind=idle n=12 frame_p50_ms=1",
                  "# editor frames phase=1 file=picorv32.v theme=dark inline=off kind=idle n=12 frame_p50_ms=1.1"]
        for theme in ["dark", "light"]:
            for phase in range(3):
                lines.append(f"# editor frames phase={phase} file=picorv32.v theme={theme} inline=off kind=typing n=20 frame_p95_ms=12")
        return "\n".join(lines)

    def test_syntax_languages_budget_idle_baseline_and_themes(self):
        rule = {"check": "syntax", "languages": ["verilog", "systemverilog", "vhdl", "lpf", "toml", "plain"]}
        self.verdict(True, rule, project=self.syntax())
        for broken in [self.syntax().replace("frame_p95_ms=12", "frame_p95_ms=17"),
                       self.syntax().replace("frame_p50_ms=1.1", "frame_p50_ms=3"),
                       self.syntax().replace("language=vhdl", "language=plain"),
                       self.syntax().replace("theme=light", "theme=dark"),
                       self.syntax().replace("file=picorv32_plain.txt", "file=missing.txt")]:
            self.verdict(False, rule, project=broken)

    def test_project_summary_does_not_treat_events_as_a_frame_header(self):
        self.bundle(project="open top.v:1 language=verilog\n# editor frames phase=1 kind=idle n=3")
        with redirect_stdout(io.StringIO()) as out:
            gui.summarize([self.root / "test.project.tsv", self.root / "test.tsv"])
        self.assertIn("language=verilog", out.getvalue())
        self.assertIn("editor frames", out.getvalue())

    def test_project_stalls_only_after_ready_while_chip_is_visible(self):
        trace, output, project = [self.root / name for name in ["trace", "out", "project"]]
        trace.write_text(HEADER + FRAME)
        output.write_text("")
        project.write_text("0\tproject fixture .\n1\tbuild start force=false\n")
        watch = gui.StallWatch(trace, output, 0, project)
        self.assertFalse(watch.poll(10))
        self.assertFalse(watch.poll(20))
        project.write_text(project.read_text() + "2\tbuild ok 1\n3\temulate build/top.config\n4\tview chip\n5\tchip ready 1\n")
        self.assertFalse(watch.poll(21))
        self.assertTrue(watch.poll(25))
        project.write_text(project.read_text() + "6\tview project\n")
        self.assertFalse(watch.poll(40))

    def test_dry_run_neither_reserves_screen_nor_launches(self):
        config = self.root / "batch.json"
        config.write_text(json.dumps({"records": [{"label": "dry", "project": "p", "project_script": "p.script"}]}))
        args = argparse.Namespace(file=config, binary=self.root / "gui", who=None, check=False, dry_run=True)
        with patch.object(gui, "prepare_run", return_value=(["never-executed"], {}, None, None, None)), \
             patch.object(gui, "console_unlocked") as console, patch.object(gui, "screen_lock") as lock, \
             patch.object(gui, "record_run") as run, redirect_stdout(io.StringIO()):
            self.assertTrue(gui.record_batch(args))
        console.assert_not_called()
        lock.assert_not_called()
        run.assert_not_called()

    def test_committed_typing_coordinates_and_plain_baseline_match_sources(self):
        kit = pathlib.Path(__file__).resolve().parent
        project = gui.ROOT / "examples/ulx3s/picorv32_console"
        source = project / "picorv32.v"
        self.assertFalse((kit / "hdl/picorv32_plain.txt").exists())
        config = gui.batch_entries(argparse.Namespace(file=kit / "batch.json", only=["v010_syntax"]))[0]
        baseline = self.root / "out/records/v010/picorv32_plain.txt"
        config["prepare"] = {"command": ["cp", str(source), str(baseline)], "output": str(baseline)}
        with redirect_stdout(io.StringIO()):
            self.assertEqual(gui.prepare_entry(config)[-1], "PASS")
        self.assertEqual(source.read_bytes(), baseline.read_bytes())
        self.assertNotEqual(source.resolve(), baseline.resolve())
        buffers = {"picorv32.v": source.read_text().split("\n")}
        for raw in (kit / "syntax.project.script").read_text().splitlines():
            line = raw.split("#", 1)[0].strip()
            match = gui.re.match(r"[\d.]+ (type|edit) (\S+) (\d+) (.*)", line)
            if not match:
                continue
            kind, file, row, text = match.groups()
            row = int(row) - 1
            self.assertLess(row, len(buffers[file]))
            if kind == "type":
                column, text = text.split(" ", 1)
                column = int(column) - 1
                self.assertLessEqual(column, len(buffers[file][row]))
                buffers[file][row] = buffers[file][row][:column] + text + buffers[file][row][column:]
            else:
                buffers[file][row] = text

    def test_header_entries_and_clean_blinky_script_match_the_supplied_format(self):
        kit = pathlib.Path(__file__).resolve().parent
        entries = gui.batch_entries(self.batch_args(kit / "batch.json", only=["header_pulls", "header_external"]))
        self.assertEqual([e["label"] for e in entries], ["header_pulls", "header_external"])
        build = ["examples/ulx3s/build.sh", "examples/ulx3s/header_probe/driven", "target/reference/header_probe_driven"]
        for entry, setting in zip(entries, ["", "GN0=0"]):
            self.assertEqual(entry["prepare"]["command"], build)
            self.assertEqual(entry["env"], {"ECP5_HEADER": setting})
            self.assertEqual(entry["unset_env"], ["ECP5_GUI_NO_BOARD"])
            self.assertEqual(entry["script"], "tools/window_record/header_probe.script")
            want = ["python3", "tools/window_record/header_summary.py"]
            if setting:
                want += ["--external", "GN0=0"]
            self.assertEqual(entry["judge"], want + ["{record}"])
        source = (gui.ROOT / "examples/ulx3s/blinky_cycle/top.v").read_text().splitlines()
        hovers = []
        for script in kit.glob("*.project.script"):
            commands = [raw.split("#", 1)[0].strip().split() for raw in script.read_text().splitlines()]
            self.assertTrue(all(len(words) < 2 or words[1] != "load" for words in commands))
            if script.name == "live_blinky.project.script":
                self.assertTrue(all(len(words) < 2 or words[1] != "edit" for words in commands))
                for words in commands:
                    if len(words) > 1 and words[1] == "hover":
                        row, column = int(words[3]), int(words[4])
                        token = "led" if row == 8 else "count"
                        self.assertTrue(source[row-1][column-1:].startswith(token))
                        hovers.append((row, token))
        self.assertEqual(hovers, [(6, "count"), (7, "count"), (8, "led")] * 2)

    def test_unavailable_live_reason_is_exact_and_numeric_still_requires_max(self):
        reason = "`live` has no net on the chip: optimised away, a constant, or inside a cell"
        rule = {"check": "live", "kind": "unavailable", "names": ["live"], "reason": reason}
        self.verdict(True, rule, window=f"# value top.v:54:20 live -> {reason}\n")
        self.verdict(True, rule, window=f"# value top.v:54:20 live -> {reason}; built from an older top.v: locations may be off\n")
        self.verdict(False, rule, window="# value top.v:54:20 live -> Design is loading\n")
        self.verdict(False, rule, window=f"# value top.v:54:20 other -> {reason}\n")
        numeric = {"check": "live", "names": ["count"], "rate": "max"}
        self.verdict(True, numeric, window=self.values().replace("# script 8 run", "# script 0 rate max\n# script 8 run"))
        self.verdict(False, numeric, window=self.values().replace("# script 8 run", "# script 0 rate real\n# script 8 run"))

    def test_layout_uses_camera_pair_in_one_note_after_unrecorded_movement(self):
        notes = self.layouts().replace(
            "resized=1 before_centre=300,200 before_cell=40 centre=300,200",
            "resized=1 before_centre=600,400 before_cell=40 centre=600,400")
        self.verdict(True, {"check": "layout"}, window=notes)
        self.verdict(False, {"check": "layout"}, window=notes.replace(
            "before_centre=600,400 before_cell=40 centre=600,400",
            "before_centre=601,400 before_cell=40 centre=600,400"))

    def batch_file(self, entries):
        config = self.root / "temporary.json"
        config.write_text(json.dumps({"records": entries}))
        return config

    def batch_args(self, file, **extra):
        values = dict(file=file, binary=self.root / "gui", who="fixture", check=False, dry_run=False, only=[])
        values.update(extra)
        return argparse.Namespace(**values)

    def test_temporary_batch_dry_run_lists_prepare_lock_env_records_and_judges_in_order(self):
        output = self.root / "prepared/reference.bit"
        entries = [{"label": name, "design": "reference", "script": "pending-merge.script",
                    "prepare": {"command": [sys.executable, "-c", "raise RuntimeError('must not execute')"], "output": str(output)},
                    "env": {"ECP5_HEADER": setting}, "unset_env": ["ECP5_GUI_NO_BOARD"],
                    "judge": [sys.executable, "pending-merge-judge.py", "{record}"]}
                   for name, setting in [("pulls", ""), ("external", "GN0=0")]]
        args = self.batch_args(self.batch_file(entries), dry_run=True)
        with patch.object(gui, "console_unlocked") as console, patch.object(gui, "screen_lock") as lock, \
             patch.object(gui, "record_run") as run, patch.object(gui.subprocess, "run", side_effect=AssertionError("no commands in dry run")), \
             redirect_stdout(io.StringIO()) as out:
            self.assertTrue(gui.record_batch(args))
        text = out.getvalue()
        markers = ["PREPARE pulls:", "SKIP prepare external:", "LOCK ", "RECORD pulls:", "JUDGE pulls:", "RECORD external:", "JUDGE external:", "UNLOCK "]
        indices = [text.index(marker) for marker in markers]
        self.assertEqual(indices, sorted(indices))
        self.assertIn("env -u ECP5_GUI_NO_BOARD ECP5_HEADER=", text)
        self.assertIn("ECP5_HEADER=GN0=0", text)
        self.assertIn(str(self.root / "pulls.tsv"), text)
        self.assertNotIn("{record}", text)
        self.assertFalse(output.parent.exists())
        console.assert_not_called()
        lock.assert_not_called()
        run.assert_not_called()

    def test_only_selects_requested_names_in_batch_order_and_rejects_unknown(self):
        entries = [{"label": name, "design": "reference", "script": "missing.script"} for name in ["a", "b", "c"]]
        config = self.batch_file(entries)
        selected = gui.batch_entries(self.batch_args(config, only=["c", "a"]))
        self.assertEqual([e["label"] for e in selected], ["a", "c"])
        with self.assertRaises(ValueError):
            gui.batch_entries(self.batch_args(config, only=["absent"]))
        with redirect_stdout(io.StringIO()) as out:
            gui.record_batch(self.batch_args(config, only=["b"], dry_run=True))
        self.assertIn("RECORD b:", out.getvalue())
        self.assertNotIn("RECORD a:", out.getvalue())
        self.assertNotIn("RECORD c:", out.getvalue())

    def test_prepare_runs_once_before_lock_and_existing_output_is_skipped(self):
        output, count = self.root / "prepared/item", self.root / "count"
        command = [sys.executable, "-c", "import pathlib,sys; pathlib.Path(sys.argv[1]).write_text('ready'); p=pathlib.Path(sys.argv[2]); p.write_text((p.read_text() if p.exists() else '')+'x')", str(output), str(count)]
        entries = [{"label": name, "design": "reference", "script": "window.script",
                    "prepare": {"command": command, "output": str(output)}} for name in ["a", "b"]]
        config = self.batch_file(entries)
        bundle = self.bundle()
        def locked(*args):
            self.assertTrue(output.exists(), "prepare finishes before screen time")
            return nullcontext()
        with patch.object(gui, "prepare_run", return_value=(["not-launched"], {}, None, None, None)), \
             patch.object(gui, "console_unlocked"), patch.object(gui, "screen_lock", side_effect=locked) as lock, \
             patch.object(gui, "record_run"), patch.object(gui, "record_bundle", return_value=bundle), \
             patch.object(gui.signal, "signal"), redirect_stdout(io.StringIO()) as log:
            self.assertTrue(gui.record_batch(self.batch_args(config)))
        self.assertEqual(count.read_text(), "x")
        self.assertIn("SKIP prepare b: output already exists", log.getvalue())
        lock.assert_called_once()
        with patch.object(gui.subprocess, "run", side_effect=AssertionError("must skip")), redirect_stdout(io.StringIO()) as log:
            self.assertEqual(gui.prepare_entry(entries[0])[-1], "PASS")
        self.assertIn("already exists", log.getvalue())

    def test_prepare_failure_does_not_take_lock_or_start_a_record(self):
        output = self.root / "prepared/missing"
        entry = {"label": "failure", "design": "reference", "script": "window.script",
                 "prepare": {"command": [sys.executable, "-c", "print('no output')"], "output": str(output)}}
        with patch.object(gui, "screen_lock") as lock, patch.object(gui, "record_run") as run, redirect_stdout(io.StringIO()) as log:
            self.assertFalse(gui.record_batch(self.batch_args(self.batch_file([entry]))))
        self.assertIn("exists=False", log.getvalue())
        lock.assert_not_called()
        run.assert_not_called()

    def test_external_judge_substitution_environment_stdout_and_exit_status(self):
        self.bundle()
        for printed, code in [("PASS", 0), ("PASS", 2), ("FAIL", 0)]:
            entry = {"label": "test", "env": {"ECP5_HEADER": "GN0=0"}, "unset_env": ["ECP5_GUI_NO_BOARD"],
                     "judge": [sys.executable, "-c",
                               "import os,pathlib,sys; assert 'ECP5_GUI_NO_BOARD' not in os.environ; assert os.environ['ECP5_HEADER']=='GN0=0'; assert pathlib.Path(sys.argv[1]).is_file(); print(sys.argv[2]+' fixture: '+sys.argv[1]); sys.exit(int(sys.argv[3]))",
                               "{record}", printed, str(code)]}
            with patch.dict(gui.os.environ, {"ECP5_GUI_NO_BOARD": "1"}):
                rows = gui.external_verdicts(entry)
            self.assertEqual(all(row[-1] == "PASS" for row in rows), printed == "PASS" and code == 0)
            self.assertIn(str(self.root / "test.tsv"), rows[0][2])
            self.assertEqual(rows[-1][2], f"exit={code}")
        entry["env"]["ECP5_HEADER"] = ""
        env = gui.entry_environment(entry)
        self.assertEqual(env["ECP5_HEADER"], "")
        self.assertNotIn("ECP5_GUI_NO_BOARD", env)
        entry["judge"] = [sys.executable, "-c", "import sys; print('  FAIL stderr check', file=sys.stderr)"]
        rows = gui.external_verdicts(entry)
        self.assertEqual(rows[0][1], "judge stderr")
        self.assertEqual(rows[0][-1], "FAIL")

    def test_external_failure_is_in_table_and_stops_before_the_next_record(self):
        entries = [{"label": name, "design": "reference", "script": "window.script",
                    "judge": [sys.executable, "-c", "print('FAIL fixture: measured bad'); raise SystemExit(3)", "{record}"]}
                   for name in ["a", "b"]]
        config = self.batch_file(entries)
        with patch.object(gui, "record_bundle", return_value=self.bundle()) as read, redirect_stdout(io.StringIO()) as log:
            self.assertFalse(gui.record_batch(self.batch_args(config, check=True)))
        self.assertEqual(read.call_count, 1)
        self.assertIn("a | judge | FAIL fixture: measured bad | FAIL", log.getvalue())
        self.assertIn("a | judge exit | exit=3 | FAIL", log.getvalue())

    def test_batch_stops_at_first_failed_record_and_prints_one_table(self):
        config = self.root / "batch.json"
        config.write_text(json.dumps({"records": [
            {"label": name, "design": "blinky_cycle", "script": "placeholder", "criteria": [
                {"check": "note", "pattern": "missing", "min": 1}]} for name in ["first", "second"]]}))
        bundle = self.bundle()
        args = argparse.Namespace(file=config, binary=self.root / "gui", who=None, check=True, dry_run=False)
        with patch.object(gui, "record_bundle", return_value=bundle) as read, redirect_stdout(io.StringIO()) as out:
            self.assertFalse(gui.record_batch(args))
        self.assertEqual(read.call_count, 1)
        self.assertEqual(out.getvalue().count("record | criterion"), 1)
        self.assertIn("FAIL", out.getvalue())

    def test_project_launch_arguments_and_traces_are_prepared_without_launching(self):
        patch.object(gui, "ROOT", self.root).start()
        for name in ["target/release/ecp5_gui", "target/trellis_image_reader", "out/ulx3s_3d/ulx3s_v3.0.8.glb", "project/ecp5.toml", "window.script", "project.script"]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("")
        (self.root / "target/release/ecp5_gui").chmod(0o755)
        entry = {"label": "test", "project": "project", "script": "window.script", "project_script": "project.script"}
        args = argparse.Namespace(binary=self.root / "target/release/ecp5_gui", who=None)
        command, env, trace, output, project = gui.prepare_run(gui.entry_args(entry, args))
        self.assertEqual(command[-2:], ["--project", str(self.root / "project")])
        self.assertEqual(env["ECP5_PROJECT_TRACE"], str(project))
        self.assertEqual(env["ECP5_PROJECT_SCRIPT"], str(self.root / "project.script"))
        del entry["script"]
        command, env, *_ = gui.prepare_run(gui.entry_args(entry, args))
        self.assertNotIn("ECP5_GUI_SCRIPT", env)
        self.assertIn("ECP5_PROJECT_SCRIPT", env)

    def test_batch_uses_one_lock_for_all_entries(self):
        config = self.root / "batch.json"
        config.write_text(json.dumps({"records": [{"label": name, "design": "blinky_cycle", "script": "placeholder"} for name in ["a", "b"]]}))
        args = argparse.Namespace(file=config, binary=self.root / "gui", who="tests", check=False, dry_run=False)
        with patch.object(gui, "prepare_run", return_value=(["never-executed"], {}, None, None, None)), \
             patch.object(gui, "console_unlocked"), patch.object(gui, "screen_lock", return_value=nullcontext()) as lock, \
             patch.object(gui, "record_run") as run, patch.object(gui, "record_bundle", return_value=self.bundle()), \
             patch.object(gui.signal, "signal"), \
             redirect_stdout(io.StringIO()):
            self.assertTrue(gui.record_batch(args))
        lock.assert_called_once()
        self.assertEqual(run.call_count, 2)
        self.assertTrue(all(call.kwargs["lock_held"] for call in run.call_args_list))


if __name__ == "__main__":
    unittest.main()
