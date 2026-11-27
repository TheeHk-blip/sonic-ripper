#!/usr/bin/env python3
"""Tests for the pre-download checkbox picker."""
import importlib.util
import io
import sys
import unittest
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


picker = load('checkbox_select_test', SRC / 'checkbox_select.py')
batch = load('batch_checkbox_test', SRC / 'ytm-artist-batch.py')


class SelectionStateTest(unittest.TestCase):
    def setUp(self):
        self.choices = [
            picker.Choice('ALBUM Burnin', 'a0', group='album'),
            picker.Choice('ALBUM Live at Leeds', 'a1', group='album'),
            picker.Choice('SINGLE Stir It Up', 's0', group='single'),
        ]
        self.state = picker.SelectionState(self.choices)

    def test_all_none_and_space_toggle(self):
        self.assertEqual(self.state.count, 3)
        self.assertEqual(self.state.handle('n'), None)
        self.assertEqual(self.state.count, 0)
        self.assertEqual(self.state.handle('space'), None)
        self.assertTrue(self.choices[0].selected)
        self.assertEqual(self.state.selected, [self.choices[0]])

    def test_a_restores_all_and_group_toggles(self):
        self.state.handle('n')
        self.state.handle('l')
        self.assertTrue(self.choices[0].selected)
        self.assertTrue(self.choices[1].selected)
        self.assertFalse(self.choices[2].selected)
        self.state.handle('s')
        self.assertTrue(self.choices[2].selected)
        self.state.handle('a')
        self.assertEqual(self.state.count, 3)

    def test_arrows_wrap_and_confirm_cancel(self):
        self.assertEqual(self.state.handle('up'), None)
        self.assertEqual(self.state.index, 2)
        self.assertEqual(self.state.handle('down'), None)
        self.assertEqual(self.state.index, 0)
        self.assertEqual(self.state.handle('enter'), 'confirm')
        self.assertEqual(self.state.handle('q'), 'cancel')

    def test_render_shows_marks_and_selection_count(self):
        self.state.handle('n')
        lines = picker.render(self.state, width=60, height=12)
        self.assertIn('0/3 selected', lines[1])
        self.assertTrue(any('[ ] ALBUM Burnin' in line for line in lines))
        self.state.handle('space')
        lines = picker.render(self.state, width=60, height=12)
        self.assertIn('1/3 selected', lines[1])
        self.assertTrue(any('[✓] ALBUM Burnin' in line for line in lines))


class RunModeTest(unittest.TestCase):
    def test_enter_defaults_to_foreground(self):
        out = io.StringIO()
        with mock.patch('sys.stdin', io.StringIO('\n')):
            self.assertEqual(
                picker.choose_run_mode(input_stream=sys.stdin, output_stream=out),
                'foreground')

    def test_d_chooses_detached(self):
        out = io.StringIO()
        self.assertEqual(
            picker.choose_run_mode(input_stream=io.StringIO('d\n'),
                                   output_stream=out),
            'detached')

    def test_q_cancels(self):
        with self.assertRaises(picker.SelectionCancelled):
            picker.choose_run_mode(input_stream=io.StringIO('q\n'),
                                   output_stream=io.StringIO())


class CatalogFilterTest(unittest.TestCase):
    def test_picked_indexes_map_back_to_albums_and_singles(self):
        albums = [('1973', 'Burnin', ''), ('1975', 'Live at Leeds', '')]
        singles = [{'title': 'Stir It Up'}, {'title': 'Get Up, Stand Up'}]
        picked = [
            picker.Choice('ALBUM Burnin', ('album', 0)),
            picker.Choice('SINGLE Get Up, Stand Up', ('single', 1)),
        ]
        with mock.patch.object(batch, 'select_items', return_value=picked), \
                mock.patch.object(batch, 'choose_run_mode',
                                  return_value='detached'):
            kept_albums, kept_singles, mode = batch.interactive_catalog_filter(
                albums, singles, 'Bob Marley')
        self.assertEqual(kept_albums, [albums[0]])
        self.assertEqual(kept_singles, [singles[1]])
        self.assertEqual(mode, 'detached')


if __name__ == '__main__':
    unittest.main()
