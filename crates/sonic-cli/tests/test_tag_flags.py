#!/usr/bin/env python3
"""Regression tests for metadata flags passed to sonic-cli.

A Lucky Dube track id is `-BOc6-sh_IU`. Passing it as the separate value after
`--track-tag` makes clap read `-B` as an unknown short option, so the whole
album download fails. The equals form keeps such values unambiguous.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import sys
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / 'src' / 'ytm-artist-batch.py'
spec = importlib.util.spec_from_file_location('ytm_artist_batch', SCRIPT)
batch = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = batch
spec.loader.exec_module(batch)


class TagFlagsTest(unittest.TestCase):
    def test_track_id_starting_with_dash(self):
        self.assertEqual(
            batch.track_tag_flags('-BOc6-sh_IU', {'EXPLICIT': '0'}),
            ['--track-tag=-BOc6-sh_IU=EXPLICIT=0'])

    def test_multiple_track_tags(self):
        self.assertEqual(
            batch.track_tag_flags('normal-id', {'EXPLICIT': '1',
                                                'ITUNESADVISORY': '1'}),
            ['--track-tag=normal-id=EXPLICIT=1',
             '--track-tag=normal-id=ITUNESADVISORY=1'])

    def test_none_values_are_skipped(self):
        self.assertEqual(batch.track_tag_flags('id', {'EXPLICIT': None}), [])
        self.assertEqual(batch.tag_flags({'date': None}), [])

    def test_tag_flags_use_equals_form(self):
        self.assertEqual(batch.tag_flags({'date': '2013'}), ['--tag=date=2013'])

    def test_no_flag_value_looks_like_an_option(self):
        flags = batch.tag_flags({'artist': 'Lucky Dube'})
        flags += batch.track_tag_flags('-BOc6-sh_IU', {'EXPLICIT': '0'})
        for token in flags:
            self.assertTrue(token.startswith('--tag=')
                            or token.startswith('--track-tag='))
            self.assertIn('=', token.split('=', 1)[1])


if __name__ == '__main__':
    unittest.main()
