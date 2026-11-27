#!/usr/bin/env python3
"""Regression tests for --cancel and detached catalog session tracking."""
import importlib.util
import json
import signal
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'
spec = importlib.util.spec_from_file_location('batch_cancel_test',
                                              SRC / 'ytm-artist-batch.py')
batch = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = batch
spec.loader.exec_module(batch)


class CancelDetachedTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.proc_root = self.base / 'proc'
        self.proc_root.mkdir()
        self.messages = []
        self.say = lambda message, *a, **k: self.messages.append(str(message))

    def add_proc(self, pid, args):
        folder = self.proc_root / str(pid)
        folder.mkdir(parents=True)
        (folder / 'cmdline').write_bytes(
            b'\0'.join(arg.encode() for arg in args) + b'\0')
        return folder

    def catalog_args(self, *extra):
        return [sys.executable, batch.__file__, '--artist', 'Strandz', *extra]

    def test_pidfile_cancellation_signals_process_group_and_removes_file(self):
        self.add_proc(4242, self.catalog_args(
            '--detached-run', 'strandz', '--run-id', 'run-1'))
        pidfile = batch.write_detached_pidfile(
            'Strandz', 4242, 'run-1', state_dir=self.base, pgid=4242)
        calls = []

        with mock.patch.object(batch, 'find_detached_sessions', return_value=[]), \
                mock.patch.object(batch.os, 'kill', return_value=None), \
                mock.patch.object(batch.os, 'getpgid', return_value=4242), \
                mock.patch.object(batch.os, 'getsid', return_value=4242), \
                mock.patch.object(batch.os, 'killpg',
                                  side_effect=lambda pgid, sig: calls.append((pgid, sig))), \
                mock.patch.object(batch, 'process_group_alive', return_value=False):
            ok = batch.cancel_catalog_session(
                'strandz', state_dir=self.base, say=self.say,
                proc_root=self.proc_root, timeout=0)

        self.assertTrue(ok)
        self.assertEqual(calls, [(4242, signal.SIGTERM)])
        self.assertFalse(pidfile.exists())
        self.assertIn("cancelled 1 session(s) for 'strandz'", self.messages)

    def test_stale_pidfile_is_not_signalled(self):
        pidfile = batch.write_detached_pidfile(
            'strandz', 9999, 'old-run', state_dir=self.base)
        with mock.patch.object(batch, 'find_detached_sessions', return_value=[]), \
                mock.patch.object(batch.os, 'killpg',
                                  side_effect=AssertionError('must not signal')), \
                mock.patch.object(batch.os, 'kill',
                                  side_effect=ProcessLookupError):
            ok = batch.cancel_catalog_session(
                'strandz', state_dir=self.base, say=self.say,
                proc_root=self.proc_root, timeout=0)

        self.assertFalse(ok)
        self.assertFalse(pidfile.exists())
        self.assertTrue(any('no detached catalog session found' in m
                            for m in self.messages))

    def test_legacy_detached_session_is_found_by_artist(self):
        self.add_proc(5555, self.catalog_args())
        calls = []

        with mock.patch.object(batch.os, 'getpgid', return_value=5555), \
                mock.patch.object(batch.os, 'getsid', return_value=5555), \
                mock.patch.object(batch.os, 'killpg',
                                  side_effect=lambda pgid, sig: calls.append((pgid, sig))), \
                mock.patch.object(batch, 'process_group_alive', return_value=False):
            ok = batch.cancel_catalog_session(
                'STRANDZ', state_dir=self.base, say=self.say,
                proc_root=self.proc_root, timeout=0)

        self.assertTrue(ok)
        self.assertEqual(calls, [(5555, signal.SIGTERM)])
        self.assertTrue(any("cancelling detached catalog session for 'strandz'"
                            in m for m in self.messages))

    def test_foreground_session_is_not_cancelled(self):
        self.add_proc(6666, self.catalog_args())
        with mock.patch.object(batch.os, 'getpgid', return_value=100), \
                mock.patch.object(batch.os, 'getsid', return_value=100), \
                mock.patch.object(batch.os, 'killpg',
                                  side_effect=AssertionError('must not signal')):
            ok = batch.cancel_catalog_session(
                'strandz', state_dir=self.base, say=self.say,
                proc_root=self.proc_root, timeout=0)

        self.assertFalse(ok)
        self.assertTrue(any('no detached catalog session found' in m
                            for m in self.messages))

    def test_spawn_records_pidfile_and_marks_child(self):
        with mock.patch.object(
                batch.subprocess, 'Popen',
                return_value=SimpleNamespace(pid=7777)) as popen:
            proc, run_id = batch.spawn_detached_process(
                ['python', 'script', '--artist', 'Strandz'], 'Strandz',
                logfile=self.base / 'strandz.log', state_dir=self.base)

        self.assertEqual(proc.pid, 7777)
        kwargs = popen.call_args.kwargs
        self.assertTrue(kwargs['start_new_session'])
        cmd = popen.call_args.args[0]
        self.assertEqual(cmd[-4:], ['--run-id', run_id,
                                    '--detached-run', 'strandz'])
        record = json.loads(
            (self.base / 'strandz.catalog.pid').read_text(encoding='utf-8'))
        self.assertEqual(record, {'pid': 7777, 'pgid': 7777,
                                  'slug': 'strandz', 'run_id': run_id})

    def test_detached_child_claims_its_pidfile(self):
        args = SimpleNamespace(detached_run='Strandz', run_id='run-9',
                               state=str(self.base / 'state.json'))
        with mock.patch.object(batch, 'set_process_title') as set_title, \
                mock.patch.object(batch.atexit, 'register') as register:
            batch.register_detached_worker(args)

        set_title.assert_called_once_with('Strandz')
        register.assert_called_once()
        record = json.loads(
            (self.base / 'strandz.catalog.pid').read_text(encoding='utf-8'))
        self.assertEqual(record['pid'], batch.os.getpid())
        self.assertEqual(record['run_id'], 'run-9')

    def test_sigterm_cleans_registered_temp_dirs_before_exit(self):
        work = self.base / '.download-test'
        work.mkdir()
        (work / 'partial.opus').write_bytes(b'incomplete')
        batch.register_temp_dir(work)
        self.addCleanup(lambda: batch.unregister_temp_dir(work))
        handlers = {}
        self.addCleanup(batch.ABORT.clear)

        with mock.patch.object(batch.signal, 'signal',
                               side_effect=lambda sig, fn: handlers.setdefault(sig, fn)), \
                mock.patch.object(batch, 'kill_all_children'), \
                mock.patch.object(batch.os, '_exit', side_effect=SystemExit):
            batch.install_signal_handlers(SimpleNamespace())
            with self.assertRaises(SystemExit):
                handlers[signal.SIGTERM](signal.SIGTERM, None)

        self.assertFalse(work.exists())

    def test_cancel_reaps_stale_artist_temp_dirs(self):
        root = self.base / 'library' / 'SL'
        tmp = root / '.sonic-tmp'
        stale = tmp / 'run-999999-old'
        stale.mkdir(parents=True)
        (stale / 'partial.opus').write_bytes(b'incomplete')
        work = root / '.download-old'
        work.mkdir(parents=True)

        with mock.patch.object(batch, 'resolve_root', return_value=root), \
                mock.patch.object(batch, 'find_detached_sessions', return_value=[]):
            removed = batch.cleanup_artist_temp_after_cancel('sl', say=self.say)

        self.assertEqual(removed, 2)
        self.assertFalse(stale.exists())
        self.assertFalse(work.exists())
        self.assertTrue(any('removed 1 stale temp dir(s)' in m
                            for m in self.messages))
        self.assertTrue(any('removed 1 stale download dir(s)' in m
                            for m in self.messages))

    def test_cancel_is_a_standalone_mode(self):
        saved_argv = sys.argv
        self.addCleanup(setattr, sys, 'argv', saved_argv)
        argv = ['get-catalog', '--cancel', 'strandz']
        with mock.patch.object(sys, 'argv', argv), \
                mock.patch.object(batch, 'cancel_catalog_session',
                                  return_value=True) as cancel, \
                mock.patch.object(batch, 'cleanup_artist_temp_after_cancel') as cleanup:
            batch.main()

        cancel.assert_called_once()
        cleanup.assert_called_once()
        self.assertEqual(cancel.call_args.args[0], 'strandz')

    def test_cancel_returns_failure_exit_code_when_not_found(self):
        saved_argv = sys.argv
        self.addCleanup(setattr, sys, 'argv', saved_argv)
        argv = ['get-catalog', '--cancel', 'nobody']
        with mock.patch.object(sys, 'argv', argv), \
                mock.patch.object(batch, 'cancel_catalog_session',
                                  return_value=False), \
                mock.patch.object(batch, 'cleanup_artist_temp_after_cancel'):
            with self.assertRaises(SystemExit) as raised:
                batch.main()
        self.assertEqual(raised.exception.code, 1)


if __name__ == '__main__':
    unittest.main()
