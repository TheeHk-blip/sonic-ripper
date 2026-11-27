#!/usr/bin/env python3
"""Regression tests for credited-artist tagging in the batch runner."""
import importlib.util
import sys
import unittest
from pathlib import Path

SRC = Path(__file__).resolve().parents[1] / 'src'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


batch = load('batch_tag_policy', SRC / 'ytm-artist-batch.py')


class CreditedArtistTagTest(unittest.TestCase):
    def test_album_updates_leave_artist_to_sonic_core(self):
        updates = batch.album_updates('album', '2023', True)
        self.assertEqual(updates['RELEASE_TYPE'], 'ALBUM')
        self.assertEqual(updates['MUSICBRAINZ_ALBUMTYPE'], 'Album')
        self.assertEqual(updates['date'], '2023')
        self.assertEqual(updates['EXPLICIT'], '1')
        self.assertNotIn('artist', updates)
        self.assertNotIn('ALBUMARTIST', updates)

    def test_single_updates_leave_artist_to_sonic_core(self):
        updates = batch.single_updates('On the Radar Freestyle', True)
        self.assertEqual(updates['title'], 'On the Radar Freestyle')
        self.assertEqual(updates['RELEASE_TYPE'], 'SINGLE')
        self.assertEqual(updates['EXPLICIT'], '1')
        self.assertNotIn('artist', updates)
        self.assertNotIn('ALBUMARTIST', updates)

    def test_tag_flag_keeps_comma_separated_artist_value(self):
        self.assertEqual(
            batch.tag_flags({'artist': 'Drake, Central Cee'}),
            ['--tag=artist=Drake, Central Cee'])

    def test_performer_hints_are_hidden_flags_and_deduplicated(self):
        self.assertEqual(
            batch.performer_hint_flags(['Central Cee', 'central cee', 'Drake']),
            ['--performer-hint=Central Cee', '--performer-hint=Drake'])


if __name__ == '__main__':
    unittest.main()
