#!/usr/bin/env python3
"""Regression tests for Tier 2 discovery/index/monitor optimizations.

Discovery lookups overlap in a small fixed pool but are applied in the original
order; release members are prefetched in that same order; cached indexes and
unchanged tags are not rewritten.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import json
import sys
import tempfile
import threading
import time
import unittest
from contextlib import ExitStack
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'
sys.path.insert(0, str(SRC))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ytm = load('ytm_index_tier2', SRC / 'ytm_index.py')
batch = load('batch_tier2', SRC / 'ytm-artist-batch.py')


def member_response(rows):
    """Minimal InnerTube browse payload with responsive list rows."""
    items = []
    for vid, title in rows:
        items.append({
            'musicResponsiveListItemRenderer': {
                'playlistItemData': {'videoId': vid},
                'flexColumns': [{
                    'musicResponsiveListItemFlexColumnRenderer': {
                        'text': {'simpleText': title},
                    },
                }],
            },
        })
    return {'contents': items}


class ParallelDiscoveryTest(unittest.TestCase):
    def test_parallel_map_is_bounded_and_returns_original_order(self):
        lock = threading.Lock()
        active = peak = 0

        def job(index):
            nonlocal active, peak
            with lock:
                active += 1
                peak = max(peak, active)
            try:
                time.sleep(0.02)
            finally:
                with lock:
                    active -= 1
            return index * 2

        jobs = [lambda i=i: job(i) for i in range(5)]
        self.assertEqual(ytm.parallel_map(jobs, workers=3), [0, 2, 4, 6, 8])
        self.assertGreater(peak, 1, 'independent lookups should overlap')
        self.assertLessEqual(peak, 3, 'pool must stay bounded')

    def test_parallel_map_with_one_worker_is_sequential(self):
        self.assertEqual(ytm.parallel_map([lambda: 1, lambda: 2], workers=1),
                         [1, 2])

    def test_safe_call_contains_expected_index_failures(self):
        self.assertEqual(ytm.safe_call(lambda: 'ok'), (True, 'ok'))
        error = ytm.YtmError('offline')
        def fail():
            raise error
        self.assertEqual(ytm.safe_call(fail), (False, error))

    def test_build_passes_artist_to_search_helper(self):
        seen = []

        def fake_browse_id(artist):
            seen.append(artist)
            return ('MPREb-test', 'Test Artist')

        def fake_api(endpoint, extra, **_kwargs):
            return {'contents': []}

        with mock.patch.object(ytm, 'artist_browse_id', fake_browse_id), \
                mock.patch.object(ytm, 'api', fake_api):
            index = ytm.build('test artist', note=lambda _message: None)

        self.assertEqual(seen, ['test artist'])
        self.assertEqual(index['artistBrowseId'], 'MPREb-test')

    def test_read_upgrades_old_cache_without_freshness_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'index.json'
            path.write_text(json.dumps({
                'fetched': 0,
                'tracks': {'song key': {'videoId': 'vid-1', 'title': 'Song'}},
            }), encoding='utf-8')
            index = ytm.read(path)
            self.assertEqual(index['videos']['vid-1']['title'], 'Song')
            self.assertEqual(index['members'], {})
            self.assertTrue(ytm.is_fresh(index, max_age=0))
            self.assertFalse(ytm.is_fresh(index, max_age=1))


class ReleaseMemberPrefetchTest(unittest.TestCase):
    def test_missing_members_are_fetched_in_key_order(self):
        index = {
            'albums': {
                'b key': {'title': 'B Album', 'browseId': 'browse-b'},
                'a key': {'title': 'A Album', 'browseId': 'browse-a'},
                'c key': {'title': 'C Album', 'browseId': 'browse-c'},
            },
            'members': {'a key': [{'videoId': 'cached'}]},
        }
        calls = []

        def fake_api(endpoint, extra, **_kwargs):
            self.assertEqual(endpoint, 'browse')
            bid = extra['browseId']
            calls.append(bid)
            # C finishes first; notes must still follow the requested key order.
            if bid == 'browse-b':
                time.sleep(0.02)
            rows = {
                'browse-b': [('b1', 'B Song')],
                'browse-c': [('c1', 'C One'), ('c2', 'C Two')],
            }[bid]
            return member_response(rows)

        notes = []
        with mock.patch.object(ytm, 'api', side_effect=fake_api):
            members = ytm.fetch_release_members(
                index, ['b key', 'c key', 'a key'], note=notes.append, workers=2)

        self.assertEqual(sorted(calls), ['browse-b', 'browse-c'],
                         'cached members must not be fetched again')
        self.assertEqual([row['videoId'] for row in members['b key']], ['b1'])
        self.assertEqual([row['videoId'] for row in members['c key']], ['c1', 'c2'])
        self.assertEqual(members['a key'], [{'videoId': 'cached'}])
        self.assertEqual(notes, [
            'index: B Album: 1 member track(s)',
            'index: C Album: 2 member track(s)',
        ])


class LocalStateTest(unittest.TestCase):
    def test_index_signature_ignores_key_insertion_order(self):
        left = {'a': 1, 'b': [2, 3], 'fetched': 123}
        right = {'b': [2, 3], 'a': 1, 'fetched': 123}
        self.assertEqual(batch.index_signature(left), batch.index_signature(right))
        right['fetched'] = 124
        self.assertNotEqual(batch.index_signature(left),
                            batch.index_signature(right))

    def test_resolve_root_uses_saved_state_then_shallow_scan(self):
        with tempfile.TemporaryDirectory() as tmp:
            lib = Path(tmp) / 'MUSIC'
            saved = lib / 'SOUL' / 'MARVIN GAYE'
            saved.mkdir(parents=True)
            with mock.patch.object(batch, 'library_root', return_value=lib):
                self.assertEqual(
                    batch.resolve_root('Marvin Gaye', None, 'SOUL',
                                       {'root': str(saved)}),
                    saved)
                stale = lib / 'MISSING'
                self.assertEqual(
                    batch.resolve_root('Marvin Gaye', None, 'SOUL',
                                       {'root': str(stale)}),
                    saved)
                self.assertEqual(
                    batch.resolve_root('Marvin Gaye', None, 'SOUL', {}), saved)

    def test_pending_tag_updates_skips_matching_values_and_none(self):
        updates = {'TITLE': 'Same', 'date': '2021', 'EXPLICIT': None}
        with mock.patch.object(batch, 'file_tags',
                               return_value={'title': 'Same', 'date': '2020'}):
            self.assertEqual(batch.pending_tag_updates(Path('/track.opus'), updates),
                             {'date': '2021'})

    def test_retag_skips_files_whose_tags_are_already_current(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'Artist'
            folder = root / 'Album'
            folder.mkdir(parents=True)
            track = folder / 'track.opus'
            track.write_bytes(b'audio')
            state = {
                'albums': {'Album': {'folder': 'Album', 'type': 'album'}},
                'singles': {},
            }
            current = {key.casefold(): value for key, value in
                       batch.release_updates('album', None).items()}
            logs = []
            retag_calls = []
            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(
                    batch, 'audio_files', return_value=[track]))
                stack.enter_context(mock.patch.object(
                    batch, 'file_tags', return_value=current))
                stack.enter_context(mock.patch.object(
                    batch, 'retag', side_effect=lambda p, u: retag_calls.append((p, u))))
                stack.enter_context(mock.patch.object(batch, 'save_state'))
                batch.retag_library(
                    SimpleNamespace(artist='Artist'), state, root,
                    Path(tmp) / 'state.json', None,
                    lambda message, **_kwargs: logs.append(str(message)))
            self.assertEqual(retag_calls, [])
            self.assertIn('unchanged skipped', ' '.join(logs))

    def test_retag_only_rewrites_changed_values(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'Artist'
            folder = root / 'Album'
            folder.mkdir(parents=True)
            track = folder / 'track.opus'
            track.write_bytes(b'audio')
            state = {
                'albums': {'Album': {'folder': 'Album', 'type': 'album'}},
                'singles': {},
            }
            current = {key.casefold(): value for key, value in
                       batch.release_updates('album', None).items()}
            current['release_type'] = 'OLD'
            retag_calls = []
            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(
                    batch, 'audio_files', return_value=[track]))
                stack.enter_context(mock.patch.object(
                    batch, 'file_tags', return_value=current))
                stack.enter_context(mock.patch.object(
                    batch, 'retag', side_effect=lambda p, u: retag_calls.append((p, u))))
                stack.enter_context(mock.patch.object(batch, 'save_state'))
                batch.retag_library(
                    SimpleNamespace(artist='Artist'), state, root,
                    Path(tmp) / 'state.json', None, lambda *_a, **_k: None)
            self.assertEqual(retag_calls,
                             [(track, {'RELEASE_TYPE': 'ALBUM'})])


class MemorySettingsLauncherTest(unittest.TestCase):
    def test_download_reuses_config_dir_and_passes_memory_flag(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp) / 'cli-config'
            args = SimpleNamespace(
                cli='/bin/true', cli_config_dir=str(base), cookies='/cookies.txt',
                format='opus', bitrate='lossless', sample_rate=None,
                video_quality=None, cli_args='', binaries_dir='/tools',
                download_env={})
            captured = {}

            def fake_run(cmd, timeout=60, env=None, monitor=None, progress_key=None):
                captured['cmd'] = cmd
                return SimpleNamespace(returncode=0, stdout='', stderr='')

            with ExitStack() as stack:
                stack.enter_context(mock.patch.object(batch, 'run', fake_run))
                stack.enter_context(mock.patch.object(
                    batch.PROGRESS, 'task_begin', return_value='key'))
                stack.enter_context(mock.patch.object(batch.PROGRESS, 'task_end'))
                stack.enter_context(mock.patch.object(
                    batch, 'start_download_bandwidth', return_value={}))
                stack.enter_context(mock.patch.object(batch, 'end_download_bandwidth'))
                batch.download(args, ['--is-album'], timeout=10)

            cmd = captured['cmd']
            self.assertIn('--in-memory-settings', cmd)
            self.assertEqual(cmd[cmd.index('--config-dir') + 1], str(base))
            self.assertEqual([p.name for p in Path(tmp).iterdir()], ['cli-config'],
                             'no private per-download config copy should remain')


if __name__ == '__main__':
    unittest.main()
