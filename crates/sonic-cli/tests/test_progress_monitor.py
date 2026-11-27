#!/usr/bin/env python3
"""Regression tests for get-catalog's in-flight byte monitor.

The monitor used to hard-code /tmp/sonic-download-*, but get-catalog points
TMPDIR at the album's temporary folder (often on an external drive). Progress
rows then stayed at 0B until task_end() credited the whole album at once.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import os
import sys
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

EXTERNAL = '/path/to/music/library/ARTIST/.sonic-tmp/run-1'
FOLDER = EXTERNAL + '/sonic-download-ABC'
PART = FOLDER + '/audio.webm.part'
ENV = (f'SONIC_BATCH_TOKEN=tok\0TMPDIR={EXTERNAL}\0HOME=/path/to/home\0').encode()


class ProgressMonitorTest(unittest.TestCase):
    def test_folder_from_target_handles_external_tmpdir_with_spaces(self):
        self.assertEqual(batch.TempByteMonitor.folder_from_target(PART), FOLDER)

    def test_folder_from_target_ignores_unrelated_descriptors(self):
        self.assertIsNone(
            batch.TempByteMonitor.folder_from_target('/tmp/other/audio.webm.part'))

    def test_tmp_root_is_read_from_marked_process(self):
        self.assertEqual(batch.TempByteMonitor.tmp_root(ENV), EXTERNAL)
        self.assertIsNone(batch.TempByteMonitor.tmp_root(b'HOME=/path/to/home\0'))

    def _monitor(self):
        monitor = batch.TempByteMonitor('tok')
        self.assertEqual(monitor.roots, set())
        return monitor

    def test_register_checks_batch_token_before_trusting_tmpdir(self):
        monitor = self._monitor()
        foreign = b'SONIC_BATCH_TOKEN=other\0TMPDIR=/path/to/other\0'
        self.assertFalse(monitor.register(999, env=foreign))
        self.assertEqual(monitor.registered, {})
        self.assertEqual(monitor.roots, set())

    def test_open_part_descriptor_is_counted_outside_tmp(self):
        monitor = self._monitor()
        self.assertTrue(monitor.register(123, env=ENV))
        ctime = time.time()

        def listdir(path):
            if path == '/proc/123/fd':
                return ['7']
            if path == FOLDER:
                return ['audio.webm.part']
            return []

        real_stat = os.stat
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(batch.os, 'listdir', listdir))
            stack.enter_context(mock.patch.object(batch.os, 'readlink',
                                                  return_value=PART))
            stack.enter_context(mock.patch.object(
                batch.os, 'stat',
                side_effect=lambda p: (SimpleNamespace(st_ctime=ctime)
                                       if p == FOLDER else real_stat(p))))
            stack.enter_context(mock.patch.object(batch.os.path, 'getsize',
                                                  return_value=8192))
            self.assertEqual(monitor._dirs(), {FOLDER})
            self.assertEqual(monitor.roots, {EXTERNAL})
            self.assertEqual(monitor.sample(), 8192)
            self.assertEqual(monitor.file_sizes(), [8192])

    def test_fallback_registers_dirs_from_external_tmpdir(self):
        """No open descriptor during conversion must not lose progress."""
        monitor = self._monitor()
        self.assertTrue(monitor.register(123, env=ENV))
        ctime = time.time()

        def listdir(path):
            if path == '/proc/123/fd':
                return []
            if path == EXTERNAL:
                return ['sonic-download-ABC']
            if path == FOLDER:
                return ['audio.webm.part']
            return []

        real_stat = os.stat
        with ExitStack() as stack:
            stack.enter_context(mock.patch.object(batch.os, 'listdir', listdir))
            stack.enter_context(mock.patch.object(
                batch.os, 'stat',
                side_effect=lambda p: (SimpleNamespace(st_ctime=ctime,
                                                       st_uid=os.getuid())
                                       if p == FOLDER else real_stat(p))))
            stack.enter_context(mock.patch.object(batch.os.path, 'getsize',
                                                  return_value=4096))
            self.assertEqual(monitor._dirs(), {FOLDER})
            self.assertIn(FOLDER, monitor.created)
            self.assertEqual(monitor.sample(), 4096)
            self.assertEqual(monitor.file_sizes(), [4096])


if __name__ == '__main__':
    unittest.main()
