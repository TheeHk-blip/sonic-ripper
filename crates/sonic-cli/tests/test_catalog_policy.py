#!/usr/bin/env python3
"""Regression tests for the Wikipedia-reference/YTM-source catalog policy.

Wikipedia proposes studio albums and singles; the YTM artist index decides what
exists. Wiki-only titles must never trigger the search resolver. If Wikipedia
has no list, every indexed YTM release is used instead.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import json
import sys
import tempfile
import unittest
import urllib.error
from pathlib import Path
from unittest import mock

SRC = Path(__file__).resolve().parents[1] / 'src'


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


batch = load('batch_catalog_policy', SRC / 'ytm-artist-batch.py')
wiki = load('wiki_catalog_policy', SRC / 'wiki_discovery.py')

ARTIST = 'Test Artist'
INDEX = {
    'albums': {
        'deep down': {'title': 'Deep Down', 'year': '1975', 'type': 'album'},
        'judge not': {'title': 'Judge Not', 'year': '1981', 'type': 'album'},
        'one single': {'title': 'One Single', 'year': '1982', 'type': 'single'},
    },
    'tracks': {
        'deep down': {'title': 'Deep Down'},
        'judge not': {'title': 'Judge Not'},
        'one single': {'title': 'One Single'},
    },
}
DUB_INDEX = {
    'albums': {
        'deep down': {'title': 'Deep Down', 'year': '1975', 'type': 'album'},
        'die hard dubwise': {'title': 'Die Hard Dubwise', 'year': '1991',
                             'type': 'album'},
        'brain drop riddim': {'title': 'Brain Drop Riddim', 'year': '2026',
                              'type': 'single'},
        'one single': {'title': 'One Single', 'year': '1982', 'type': 'single'},
    },
    'tracks': {
        'deep down': {'title': 'Deep Down'},
        'die hard dubwise': {'title': 'Die Hard Dubwise'},
        'brain drop riddim': {'title': 'Brain Drop Riddim'},
        'one single': {'title': 'One Single'},
    },
}
MEMBER_INDEX = {
    'albums': {
        'deep down': {'title': 'Deep Down', 'year': '1975', 'type': 'album',
                      'browseId': 'b-album'},
        'its magic': {'title': "It's Magic", 'year': '2026', 'type': 'single',
                      'videoId': 'v-single'},
        'we three kings': {'title': 'We Three Kings', 'year': '2026',
                           'type': 'single', 'videoId': 'v-other'},
    },
    'tracks': {
        'deep down': {'title': 'Deep Down', 'videoId': 'v-album'},
        'its magic': {'title': "It's Magic", 'videoId': 'v-single'},
        'we three kings': {'title': 'We Three Kings', 'videoId': 'v-other'},
        'track in album': {'title': 'Track In Album', 'videoId': 'v-track'},
        'alias track': {'title': 'Alias Track', 'videoId': 'v-different'},
    },
    'members': {
        'deep down': [
            {'videoId': 'v-track', 'title': 'Track In Album'},
            {'videoId': 'v-alias', 'title': 'Alias Track'},
        ],
    },
    'videos': {},
    'memberTitles': {},
}
REISSUE_INDEX = {
    'albums': {
        'deep down': {'title': 'Deep Down', 'year': '1975', 'type': 'album'},
        'inseparable remastered edition': {
            'title': 'Inseparable: Remastered Edition', 'year': '2024',
            'type': 'album'},
        'gold 50th anniversary edition': {
            'title': 'Gold: 50th Anniversary Edition', 'year': '2020',
            'type': 'album'},
        'die hard special edition': {
            'title': 'Die Hard - Special Edition', 'year': '1996',
            'type': 'album'},
        'satisfaction feeling': {
            'title': 'Satisfaction Feeling (Deluxe Edition)', 'year': '2020',
            'type': 'album'},
        'special occasion': {'title': 'Special Occasion', 'year': '1988',
                             'type': 'album'},
        'old hit 2020 remaster': {'title': 'Old Hit (2020 Remaster)',
                                  'year': '2020', 'type': 'single'},
        'one single': {'title': 'One Single', 'year': '1982', 'type': 'single'},
    },
    'tracks': {
        'deep down': {'title': 'Deep Down'},
        'old hit 2020 remaster': {'title': 'Old Hit (2020 Remaster)'},
        'one single': {'title': 'One Single'},
    },
}


class CatalogPolicyTest(unittest.TestCase):
    def test_auto_inputs_announces_missing_wikipedia_cache(self):
        with tempfile.TemporaryDirectory() as tmp:
            notes = []
            with mock.patch.object(batch, 'fetch_wiki_albums', return_value=[]), \
                    mock.patch.object(batch, 'fetch_wiki_singles', return_value=[]):
                batch.auto_inputs(
                    'Sl', Path(tmp),
                    lambda message, **_kwargs: notes.append(message))

        self.assertIn("Wikipedia cache missing; discovering 'Sl discography'",
                      notes[0])

    def test_wiki_albums_are_filtered_to_exact_ytm_matches(self):
        wiki_albums = [('1975', 'Deep Down', ''),
                       ('1975', 'Wiki Only Album', ''),
                       ('1981', 'Judge Not', '')]
        notes = []
        albums, singles = batch.catalog_from_wiki(
            INDEX, ARTIST, wiki_albums, [],
            lambda message, **_kwargs: notes.append(message))
        self.assertEqual(albums, wiki_albums[::2])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])
        self.assertIn('0 dub/re-release and 1 wiki-only title(s)', notes[0])

    def test_no_wiki_album_list_uses_full_ytm_catalog(self):
        albums, singles = batch.catalog_from_wiki(INDEX, ARTIST, [], [])
        self.assertEqual(albums, [('1975', 'Deep Down', ''),
                                  ('1981', 'Judge Not', '')])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])

    def test_wiki_list_with_no_exact_ytm_match_falls_back_to_index(self):
        notes = []
        albums, singles = batch.catalog_from_wiki(
            INDEX, ARTIST,
            [('1999', 'Rah Digga Album', '')],
            [{'title': 'Rah Digga Single', 'year': '1999'}],
            lambda message, **_kwargs: notes.append(message))
        self.assertEqual(albums, [('1975', 'Deep Down', ''),
                                  ('1981', 'Judge Not', '')])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])
        self.assertIn('no exact YTM album match', '\n'.join(notes))
        self.assertIn('no exact YTM single match', '\n'.join(notes))

    def test_wiki_singles_are_filtered_and_wiki_only_ignored(self):
        wiki_singles = [{'title': 'One Single', 'year': '1982'},
                        {'title': 'Wiki Only Single', 'year': '1982'}]
        _, singles = batch.catalog_from_wiki(
            INDEX, ARTIST, [('1975', 'Deep Down', '')], wiki_singles)
        self.assertEqual(singles, [wiki_singles[0]])

    def test_no_wiki_single_list_uses_ytm_singles(self):
        _, singles = batch.catalog_from_wiki(
            INDEX, ARTIST, [('1975', 'Deep Down', '')], [])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])

    def test_dub_titles_are_recognised(self):
        for title in ('Die Hard Dubwise', 'Brain Drop Riddim',
                      'Version Excursion', 'Instrumentals', 'Dub Plate'):
            self.assertTrue(batch.is_dub_release(title), title)
        for title in ('Deep Down', 'Wolf & Leopards', 'Dublin', 'Dubbing'):
            self.assertFalse(batch.is_dub_release(title), title)

    def test_fallback_skips_dub_and_riddim_releases(self):
        notes = []
        albums, singles = batch.catalog_from_wiki(
            DUB_INDEX, ARTIST, [], [],
            lambda message, **_kwargs: notes.append(message))
        self.assertEqual(albums, [('1975', 'Deep Down', '')])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])
        self.assertIn('skipping 1 dub/re-release(s)', notes[0])

    def test_wiki_dub_title_is_skipped_before_ytm_match(self):
        notes = []
        wiki_albums = [('1975', 'Deep Down', ''),
                       ('1991', 'Die Hard Dubwise', '')]
        albums, _ = batch.catalog_from_wiki(
            DUB_INDEX, ARTIST, wiki_albums, [],
            lambda message, **_kwargs: notes.append(message))
        self.assertEqual(albums, [wiki_albums[0]])
        self.assertIn('1 dub/re-release and 0 wiki-only title(s)', notes[0])

    def test_rerelease_titles_are_recognised(self):
        for title in ('Inseparable: Remastered Edition',
                      'Gold: 50th Anniversary Edition',
                      'Old Hit (2020 Reissue)', 'Love (Re-Recorded)'):
            self.assertTrue(batch.is_rerelease(title), title)
        for title in ('Special Occasion', 'Satisfaction Feeling (Deluxe Edition)',
                      'Deluxe Edition', 'Die Hard - Special Edition',
                      'Limited Edition', "Collector's Edition"):
            self.assertFalse(batch.is_rerelease(title), title)

    def test_fallback_skips_anniversary_and_remastered_releases(self):
        notes = []
        albums, singles = batch.catalog_from_wiki(
            REISSUE_INDEX, ARTIST, [], [],
            lambda message, **_kwargs: notes.append(message))
        self.assertEqual([a[1] for a in albums],
                         ['Deep Down', 'Die Hard - Special Edition',
                          'Satisfaction Feeling (Deluxe Edition)',
                          'Special Occasion'])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])
        self.assertIn('skipping 2 dub/re-release(s)', notes[0])
        self.assertIn('skipping 1 dub/re-release(s)', notes[1])

    def test_index_rerelease_cannot_match_wiki_original(self):
        """A failed wiki match falls back to the index, never to the remaster."""
        notes = []
        albums, _ = batch.catalog_from_wiki(
            REISSUE_INDEX, ARTIST, [('1989', 'Inseparable', '')], [],
            lambda message, **_kwargs: notes.append(message))
        titles = [a[1] for a in albums]
        self.assertNotIn('Inseparable: Remastered Edition', titles)
        self.assertNotIn('Gold: 50th Anniversary Edition', titles)
        self.assertIn('no exact YTM album match', '\n'.join(notes))

    def test_deluxe_edition_can_match_wiki_original(self):
        """Deluxe is the only indexed copy of some original albums."""
        albums, _ = batch.catalog_from_wiki(
            REISSUE_INDEX, ARTIST, [('1983', 'Satisfaction Feeling', '')], [])
        self.assertEqual(albums, [('1983', 'Satisfaction Feeling', '')])

    def test_cached_wiki_lists_are_loaded_without_network(self):
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp) / batch.slug(ARTIST)
            work.mkdir()
            (work / 'albums.tsv').write_text('1975\tDeep Down\n', encoding='utf-8')
            (work / 'singles.json').write_text(
                json.dumps([{'title': 'One Single', 'year': '1982'}]),
                encoding='utf-8')
            with mock.patch.object(batch, 'fetch_wiki_albums',
                                   side_effect=AssertionError('network used')):
                albums, singles = batch.wiki_reference_lists(ARTIST, Path(tmp))
        self.assertEqual(albums, [('1975', 'Deep Down', '')])
        self.assertEqual(singles, [{'title': 'One Single', 'year': '1982'}])


class DuplicateFilterTest(unittest.TestCase):
    def test_index_members_and_disk_album_tracks_are_removed_before_queue(self):
        notes = []
        singles = [{'title': 'Track In Album'},
                   {'title': 'Alias Track'},
                   {'title': "It's Magic"},
                   {'title': 'We Three Kings'}]
        kept = batch.dedupe_singles_for_albums(
            MEMBER_INDEX, ARTIST, [('1975', 'Deep Down', '')], singles,
            disk_titles={batch.norm("It's Magic")},
            log=lambda message, **_kwargs: notes.append(message))
        self.assertEqual(kept, [{'title': 'We Three Kings'}])
        self.assertIn('removed 3 single(s) already in album(s)', notes[0])
        self.assertIn("  SKIP duplicate single: It's Magic (already in an album folder)",
                      notes)

    def test_songs_mode_keeps_requested_tracks(self):
        singles = [{'title': 'Track In Album'}]
        kept = batch.dedupe_singles_for_albums(
            MEMBER_INDEX, ARTIST, [('1975', 'Deep Down', '')], singles,
            song_titles=True)
        self.assertEqual(kept, singles)


class WikiScraperTest(unittest.TestCase):
    def setUp(self):
        wiki.discography_search.cache_clear()

    def tearDown(self):
        wiki.discography_search.cache_clear()

    def test_rate_limit_wait_is_short_and_retried_once(self):
        error = urllib.error.HTTPError(
            'https://en.wikipedia.org/w/api.php', 429, 'Too many requests',
            {'Retry-After': '60'}, None)
        self.addCleanup(error.close)
        sleeps = []
        calls = []

        def fake_urlopen(_req, timeout=None):
            calls.append(timeout)
            raise error

        with mock.patch.object(wiki.urllib.request, 'urlopen',
                               side_effect=fake_urlopen), \
                mock.patch.object(wiki.time, 'sleep',
                                  side_effect=lambda seconds: sleeps.append(seconds)):
            with self.assertRaises(urllib.error.HTTPError):
                wiki.api({'action': 'parse'}, tries=5)

        self.assertEqual(len(calls), 2)
        self.assertEqual(sleeps, [wiki.MAX_RATE_LIMIT_WAIT])

    def test_search_never_accepts_unrelated_artist_page(self):
        payload = {'query': {'search': [
            {'title': 'Cutty Ranks'},
            {'title': 'Shaggy discography'},
        ]}}
        with mock.patch.object(wiki, 'api', return_value=payload):
            hits = wiki.discography_search('Cutty Ranks discography')
        self.assertEqual(hits, ['Cutty Ranks'])

    def test_search_rejects_name_spanning_next_word(self):
        payload = {'query': {'search': [
            {'title': 'Rah Digga discography'},
            {'title': 'Digga D'},
        ]}}
        with mock.patch.object(wiki, 'api', return_value=payload):
            hits = wiki.discography_search('Digga D discography')
        self.assertEqual(hits, ['Digga D'])

    def test_wikipedia_title_matching_is_exact_after_normalization(self):
        self.assertTrue(wiki.artist_title_matches('Digga D', 'Digga D'))
        self.assertTrue(wiki.artist_title_matches('Digga D discography', 'Digga D'))
        self.assertTrue(wiki.artist_title_matches('Digga D (rapper)', 'Digga D'))
        self.assertTrue(wiki.artist_title_matches('Sade and Adu', 'Sade & Adu'))
        self.assertFalse(wiki.artist_title_matches('Rah Digga discography', 'Digga D'))
        self.assertFalse(wiki.artist_title_matches('Digga D (rapper) remixes', 'Digga D'))

    def test_section_stops_at_prefix_of_real_heading_id(self):
        html = ('<h2 id="Studio_albums">Studio albums</h2><ul><li>1975 - Deep Down'
                '</li></ul><h2 id="Posthumous_releases_and_compilations">Later'
                '</h2><table><tr><td>Authority</td></tr></table>')
        sec = wiki.section(html, 'Studio_albums', ('Posthumous',))
        self.assertIn('Deep Down', sec)
        self.assertNotIn('Authority', sec)

    def test_album_from_line_cleans_labels_and_reissues(self):
        self.assertEqual(
            wiki.album_from_line('1975 – Deep Down (Observer), reissued in 1979 '
                                 'as So Long Rastafari (Harry J)'),
            {'year': '1975', 'title': 'Deep Down'})
        self.assertEqual(
            wiki.album_from_line('1977 – Dennis Brown Meets Harry Hippy '
                                 '(Pioneer)(with Jackie Robinson (musician))'),
            {'year': '1977', 'title': 'Dennis Brown Meets Harry Hippy'})
        self.assertIsNone(wiki.album_from_line('not an album row'))

    def test_bullet_style_discography_is_parsed(self):
        html = ('<h2 id="Studio_albums">Studio albums</h2><div class="div-col">'
                '<ul><li>1970 – <i><a href="/wiki/No_Man_Is_an_Island_(album)">'
                'No Man Is an Island</a></i> (Studio One)</li>'
                '<li>1975 – Deep Down (Observer)</li></ul></div>'
                '<h2 id="Posthumous_releases_and_compilations">Later</h2>')
        with mock.patch.object(wiki, 'wiki_html', return_value=html):
            albums = wiki.albums('Test discography')
        self.assertEqual(albums, [
            {'year': '1970', 'title': 'No Man Is an Island'},
            {'year': '1975', 'title': 'Deep Down'}])

    def test_table_style_discography_still_parsed(self):
        html = ('<h2 id="Studio_albums">Studio albums</h2>'
                '<table><tr><th>Title</th><th>Year</th></tr>'
                '<tr><td>Diamond Life [nb 1]</td><td>1984</td></tr></table>'
                '<h2 id="Collaborative_albums">Collaborations</h2>')
        with mock.patch.object(wiki, 'wiki_html', return_value=html):
            albums = wiki.albums('Test discography')
        self.assertEqual(albums, [{'year': '1984', 'title': 'Diamond Life'}])

    def test_mixtape_and_ep_tables_are_included(self):
        html = (
            '<h2 id="Studio_albums">Studio albums</h2>'
            '<table><tr><th>Title</th><th>Album details</th></tr>'
            '<tr><td>Can\'t Rush Greatness</td><td>Released: 2025</td></tr>'
            '</table>'
            '<h2 id="Mixtapes">Mixtapes and EPs</h2>'
            '<table><tr><th>Title</th><th>Mixtape details</th></tr>'
            '<tr><td>Wild West</td><td>Released: 2021</td></tr></table>'
            '<table><tr><th>Title</th><th>EP details</th></tr>'
            '<tr><td>Split Decision (with Dave)</td>'
            '<td>Released: 2023</td></tr></table>'
            '<h2 id="Singles">Singles</h2>')
        with mock.patch.object(wiki, 'wiki_html', return_value=html):
            albums = wiki.albums('Test discography')
        self.assertEqual(albums, [
            {'year': '2025', 'title': "Can't Rush Greatness"},
            {'year': '2021', 'title': 'Wild West'},
            {'year': '2023', 'title': 'Split Decision'},
        ])


if __name__ == '__main__':
    unittest.main()
