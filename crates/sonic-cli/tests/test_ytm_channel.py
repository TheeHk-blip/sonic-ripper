#!/usr/bin/env python3
"""Regression tests for --ytm-channel id/handle resolution."""
import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'
spec = importlib.util.spec_from_file_location(
    'batch_ytm_channel_test', SRC / 'ytm-artist-batch.py')
batch = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = batch
spec.loader.exec_module(batch)

UC_ID = 'UC0123456789abcdefghijkl'


class ChannelArgumentTest(unittest.TestCase):
    def test_bare_uc_id_needs_no_network(self):
        with mock.patch.object(batch, 'run',
                               side_effect=AssertionError('must not resolve')):
            self.assertEqual(batch.resolve_channel_argument(UC_ID), UC_ID)
            self.assertEqual(
                batch.resolve_channel_argument(
                    f'https://music.youtube.com/channel/{UC_ID}'), UC_ID)

    def test_music_handle_url_is_rewritten_to_www(self):
        result = subprocess.CompletedProcess([], 0, f'{UC_ID}\n', '')
        notes = []
        with mock.patch.object(batch, 'run', return_value=result) as run:
            found = batch.resolve_channel_argument(
                'https://music.youtube.com/@Artist',
                note=lambda message, **_kwargs: notes.append(message))

        self.assertEqual(found, UC_ID)
        self.assertEqual(run.call_args.args[0][-1],
                         'https://www.youtube.com/@Artist')
        self.assertIn(f"resolved 'https://music.youtube.com/@Artist' "
                      f"to {UC_ID}", notes[0])

    def test_bare_handle_is_accepted(self):
        result = subprocess.CompletedProcess([], 0, f'{UC_ID}\n', '')
        with mock.patch.object(batch, 'run', return_value=result) as run:
            found = batch.resolve_channel_argument('@Artist')

        self.assertEqual(found, UC_ID)
        self.assertEqual(run.call_args.args[0][-1],
                         'https://www.youtube.com/@Artist')

    def test_non_youtube_text_is_rejected_without_network(self):
        with mock.patch.object(batch, 'run',
                               side_effect=AssertionError('must not resolve')):
            self.assertIsNone(batch.resolve_channel_argument('Artist'))

    def test_failed_resolution_is_reported(self):
        result = subprocess.CompletedProcess([], 1, '', 'no such channel')
        notes = []
        with mock.patch.object(batch, 'run', return_value=result):
            found = batch.resolve_channel_argument(
                '@nobody', note=lambda message, **_kwargs: notes.append(message))

        self.assertIsNone(found)
        self.assertIn("resolution failed for '@nobody' (no such channel)",
                      notes[0])


if __name__ == '__main__':
    unittest.main()
