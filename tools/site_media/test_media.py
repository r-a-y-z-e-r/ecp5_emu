"""Offline checks for validation, evidence gates and capture guards."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import media
import encode


class KitTests(unittest.TestCase):
    def test_new_project_gate_uses_immediate_project_event_without_window_flush(self):
        entries, _, _ = media.load(type('Args', (), {'shot': 'all'})())
        entry = next(e for e in entries if e['label'] == 'new-project')
        project = '0.505\tnew-project dialog "My counter" resources=clock,leds,buttons,switches,uart\n'
        self.assertEqual(entry['gate']['kind'], 'project')
        self.assertTrue(media.gate_ready(entry, '', project, {}))
        self.assertFalse(media.gate_ready(entry, project, '', {}))
        self.assertFalse(media.gate_ready(entry, '', '0.4\tview project\n', {}))

    def test_other_window_gates_have_live_chip_frames_to_flush_notes(self):
        entries, _, _ = media.load(type('Args', (), {'shot': 'all'})())
        for entry in entries:
            if entry['gate']['kind'] not in ('script', 'oled'):
                continue
            with self.subTest(shot=entry['label']):
                self.assertIn('project', entry)  # a loaded Chip, not a static start/dialog
                commands = media.commands(media.ROOT / entry['script'])
                self.assertIn((0.0, 'run'), commands)
                if entry['gate']['kind'] == 'script':
                    trace = ''.join(f'# script\t{stamp:.3f}\t{command}\n' for stamp, command in commands)
                    self.assertTrue(media.gate_ready(entry, trace, '', {}))

    def test_all_shots_fit_display_and_full_window_stills_preserve_aspect(self):
        entries, _, _ = media.load(type('Args', (), {'shot': 'all'})())
        for entry in entries:
            with self.subTest(shot=entry['label']):
                media.check_geometry(entry)
                if entry.get('still') and not entry.get('crop'):
                    self.assertTrue(encode.aspect_matches(entry['window_size'], entry['still_size']))
        life = next(e for e in entries if e['label'] == 'life')
        with self.assertRaisesRegex(ValueError, 'does not fit'):
            media.check_geometry(life | {'window_size': [1800, 1174]})
        with self.assertRaisesRegex(ValueError, 'explicit crop rectangle'):
            media.check_geometry(life | {'still_size': [1800, 1174]})
        with self.assertRaisesRegex(ValueError, 'explicit normalised rectangle'):
            media.check_geometry(life | {'crop': 'board'})

    def test_capture_rejects_os_clamp_before_starting_any_capture(self):
        good = {'rect': [30, 80, 1500, 978], 'asleep': False, 'covered': [], 'onscreen': True}
        clamped = good | {'rect': [30, 80, 1698, 981]}
        entry = {'label': 'test', 'window_size': [1680, 960], 'timeout': 600,
                 'gate': {'kind': 'delay'}, 'clip': 'test.mp4', 'clip_seconds': 15,
                 'still': 'test.webp', 'still_at': 0}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            neutral, take = root / 'neutral', root / 'take'
            neutral.mkdir(); take.mkdir()
            with patch.object(media.subprocess, 'Popen') as start, \
                 patch.object(media, 'window_info', side_effect=[good, clamped]), \
                 patch.object(media, 'position'), patch.object(media.time, 'sleep'), \
                 patch.object(media.runner, 'stop') as stop, \
                 patch.object(media.subprocess, 'run') as shot:
                start.return_value.poll.return_value = None
                with self.assertRaisesRegex(ValueError, 'window resized by the OS: asked 1680x960, got 1698x981'):
                    media.capture(entry, Path('unused'), neutral, take, Path('unused'), {})
                self.assertEqual(start.call_count, 1)  # app only; no recording process
                shot.assert_not_called()
                stop.assert_called_once_with(start.return_value)

    def test_encoder_rejects_implicit_aspect_change_and_wrong_capture_aspect(self):
        info = {'streams': [{'width': 3360, 'height': 1920}], 'format': {'duration': '20'}}
        with patch.object(encode, 'probe', return_value=info), patch.object(encode, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'explicit crop rectangle'):
                encode.still('raw.png', 'hero.webp', [1800, 1174])
            with self.assertRaisesRegex(ValueError, 'captured media aspect'):
                encode.still('raw.png', 'hero.webp', [1680, 960], expected_size=[1800, 1174])
            with self.assertRaisesRegex(ValueError, 'captured media aspect'):
                encode.clip('raw.mov', 'hero.mp4', 15, expected_size=[1800, 1174])
            run.assert_not_called()

    def test_still_encoder_scales_whole_window_and_only_applies_explicit_crop(self):
        info = {'streams': [{'width': 3360, 'height': 1920}]}
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp) / 'hero.webp'
            def fake_encode(argv):
                if argv[0] == 'cwebp':
                    dest.write_bytes(b'encoded')
            with patch.object(encode, 'probe', return_value=info), \
                 patch.object(encode, 'run', side_effect=fake_encode) as run:
                encode.still('raw.png', dest, [1680, 960], expected_size=[1680, 960])
                argv = run.call_args_list[0].args[0]
                filters = argv[argv.index('-vf') + 1]
                self.assertEqual(filters, 'scale=1680:960:flags=lanczos,setsar=1')
                run.reset_mock()
                encode.still('raw.png', dest, [1000, 690], crop=[10, 20, 600, 400], expected_size=[1680, 960])
                argv = run.call_args_list[0].args[0]
                filters = argv[argv.index('-vf') + 1]
                self.assertEqual(filters.count('crop='), 1)
                self.assertTrue(filters.startswith('crop=600:400:10:20,'))
                self.assertIn('force_original_aspect_ratio=decrease', filters)
                self.assertIn('pad=1000:690', filters)

    def test_clip_encoder_preserves_entire_frame(self):
        source = {'streams': [{'width': 3360, 'height': 1920}], 'format': {'duration': '20'}}
        output = {'streams': [{'width': 1100, 'height': 628, 'codec_name': 'h264'}]}
        with patch.object(encode, 'probe', side_effect=[source, output]), \
             patch.object(encode, 'run') as run:
            encode.clip('raw.mov', 'hero.mp4', 15, expected_size=[1680, 960])
            argv = run.call_args.args[0]
            self.assertEqual(argv[argv.index('-vf') + 1], 'fps=12,scale=1100:-2:flags=lanczos,setsar=1')
            self.assertEqual(argv[argv.index('-crf') + 1], '28')
            self.assertEqual(argv[argv.index('-preset') + 1], 'veryslow')

    def test_clip_profiles_are_validated_offline(self):
        self.assertEqual(encode.clip_profile(), {'crf': 28, 'preset': 'veryslow', 'fps': 12})
        for profile in ({'fps': 0}, {'fps': 12.5}, {'crf': 52}, {'preset': 'invalid'}):
            with self.subTest(profile=profile), self.assertRaises(ValueError):
                encode.clip_profile(**profile)

    def test_explicit_layout_rectangle_maps_to_backing_pixels(self):
        trace = '# layout\twindow x 0..1500 y 0..950\tboard x 0..300 y 40..400\tdie map x 310..1500 y 50..950\n'
        with patch.object(encode, 'probe', return_value={'streams': [{'width': 3000, 'height': 1956}]}):
            crop = encode.crop_layout(trace, {'layout': 'board', 'rect': [0.25, 0, 0.5, 1]},
                                      'raw.png', [30, 80, 1500, 978])
            self.assertEqual(crop, [150, 136, 300, 720])

    def test_refresh_preserves_build_cache_and_updates_sources_and_symlinks(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources, neutral = root / 'resources', root / 'neutral'
            project = resources / 'examples/ulx3s/life_full'
            project.mkdir(parents=True)
            (project / 'top.v').write_text('first source')
            (project / 'alias.v').symlink_to('top.v')
            neutral.mkdir()
            media.refresh_examples(resources, neutral)
            copied = neutral / 'examples/ulx3s/life_full'
            cache = copied / 'build/.flow/state.json'
            cache.parent.mkdir(parents=True)
            cache.write_text('cached hashes')
            routed = copied / 'build/top.config'
            routed.write_text('forty minutes of place and route')
            def identities():
                return [(s.st_ino, s.st_mtime_ns, s.st_size) for s in (cache.stat(), routed.stat())]
            before = identities()
            (project / 'top.v').write_text('updated source')
            (copied / 'obsolete.v').write_text('removed from package')
            # A package refresh must not replace the neutral cache with shipped build files.
            (project / 'build').mkdir()
            (project / 'build/top.config').write_text('do not copy')
            media.refresh_examples(resources, neutral)
            media.refresh_examples(resources, neutral)  # repeat symlink refresh is safe
            self.assertEqual((copied / 'top.v').read_text(), 'updated source')
            self.assertEqual(os.readlink(copied / 'alias.v'), 'top.v')
            self.assertFalse((copied / 'obsolete.v').exists())
            self.assertEqual(cache.read_text(), 'cached hashes')
            self.assertEqual(routed.read_text(), 'forty minutes of place and route')
            self.assertEqual(before, identities())

    def test_removed_example_keeps_its_build_folder(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources, neutral = root / 'resources', root / 'neutral'
            (resources / 'examples').mkdir(parents=True)
            orphan = neutral / 'examples/ulx3s/removed'
            (orphan / 'build').mkdir(parents=True)
            (orphan / 'build/top.config').write_text('keep')
            (orphan / 'old.v').write_text('remove')
            media.refresh_examples(resources, neutral)
            self.assertEqual((orphan / 'build/top.config').read_text(), 'keep')
            self.assertFalse((orphan / 'old.v').exists())

    def test_fixed_neutral_reuses_existing_folder_and_default_is_fresh(self):
        with tempfile.TemporaryDirectory(dir='/private/tmp') as temp:
            fixed = Path(temp) / 'reusable'
            with patch.dict(os.environ, {'ECP5_MEDIA_NEUTRAL': str(fixed)}):
                self.assertEqual(media.neutral_folder(), fixed)
                marker = fixed / 'keep'
                marker.write_text('retained')
                self.assertEqual(media.neutral_folder(), fixed)
                self.assertEqual(marker.read_text(), 'retained')
            with patch.dict(os.environ, {'ECP5_MEDIA_NEUTRAL': ''}), \
                 patch.object(media.tempfile, 'mkdtemp', return_value=str(Path(temp) / 'fresh')) as fresh:
                self.assertEqual(media.neutral_folder(), Path(temp) / 'fresh')
                self.assertEqual(fresh.call_args.kwargs['dir'], Path('/private/tmp/ecp5-media'))

    def test_prepare_uses_flow_statuses_deduplicates_and_applies_design_timeouts(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources, neutral, take = root / 'resources', root / 'neutral', root / 'take'
            take.mkdir()
            for design in ('ulx3s/life_full', 'ulx3s/blinky_cycle'):
                project = neutral / 'examples' / design
                (project / 'build').mkdir(parents=True)
                (project / 'ecp5.toml').write_text('[project]')
                (project / 'build/top.config').write_text('config')
            def fake_build(argv, **kwargs):
                skipped = 'life_full' not in argv[-1]
                kwargs['stdout'].write('[1/3] Synthesis (yosys) … skipped: inputs unchanged\n'
                    '[2/3] Place and route (nextpnr-ecp5) … ' +
                    ('skipped: inputs unchanged\n' if skipped else 'done in 2400.0 s\n') +
                    '[3/3] Bitstream (ecppack) … skipped: inputs unchanged\n')
                return subprocess.CompletedProcess(argv, 0)
            entries = [{'label': 'life', 'project': 'examples/ulx3s/life_full'},
                       {'label': 'life-zoom', 'project': 'examples/ulx3s/life_full'},
                       {'label': 'cross-probe', 'project': 'examples/ulx3s/blinky_cycle'}]
            with patch.object(media.subprocess, 'run', side_effect=fake_build) as build, \
                 contextlib.redirect_stdout(io.StringIO()):
                reports = media.prepare_projects(entries, resources, neutral, take)
                # Reuse also works when the build HOME already exists.
                media.prepare_projects(entries, resources, neutral, take)
            self.assertEqual([r['status'] for r in reports], ['skipped', 'ran'])
            self.assertEqual([c.kwargs['timeout'] for c in build.call_args_list], [600, 3600, 600, 3600])
            self.assertEqual(json.loads((take / 'builds.json').read_text()), reports)
            self.assertEqual(reports[1]['steps'][1]['status'], 'ran')
            self.assertTrue(all(c.args[0][0] == str(resources / 'bin/ecp5_emu') for c in build.call_args_list))

    def test_prebuild_mode_never_reaches_screen_operations(self):
        def forbidden(*args, **kwargs):
            self.fail('prebuild reached a screen operation')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            resources, neutral = root / 'resources', root / 'neutral'
            (resources / 'examples').mkdir(parents=True)
            neutral.mkdir()
            with patch('sys.argv', ['media.py', 'prebuild', 'ulx3s/life_full']), \
                 patch.dict(os.environ, {'ECP5_MEDIA_NEUTRAL': str(neutral)}), \
                 patch.object(media, 'ROOT', root), patch.object(media.sys, 'platform', 'darwin'), \
                 patch.object(media, 'load', return_value=([], {}, 16)), \
                 patch.object(media, 'package', return_value=(root, root, resources)), \
                 patch.object(media, 'neutral_folder', return_value=neutral), \
                 patch.object(media, 'build_designs') as build, \
                 patch.object(media.runner, 'screen_lock', forbidden), \
                 patch.object(media.runner, 'console_unlocked', forbidden), \
                 patch.object(media, 'window_info', forbidden), \
                 patch.object(media, 'capture', forbidden), \
                 patch.object(media.subprocess, 'Popen', forbidden), \
                 contextlib.redirect_stdout(io.StringIO()):
                media.main()
                self.assertEqual(build.call_args.args[0], ['ulx3s/life_full'])

    def script(self, source, project=False):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'test.script'
            path.write_text(source)
            return media.commands(path, project)

    def test_rejects_unsafe_or_mistyped_window_commands(self):
        for command in ('hide', 'quit', 'rate nope', 'board oled', 'panel old open',
                        'values maybe', 'watch', 'new-project {"run":true}'):
            with self.subTest(command=command), self.assertRaises(ValueError):
                self.script('0 ' + command)
        for source in ('nan run', '-1 run', '2 run\n1 pause'):
            with self.assertRaises(ValueError):
                self.script(source)

    def test_project_delays_and_json_comments(self):
        self.assertEqual(len(self.script('2 build\n0 emulate', True)), 2)
        self.script('0 new-project {"name":"Counter #1","dir":"counter","resources":["clock"]} # comment')
        with self.assertRaises(ValueError):
            self.script('0 source top.v 0 32', True)

    def test_creation_rejects_removed_run_field_including_false(self):
        for value in ('false', 'true', 'null', '0'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.script('0 new-project {"name":"Counter","dir":"counter",'
                            '"resources":["clock","leds"],"run":' + value + '}')

    def test_oled_requires_simulation_score_and_both_visible_screens(self):
        budget = media.oled_budget(media.ROOT / 'examples/gallery/oled_bricks/README.md')
        self.assertEqual(budget['min_cycles'], 37523707)
        entry = {'gate': {'kind': 'oled'}}
        trace = ('t\tcycle\n1\t37523707\n'
                 '# uart\t1\t37523707\tF004B C12345678 S003 L3 B59\\r\\n\n'
                 '# oled picture 76 on frames 76 data crc 12345678\n'
                 '# board OLED module asked for (picture 76, with lit pixels): module drawn, screen drawn, lit pixels drawn\n')
        self.assertTrue(media.gate_ready(entry, trace, '', budget))
        for before, after in [('37523707', '100'), ('S003', 'S000'),
                              ('B59', 'B62'), ('screen drawn', 'screen not in sight'),
                              ('on frames 76', 'on frames 0')]:
            self.assertFalse(media.gate_ready(entry, trace.replace(before, after), '', budget))
        # A later score reset must invalidate old positive-score evidence.
        trace += '# uart\t2\t40000000\tF004C C12345678 S000 L3 B62\\r\\n\n'
        self.assertFalse(media.gate_ready(entry, trace, '', budget))

    def test_window_guard_rejects_absence_occlusion_sleep_and_motion(self):
        good = {'rect': [30, 80, 1500, 978], 'asleep': False, 'covered': [], 'onscreen': True}
        self.assertEqual(media.safe_window(good), good['rect'])
        for change in ({'rect': None}, {'asleep': True}, {'covered': ['Notification Centre']}, {'onscreen': False}):
            with self.assertRaises(ValueError):
                media.safe_window(good | change)
        with self.assertRaises(ValueError):
            media.safe_window(good, [31, 80, 1500, 978])

    def test_abort_reaps_capture_before_closing_window(self):
        # A moved window during recording must discard raw pixels before app stop.
        events = []
        class Process:
            pid = 123
            returncode = None
            def poll(self):
                return None
            def kill(self):
                events.append('kill capture')
            def wait(self):
                events.append('reap capture')
                self.returncode = -9
        gui, cap = Process(), Process()
        cap.poll = lambda: cap.returncode
        good = {'rect': [30, 80, 1500, 978], 'asleep': False, 'covered': [], 'onscreen': True}
        moved = good | {'rect': [31, 80, 1500, 978]}
        entry = {'label': 'test', 'script': 'tools/site_media/shots/start.window.script',
                 'window_size': [1500, 978], 'timeout': 600, 'gate': {'kind': 'delay'},
                 'clip': 'test.mp4', 'clip_seconds': 15, 'still': None, 'still_at': 0}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            neutral, take = root / 'neutral', root / 'take'
            neutral.mkdir(); take.mkdir()
            raw = take / 'test.raw.mov'
            raw.write_bytes(b'partial')
            with patch.object(media.subprocess, 'Popen', side_effect=[gui, cap]), \
                 patch.object(media, 'window_info', side_effect=[good, good, good, good, moved]), \
                 patch.object(media, 'position'), patch.object(media.time, 'sleep'), \
                 patch.object(media.runner, 'stop', side_effect=lambda proc: events.append('stop gui')):
                with self.assertRaisesRegex(ValueError, 'moved/resized'):
                    media.capture(entry, Path('unused'), neutral, take, Path('unused'), {})
            self.assertEqual(events, ['kill capture', 'reap capture', 'stop gui'])
            self.assertFalse(raw.exists())

    def test_check_never_calls_live_operations(self):
        def forbidden(*args, **kwargs):
            self.fail('offline check reached a live operation')
        with patch('sys.argv', ['media.py', 'all', '--check']), \
             patch.object(media.runner, 'screen_lock', forbidden), \
             patch.object(media.runner, 'console_unlocked', forbidden), \
             patch.object(media, 'package', forbidden), \
             patch.object(media.subprocess, 'Popen', forbidden), \
             contextlib.redirect_stdout(io.StringIO()):
            media.main()
        with patch('sys.argv', ['media.py', 'prebuild', 'ulx3s/life_full', '--check']), \
             patch.object(media, 'package', forbidden), \
             patch.object(media, 'neutral_folder', forbidden), \
             patch.object(media.subprocess, 'Popen', forbidden), \
             contextlib.redirect_stdout(io.StringIO()):
            media.main()


if __name__ == '__main__':
    unittest.main()
