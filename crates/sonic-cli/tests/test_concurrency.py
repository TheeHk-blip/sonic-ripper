#!/usr/bin/env python3
"""Regression tests for get-catalog download concurrency.

The original bug: --concurrency N was both the album pool size and the
--concurrency passed to each sonic-cli album process, so N albums spawned
N track workers each (N*N processes). Now --concurrency N means one album with
N tracks, --album-concurrency N means N albums with one track each, and pairing
the two runs A*T downloads capped at 16 so the multiplication cannot come back.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import copy
import importlib.util
import io
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

SCRIPT = Path(__file__).resolve().parents[1] / 'src' / 'ytm-artist-batch.py'
spec = importlib.util.spec_from_file_location('ytm_artist_batch', SCRIPT)
batch = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = batch
spec.loader.exec_module(batch)

ALBUMS = [(str(1960 + i), f'Album {i + 1}', f'OLAK5uy_test{i + 1}')
          for i in range(6)]


def flag_values(argv, flag):
    """All values given to `flag` in an argv list (space or `=` form)."""
    values = []
    for i, token in enumerate(argv):
        if token == flag and i + 1 < len(argv):
            values.append(argv[i + 1])
        elif token.startswith(flag + '='):
            values.append(token.split('=', 1)[1])
    return values


class ConcurrencyTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.albums_file = self.base / 'albums.tsv'
        self.albums_file.write_text(
            ''.join(f'{year}\t{title}\t{pinned}\n' for year, title, pinned in ALBUMS),
            encoding='utf-8')
        self.saved_argv = sys.argv
        self.addCleanup(setattr, sys, 'argv', self.saved_argv)

    def run_catalog(self, extra_argv, verbose=False):
        """Run main() with all external I/O stubbed except concurrency logic."""
        calls = []
        stats = {'active': 0, 'peak': 0}
        lock = threading.Lock()

        def fake_download(args, extra, timeout, total_bytes=0, file_totals=None,
                          label='', concurrency=None):
            with lock:
                stats['active'] += 1
                stats['peak'] = max(stats['peak'], stats['active'])
            try:
                # Hold the slot briefly so parallel pool workers really overlap.
                time.sleep(0.05)
                # Pretend sonic-cli produced one album folder in its private
                # work dir. Per-album names keep the test itself race-free.
                work = Path(extra[extra.index('--output-dir') + 1])
                (work / f'folder-{label}').mkdir(parents=True, exist_ok=True)
                calls.append({
                    # What run_artist decided to pass to the download helper.
                    'worker': concurrency,
                    # What sonic-cli actually receives after cli_opts() rewrites it.
                    'argv': batch.cli_opts(args, extra, concurrency=concurrency),
                    'album_pool': batch.worker_slots(args),
                    'track_budget': batch.track_slots(args),
                    'cli_args': args.cli_args,
                    'explicit': args.batch_explicit,
                })
                return SimpleNamespace(returncode=0, stdout='', stderr='')
            finally:
                with lock:
                    stats['active'] -= 1

        def forbid_run(*_a, **_kw):
            self.fail('test must not spawn a real subprocess')

        argv = [
            'get-catalog', '--artist', 'Test Artist',
            '--albums', str(self.albums_file),
            '--root', str(self.base / 'library'),
            '--state', str(self.base / 'state.json'),
            '--config-dir', str(self.base / 'cli-config'),
            '--no-artist-index', '--cli', '/bin/true', '--ytdlp', '/bin/true',
            *extra_argv,
        ]
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(sys, 'argv', argv))
            if not verbose:
                stack.enter_context(mock.patch.object(batch, 'log', lambda *a, **k: None))
            stack.enter_context(
                mock.patch.object(batch, 'install_signal_handlers', lambda *a, **k: None))
            stack.enter_context(mock.patch.object(batch, 'download', fake_download))
            stack.enter_context(mock.patch.object(batch, 'run', forbid_run))
            stack.enter_context(
                mock.patch.object(batch, 'validate',
                                  lambda ytdlp, pid, title: {
                                      'playlist': title, 'album': title,
                                      'date': '1965', 'type': 'album',
                                      'tracks': 8, 'titles': []}))
            stack.enter_context(mock.patch.object(batch, 'score', lambda *a: 100))
            stack.enter_context(
                mock.patch.object(batch, 'playlist_sizes', lambda *a: [10 ** 6] * 8))
            batch.main()
        return calls, stats['peak']

    def assert_plan(self, calls, peak, albums, tracks):
        """Shared invariant: album pool A, per-album T, total A*T."""
        self.assertEqual(len(calls), len(ALBUMS), 'each album should be attempted')
        self.assertTrue(all(c['worker'] == tracks for c in calls),
                        f'album jobs must use {tracks} track worker(s): '
                        f'{[c["worker"] for c in calls]}')
        for call in calls:
            self.assertEqual(flag_values(call['argv'], '--concurrency'), [str(tracks)],
                             f'sonic-cli argv should carry one --concurrency {tracks}: '
                             f'{call["argv"]}')
            self.assertEqual(call['album_pool'], albums)
            self.assertEqual(call['track_budget'], albums * tracks)
        self.assertLessEqual(peak, albums,
                             f'never more than {albums} album processes at once')
        if albums > 1:
            self.assertGreaterEqual(peak, 2, 'album jobs should overlap at all')
        else:
            self.assertEqual(peak, 1)

    def test_default_is_album_only(self):
        """No flags => 6 albums at once, one track each."""
        calls, peak = self.run_catalog([])
        self.assert_plan(calls, peak, albums=6, tracks=1)

    def test_concurrency_is_track_only(self):
        """--concurrency 6 => 1 album with 6 tracks, never 6 albums."""
        calls, peak = self.run_catalog(['--concurrency', '6'])
        self.assert_plan(calls, peak, albums=1, tracks=6)
        self.assertTrue(all(c['explicit'] is None for c in calls))

    def test_track_concurrency_alone_is_track_only(self):
        """--track-concurrency 6 without album flag is the same as --concurrency 6."""
        calls, peak = self.run_catalog(['--track-concurrency', '6'])
        self.assert_plan(calls, peak, albums=1, tracks=6)

    def test_album_concurrency_alone_is_album_only(self):
        """--album-concurrency 6 => 6 albums, one track each."""
        calls, peak = self.run_catalog(['--album-concurrency', '6'])
        self.assert_plan(calls, peak, albums=6, tracks=1)

    def test_album_and_track_knobs(self):
        """--album-concurrency 2 --track-concurrency 3 => 2 x 3 = 6 slots."""
        calls, peak = self.run_catalog(['--album-concurrency', '2',
                                        '--track-concurrency', '3'])
        self.assert_plan(calls, peak, albums=2, tracks=3)

    def test_short_aliases(self):
        calls, peak = self.run_catalog(['--album-c', '3', '--track-c', '2'])
        self.assert_plan(calls, peak, albums=3, tracks=2)

    def test_explicit_cli_arg_maps_to_track_concurrency(self):
        """Legacy --cli-args --concurrency N can no longer multiply silently."""
        calls, peak = self.run_catalog(['--cli-args', '--concurrency 9'])
        self.assert_plan(calls, peak, albums=1, tracks=9)
        self.assertTrue(all(c['explicit'] == 9 for c in calls))

    def test_safety_cap_rejects_process_explosion(self):
        """A * T above 16 is refused instead of spawning hundreds of processes."""
        for argv, needle in (
                (['--album-concurrency', '8', '--track-concurrency', '4'],
                 'exceeds the 16-download safety cap'),
                (['--track-concurrency', '0'], 'must be between 1 and 16'),
                (['--album-concurrency', '17'], 'must be between 1 and 16'),
                (['--concurrency', '0'], 'must be between 1 and 16')):
            with self.subTest(argv=argv):
                stderr = io.StringIO()
                with ExitStack() as stack:
                    stack.enter_context(mock.patch.object(sys, 'argv',
                                                          ['get-catalog', '--artist',
                                                           'Test Artist', *argv]))
                    stack.enter_context(mock.patch.object(sys, 'stderr', stderr))
                    stack.enter_context(mock.patch.object(batch, 'log', lambda *a, **k: None))
                    with self.assertRaises(SystemExit) as raised:
                        batch.main()
                self.assertEqual(raised.exception.code, 2)
                self.assertIn(needle, stderr.getvalue())

    def test_library_scans_ignore_transient_download_dirs(self):
        """A parallel album's .download-* dir may vanish during a library scan."""
        root = self.base / 'library'
        work = root / '.download-live'
        work.mkdir(parents=True)
        album = root / '1960 - Album 1'
        album.mkdir()

        def audio_files_or_raise(folder):
            if folder.name.startswith('.download-'):
                # What really happened: the dir was removed between is_dir()
                # and iterdir() by the album job that owned it.
                raise FileNotFoundError(str(folder))
            return [object()] if folder == album else []

        with mock.patch.object(batch, 'audio_files', audio_files_or_raise):
            found = batch.find_existing_album(root, 'Album 1', '1960', 'Test Artist')
        self.assertEqual(found, album)
        self.assertFalse(batch.is_library_folder(work))
        self.assertTrue(batch.is_library_folder(album))

    def run_multi_artist(self, extra_argv):
        captured = []

        def fake_run_artist(args, artist, song_titles, album_titles):
            captured.append(copy.copy(args))

        argv = ['get-catalog', '--artists', json.dumps(['Artist A', 'Artist B']),
                '--cli', '/bin/true', '--ytdlp', '/bin/true', *extra_argv]
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(sys, 'argv', argv))
            stack.enter_context(mock.patch.object(batch, 'log', lambda *a, **k: None))
            stack.enter_context(
                mock.patch.object(batch, 'install_signal_handlers', lambda *a, **k: None))
            stack.enter_context(mock.patch.object(batch, 'run_artist', fake_run_artist))
            batch.main()
        return captured

    def test_multi_artist_track_only_split(self):
        """--concurrency 6 with two artists => 1 album + 3 tracks each."""
        workers = self.run_multi_artist(['--concurrency', '6'])
        self.assertEqual(len(workers), 2)
        self.assertEqual([w.worker_concurrency for w in workers], [1, 1])
        self.assertEqual([w.batch_track_concurrency for w in workers], [3, 3])
        self.assertEqual(sum(batch.track_slots(w) for w in workers), 6)

    def test_multi_artist_track_only_remainder(self):
        """--concurrency 7 with three artists => 3, 2, 2 tracks; one album each."""
        captured = []
        argv = ['get-catalog', '--artists', json.dumps(['A', 'B', 'C']),
                '--cli', '/bin/true', '--ytdlp', '/bin/true',
                '--concurrency', '7']
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(sys, 'argv', argv))
            stack.enter_context(mock.patch.object(batch, 'log', lambda *a, **k: None))
            stack.enter_context(
                mock.patch.object(batch, 'install_signal_handlers',
                                  lambda *a, **k: None))
            stack.enter_context(
                mock.patch.object(batch, 'run_artist',
                                  lambda args, *a: captured.append(copy.copy(args))))
            batch.main()
        self.assertEqual([w.batch_track_concurrency for w in captured], [3, 2, 2])
        self.assertTrue(all(w.worker_concurrency == 1 for w in captured))
        self.assertEqual(sum(batch.track_slots(w) for w in captured), 7)

    def test_multi_artist_album_only_split(self):
        """--album-concurrency 6 with two artists => 3 + 3 albums, 1 track."""
        workers = self.run_multi_artist(['--album-concurrency', '6'])
        self.assertEqual([w.worker_concurrency for w in workers], [3, 3])
        self.assertTrue(all(w.batch_track_concurrency == 1 for w in workers))
        self.assertEqual(sum(batch.track_slots(w) for w in workers), 6)

    def test_multi_artist_album_and_track_split(self):
        """--album-concurrency 4 --track-concurrency 2 => 2+2 albums x 2 tracks."""
        workers = self.run_multi_artist(['--album-concurrency', '4',
                                         '--track-concurrency', '2'])
        self.assertEqual(len(workers), 2)
        self.assertEqual([w.worker_concurrency for w in workers], [2, 2])
        self.assertTrue(all(w.batch_track_concurrency == 2 for w in workers))
        self.assertEqual(sum(batch.track_slots(w) for w in workers), 8)

    def test_helper_contracts(self):
        self.assertEqual(batch.split_evenly(6, 2), [3, 3])
        self.assertEqual(batch.split_evenly(7, 3), [3, 2, 2])
        self.assertEqual(batch.explicit_concurrency('--concurrency=4 --no-id3'), 4)
        self.assertIsNone(batch.explicit_concurrency('--no-id3'))

        args = SimpleNamespace(batch_slots=2, batch_track_concurrency=3,
                               cli='cli', cli_config_dir='/cfg', cookies='/ck',
                               format='opus', bitrate='lossless', sample_rate=None,
                               video_quality=None,
                               cli_args='--concurrency 9 --no-id3')
        self.assertEqual(batch.worker_slots(args), 2, 'album pool ignores cli-args')
        self.assertEqual(batch.track_slots(args), 6)
        opts = batch.cli_opts(args, ['--is-album'], config_dir='/private',
                              concurrency=3)
        self.assertEqual(flag_values(opts, '--concurrency'), ['3'])
        self.assertIn('--no-id3', opts)
        self.assertIn('--is-album', opts)

        opts = batch.cli_opts(args, concurrency=3)
        self.assertEqual(flag_values(opts, '--concurrency'), ['3'])
        self.assertEqual(
            flag_values(batch.cli_opts(args), '--concurrency'), ['9'],
            'without a forced value the user flag must survive')

        with_tools = SimpleNamespace(**vars(args), binaries_dir='/fixed-tools')
        self.assertEqual(
            flag_values(batch.cli_opts(with_tools), '--binaries-dir'),
            ['/fixed-tools'],
            'downloads must use the fixed sidecars, not the CLI symlink target')


if __name__ == '__main__':
    unittest.main()
