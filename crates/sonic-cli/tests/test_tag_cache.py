#!/usr/bin/env python3
"""Regression tests for the per-file ffprobe tag cache."""
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'
spec = importlib.util.spec_from_file_location(
    'batch_tag_cache_test', SRC / 'ytm-artist-batch.py')
batch = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = batch
spec.loader.exec_module(batch)

PROBE = subprocess.CompletedProcess([], 0, json.dumps({
    'streams': [{'tags': {'TITLE': 'Track One', 'DATE': '2021'}}],
    'format': {'tags': {'TITLE': 'Format Wins Never'}},
}), '')


class TagCacheTest(unittest.TestCase):
    def setUp(self):
        batch.TAG_CACHE.clear()
        self.addCleanup(batch.TAG_CACHE.clear)

    def test_title_and_date_share_one_ffprobe(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'track.opus'
            path.write_bytes(b'audio')
            with mock.patch.object(batch, 'run', return_value=PROBE) as run:
                self.assertEqual(batch.tag(path, 'title'), 'Track One')
                self.assertEqual(batch.tag(path, 'date'), '2021')
            self.assertEqual(run.call_count, 1)
            fields = run.call_args.args[0]
            self.assertEqual(fields[0], 'ffprobe')
            self.assertIn('stream_tags=title,date', fields[4])

    def test_changed_file_is_reprobed(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'track.opus'
            path.write_bytes(b'audio')
            with mock.patch.object(batch, 'run', return_value=PROBE) as run:
                batch.tag(path, 'title')
                os.utime(path, ns=(path.stat().st_atime_ns,
                                   path.stat().st_mtime_ns + 1000))
                batch.tag(path, 'title')
            self.assertEqual(run.call_count, 2)

    def test_retag_invalidates_cached_tags(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'track.opus'
            path.write_bytes(b'audio')
            ok = subprocess.CompletedProcess([], 0, '', '')
            with mock.patch.object(batch, 'run', side_effect=[PROBE, ok, PROBE]) as run:
                batch.tag(path, 'title')
                batch.retag(path, {'date': '2022'})
                batch.tag(path, 'title')
            self.assertEqual(run.call_count, 3)


if __name__ == '__main__':
    unittest.main()
