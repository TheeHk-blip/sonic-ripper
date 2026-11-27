#!/usr/bin/env python3
"""Regression tests for flat YTM-index track ownership.

The same song can be a member of several releases. Only the earliest release
by year keeps it: the duplicate row is removed from the cached index and its
video id is excluded before sonic-cli analyses a later playlist.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


ytm = load('ytm_index_flat_dedupe', SRC / 'ytm_index.py')
batch = load('batch_flat_dedupe', SRC / 'ytm-artist-batch.py')


FLAT_INDEX = {
    'albums': {
        'early album': {
            'title': 'Early Album', 'year': '2019', 'type': 'album',
            'playlistId': 'OLAK5uyEARLY', 'videoId': 'v-early-cover',
        },
        'late album': {
            'title': 'Late Album', 'year': '2023', 'type': 'album',
            'playlistId': 'OLAK5uyLATE', 'videoId': 'v-late-cover',
        },
    },
    'members': {
        # Different video ids, one normalized title: still the same song.
        'early album': [
            {'videoId': 'v-shared-early', 'title': 'Shared Song (Live)'},
            {'videoId': 'v-early-only', 'title': 'Early Only'},
        ],
        'late album': [
            {'videoId': 'v-late-only', 'title': 'Late Only'},
            {'videoId': 'v-shared-late', 'title': 'Shared Song'},
        ],
    },
    'tracks': {},
    'videos': {},
}


class FlatOwnershipTest(unittest.TestCase):
    def test_earliest_release_owns_duplicate_and_index_is_pruned(self):
        index = {key: {inner: [dict(row) for row in value] if isinstance(value, list)
                       else dict(value) for inner, value in group.items()}
                 for key, group in FLAT_INDEX.items()}
        notes = []
        owners, excluded = ytm.prune_duplicate_members(
            index, 'Test Artist', note=lambda message, **_kwargs: notes.append(message),
            fetch=False)

        self.assertEqual(
            [row['videoId'] for row in index['members']['late album']],
            ['v-late-only'])
        self.assertEqual(
            [row['videoId'] for row in index['members']['early album']],
            ['v-shared-early', 'v-early-only'])
        self.assertEqual(
            [row['videoId'] for row in excluded['late album']],
            ['v-shared-late'])
        self.assertEqual(excluded['late album'][0]['ownerYear'], '2019')
        owner = owners[excluded['late album'][0]['ownerKey']]
        self.assertEqual(owner['title'], 'Early Album')
        self.assertEqual(owner['videoId'], 'v-shared-early')
        self.assertTrue(index['flatDedupe'])
        self.assertIn('1 duplicate track(s) removed', '\n'.join(notes))

    def test_single_release_does_not_own_an_album_track(self):
        index = {key: {inner: [dict(row) for row in value] if isinstance(value, list)
                       else dict(value) for inner, value in group.items()}
                 for key, group in FLAT_INDEX.items()}
        index['albums']['early single'] = {
            'title': 'Shared Song', 'year': '2015', 'type': 'single',
            'playlistId': 'OLAK5uySINGLE', 'videoId': 'v-single-cover',
        }
        index['members']['early single'] = [
            {'videoId': 'v-single-shared', 'title': 'Shared Song'},
        ]

        owners, _ = ytm.prune_duplicate_members(index, 'Test Artist', fetch=False)

        owner = next(value for value in owners.values()
                     if value['trackTitle'] == 'Shared Song (Live)')
        self.assertEqual(owner['title'], 'Early Album')
        self.assertEqual(owner['videoId'], 'v-shared-early')

    def test_legacy_excluded_album_row_is_restored(self):
        index = {key: {inner: [dict(row) for row in value] if isinstance(value, list)
                       else dict(value) for inner, value in group.items()}
                 for key, group in FLAT_INDEX.items()}
        index['members']['early album'] = [
            {'videoId': 'v-early-only', 'title': 'Early Only'},
        ]
        index['albums']['early single'] = {
            'title': 'Shared Song', 'year': '2015', 'type': 'single',
            'playlistId': 'OLAK5uySINGLE', 'videoId': 'v-single-cover',
        }
        index['excludedMembers'] = {
            'early album': [{
                'index': 0,
                'row': {'videoId': 'v-shared-early', 'title': 'Shared Song',
                        'ownerKey': 'v:v-shared-early'},
            }],
        }

        ytm.prune_duplicate_members(index, 'Test Artist', fetch=False)

        self.assertEqual(
            [row['videoId'] for row in index['members']['early album']],
            ['v-shared-early', 'v-early-only'])

    def test_unselected_owner_release_does_not_hide_single(self):
        index = {
            'albums': FLAT_INDEX['albums'],
            'members': FLAT_INDEX['members'],
            'tracks': {
                'shared song': {'title': 'Shared Song', 'videoId': 'v-shared-late',
                                'type': 'single'},
            },
            'trackOwners': {
                'v:v-shared-early': {
                    'key': 'early album', 'title': 'Early Album',
                    'type': 'album', 'year': '2019', 'playlistId': 'OLAK5uyEARLY',
                    'videoId': 'v-shared-early', 'trackTitle': 'Shared Song',
                },
            },
            'memberTitles': {
                'shared song': {'title': 'Early Album', 'type': 'album'},
            },
        }
        singles = [{'title': 'Shared Song'}]

        video_release = batch.build_membership(index, 'Test Artist', [])
        kept = batch.dedupe_singles_for_albums(
            index, 'Test Artist', [], singles, video_release=video_release)

        self.assertEqual(kept, singles)
        self.assertEqual(index['memberTitles'], {})

    def test_empty_excluded_row_is_not_restored(self):
        index = {key: {inner: [dict(row) for row in value] if isinstance(value, list)
                       else dict(value) for inner, value in group.items()}
                 for key, group in FLAT_INDEX.items()}
        index['excludedMembers'] = {
            'early album': [{
                'index': 0,
                'row': {'ownerKey': 'v:old', 'ownerRelease': 'Old Release',
                        'ownerYear': '2010'},
            }],
        }

        ytm.prune_duplicate_members(index, 'Test Artist', fetch=False)

        self.assertTrue(all(row.get('videoId') or row.get('title')
                            for row in index['members']['early album']))

    def test_refresh_merges_rows_added_after_the_cache(self):
        index = {
            'albums': {
                'album': {'title': 'Album', 'year': '2024', 'type': 'album',
                          'browseId': 'MPREbALBUM'},
            },
            'members': {
                'album': [{'videoId': 'v-old', 'title': 'Old Song'}],
            },
        }
        new_rows = [
            {'videoId': 'v-old', 'title': 'Old Song'},
            {'videoId': 'v-new', 'title': 'New Song'},
        ]
        notes = []
        with mock.patch.object(ytm, 'api', return_value={}) as api, \
                mock.patch.object(ytm, '_member_rows', return_value=new_rows):
            ytm.refresh_release_members(
                index, ['album'],
                note=lambda message, **_kwargs: notes.append(message))

        api.assert_called_once_with('browse', {'browseId': 'MPREbALBUM'})
        self.assertEqual(
            [row['videoId'] for row in index['members']['album']],
            ['v-old', 'v-new'])
        self.assertIn('added 1 member track(s)', '\n'.join(notes))

    def test_exclude_flags_handle_video_ids_starting_with_dash(self):
        self.assertEqual(
            batch.exclude_track_flags(['-abc123', 'plain', '-abc123', '']),
            ['--exclude-track=-abc123', '--exclude-track=plain'])


class AlbumExclusionTest(unittest.TestCase):
    def test_library_tracks_are_excluded_from_selected_releases(self):
        index = {
            'trackOwners': {},
            'excludedMembers': {},
            'members': {
                'ep 2023': [{'videoId': 'v-ep', 'title': 'Welcome 2 My Strip'}],
                'album 2024': [{'videoId': 'v-album', 'title': 'Welcome 2 My Strip'}],
            },
        }

        exclusions, flat_count, disk_count = batch.album_duplicate_exclusions(
            index, {'ep 2023', 'album 2024'},
            {batch.norm('WELCOME 2 MY STRIP')})

        self.assertEqual(flat_count, 0)
        self.assertEqual(disk_count, 2)
        self.assertEqual(exclusions['ep 2023'], ['v-ep'])
        self.assertEqual(exclusions['album 2024'], ['v-album'])

    def test_unselected_owner_does_not_exclude_selected_release(self):
        index = {
            'trackOwners': {
                'owner': {'key': 'other album', 'trackTitle': 'Shared Song'},
            },
            'excludedMembers': {
                'selected album': [{
                    'videoId': 'v-shared', 'ownerKey': 'owner',
                    'title': 'Shared Song',
                }],
            },
            'members': {},
        }

        exclusions, flat_count, disk_count = batch.album_duplicate_exclusions(
            index, {'selected album'}, set())

        self.assertEqual((exclusions, flat_count, disk_count), ({}, 0, 0))


class AlbumExclusionSmokeTest(unittest.TestCase):
    """A selected album with exclusions must reach the downloader without NameError."""

    def test_process_album_can_report_library_exclusions(self):
        normkey = batch.ytm_index.normkey
        index = {
            'artist': 'Test Artist',
            'artistName': 'Test Artist',
            'albums': {
                'early album': {
                    'title': 'Early Album', 'year': '2020', 'type': 'album',
                    'playlistId': 'OLAK5uyEARLY', 'videoId': 'v-cover',
                    'explicit': True,
                },
            },
            'members': {
                'early album': [{'videoId': 'v-shared', 'title': 'Shared Song',
                                 'explicit': True}],
            },
            'tracks': {},
            'videos': {},
            'flatDedupe': True,
            'trackOwners': {},
            'excludedMembers': {},
        }
        messages = []

        class StopBeforeDownload(Exception):
            pass

        def stop(*_args, **_kwargs):
            raise StopBeforeDownload

        argv = [
            'get-catalog', '--artist', 'Test Artist',
            '--root', '/tmp/opencode/flat-dedupe-album',
            '--state', '/tmp/opencode/flat-dedupe-album/state.json',
            '--config-dir', '/tmp/opencode/flat-dedupe-album/config',
            '--cli', '/bin/true', '--ytdlp', '/bin/true',
            '--selected-albums', '[["2020", "Early Album", ""]]',
            '--selected-singles', '[]',
            '--disk-album-titles', json.dumps([batch.norm('Shared Song')]),
        ]
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(sys, 'argv', argv))
            stack.enter_context(
                mock.patch.object(batch, 'log',
                                  lambda message, *a, **k: messages.append(str(message))))
            stack.enter_context(
                mock.patch.object(batch, 'install_signal_handlers',
                                  lambda *a, **k: None))
            stack.enter_context(mock.patch.object(batch, 'PROGRESS', mock.MagicMock()))
            stack.enter_context(mock.patch.object(
                batch.ytm_index, 'load', return_value=index))
            stack.enter_context(mock.patch.object(
                batch, 'find_existing_album', return_value=None))
            stack.enter_context(mock.patch.object(
                batch, 'playlist_sizes', return_value=[100, 200]))
            stack.enter_context(mock.patch.object(
                batch, 'register_temp_dir', side_effect=stop))
            with self.assertRaises(StopBeforeDownload):
                batch.main()

        self.assertTrue(any('flat dedupe: skip 1 track(s) already kept in '
                            'an earlier release or library folder' in message
                            for message in messages))


class ForegroundInteractiveDedupeTest(unittest.TestCase):
    """The picker shows duplicates, but confirmation must remove them."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.saved_argv = sys.argv
        self.addCleanup(setattr, sys, 'argv', self.saved_argv)

    def test_selected_duplicate_single_is_dropped_before_queue(self):
        normkey = batch.ytm_index.normkey
        index = {
            'artist': 'Test Artist',
            'albums': {
                'early album': {
                    'title': 'Early Album', 'year': '2020', 'type': 'album',
                    'playlistId': 'OLAK5uyEARLY', 'videoId': 'v-cover',
                },
            },
            'members': {
                'early album': [
                    {'videoId': 'v-shared', 'title': 'Shared Song'},
                ],
            },
            'tracks': {
                normkey('Shared Song', 'Test Artist'): {
                    'title': 'Shared Song', 'year': '2020',
                    'videoId': 'v-shared',
                },
                normkey('Only Single', 'Test Artist'): {
                    'title': 'Only Single', 'year': '2021',
                    'videoId': 'v-only',
                },
            },
            'videos': {},
            'flatDedupe': True,
            'trackOwners': {},
            'excludedMembers': {},
        }
        albums = [('2020', 'Early Album', '')]
        singles = [
            {'title': 'Shared Song', 'year': '2020'},
            {'title': 'Only Single', 'year': '2021'},
        ]
        events = []
        messages = []
        captured = {}

        class StopAfterQueue(Exception):
            pass

        class TtyStringIO(io.StringIO):
            def isatty(self):
                return True

        class DummyDisplay:
            lanes = [object()]

            def __init__(self, *_args, **_kwargs):
                pass

            def suspend(self):
                events.append('display-suspend')

            def resume(self):
                events.append('display-resume')

            def close(self):
                events.append('display-close')

        def fake_picker(picked_albums, picked_singles, artist=''):
            events.append('picker')
            return list(picked_albums), list(picked_singles), 'foreground'

        def fake_catalog(_index):
            events.append('catalog')
            return list(albums), list(singles)

        real_prune = batch.ytm_index.prune_duplicate_members
        real_membership = batch.build_membership
        real_dedupe = batch.dedupe_singles_for_albums

        def recording_prune(*args, **kwargs):
            events.append('prune')
            return real_prune(*args, **kwargs)

        def recording_membership(*args, **kwargs):
            events.append('membership')
            return real_membership(*args, **kwargs)

        def recording_dedupe(*args, **kwargs):
            events.append('dedupe')
            return real_dedupe(*args, **kwargs)

        def stop_before_download(*_args, **_kwargs):
            events.append('stop')
            raise StopAfterQueue

        progress = mock.MagicMock()

        def record_queue(queued_albums, queued_singles):
            events.append('queue')
            captured['albums'] = list(queued_albums)
            captured['singles'] = list(queued_singles)

        progress.set_queue.side_effect = record_queue

        argv = [
            'get-catalog', '--artist', 'Test Artist', '--interactive',
            '--root', str(self.base / 'library'),
            '--state', str(self.base / 'state.json'),
            '--config-dir', str(self.base / 'cli-config'),
            '--cli', '/bin/true', '--ytdlp', '/bin/true',
        ]
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(sys, 'argv', argv))
            stack.enter_context(mock.patch.object(sys, 'stdin', TtyStringIO()))
            stack.enter_context(mock.patch.object(sys, 'stdout', TtyStringIO()))
            stack.enter_context(
                mock.patch.object(batch, 'log',
                                  lambda message, *a, **k: messages.append(str(message))))
            stack.enter_context(
                mock.patch.object(batch, 'install_signal_handlers',
                                  lambda *a, **k: None))
            stack.enter_context(mock.patch.object(batch, 'Display', DummyDisplay))
            stack.enter_context(mock.patch.object(batch, 'PROGRESS', progress))
            stack.enter_context(mock.patch.object(
                batch.ytm_index, 'load', return_value=index))
            stack.enter_context(mock.patch.object(
                batch, 'catalog_from_index', fake_catalog))
            stack.enter_context(mock.patch.object(
                batch, 'interactive_catalog_filter', fake_picker))
            stack.enter_context(mock.patch.object(
                batch.ytm_index, 'prune_duplicate_members', recording_prune))
            stack.enter_context(mock.patch.object(
                batch, 'build_membership', recording_membership))
            stack.enter_context(mock.patch.object(
                batch, 'dedupe_singles_for_albums', recording_dedupe))
            stack.enter_context(mock.patch.object(
                batch, 'clean_stale_tmp_runs', stop_before_download))
            with self.assertRaises(StopAfterQueue):
                batch.main()

        self.assertIn('picker', events)
        self.assertLess(events.index('picker'), events.index('prune'))
        self.assertLess(events.index('picker'), events.index('dedupe'))
        self.assertLess(events.index('dedupe'), events.index('queue'))
        self.assertEqual(captured['albums'], ['Early Album'])
        self.assertEqual(captured['singles'], ['Only Single'])
        self.assertFalse(any('keeping all listed singles' in message
                             for message in messages))
        self.assertTrue(any('removed 1 single(s) already in album(s)'
                            in message for message in messages))


if __name__ == '__main__':
    unittest.main()
