#!/usr/bin/env python3
"""Regression tests for merging a YouTube Music channel into the artist index.

Topic channels can carry singles that the artist page does not list. The
channel id is parsed from a URL, cached in the index, and its release tiles are
merged without dropping the artist-page releases.

Run with: python3 -m unittest discover -s crates/sonic-cli/tests -v
"""
import importlib.util
import json
import sys
import tempfile
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


ytm = load('ytm_index_channel', SRC / 'ytm_index.py')
batch = load('batch_channel_policy', SRC / 'ytm-artist-batch.py')

CHANNEL_ID = 'UC0123456789abcdefghijkl'
CHANNEL = f'https://music.youtube.com/channel/{CHANNEL_ID}'


def tile(title, subtitle, browse_id, video_id, playlist_id):
    return {'musicTwoRowItemRenderer': {
        'title': {'runs': [{'text': title}]},
        'subtitle': {'runs': [{'text': subtitle}]},
        'navigationEndpoint': {'browseEndpoint': {
            'browseId': browse_id,
            'params': 'params',
            'browseEndpointContextSupportedConfigs': {
                'browseEndpointContextMusicConfig': {
                    'pageType': 'MUSIC_PAGE_TYPE_ALBUM',
                },
            },
        }},
        'thumbnailOverlay': {'musicItemThumbnailOverlayRenderer': {'content': {
            'musicPlayButtonRenderer': {'playNavigationEndpoint': {
                'watchEndpoint': {'videoId': video_id,
                                  'playlistId': playlist_id},
            }},
        }}},
    }}


def songs_shelf(playlist_id):
    return {'musicShelfRenderer': {
        'title': {'runs': [{'text': 'Top songs', 'navigationEndpoint': {
            'browseEndpoint': {'browseId': playlist_id, 'params': 'params'},
        }}]},
        'bottomText': {'runs': [{'text': 'Show all'}]},
    }}


def song_row(title, video_id, artist):
    return {'musicResponsiveListItemRenderer': {
        'playlistItemData': {'videoId': video_id},
        'flexColumns': [
            {'musicResponsiveListItemFlexColumnRenderer': {
                'text': {'runs': [{'text': title}]},
            }},
            {'musicResponsiveListItemFlexColumnRenderer': {
                'text': {'runs': [{'text': artist}]},
            }},
        ],
    }}


class ChannelIndexTest(unittest.TestCase):
    def test_channel_browse_id_accepts_url_and_bare_id(self):
        self.assertEqual(ytm.channel_browse_id(CHANNEL), CHANNEL_ID)
        self.assertEqual(ytm.channel_browse_id(CHANNEL_ID), CHANNEL_ID)
        self.assertIsNone(ytm.channel_browse_id('not-a-channel'))

    def test_header_title_reads_immersive_channel_header(self):
        data = {'header': {'musicImmersiveHeaderRenderer': {
            'title': {'runs': [{'text': 'Digga'}]},
        }}}
        self.assertEqual(ytm._header_title(data), 'Digga')

    def test_merge_prefers_plain_album_over_edition_tile(self):
        releases = {'album': {
            'title': 'Back To Square One (Bluuwuu Edition)',
            'type': 'album', 'playlistId': 'OLAK5uyEDITION',
        }}
        ytm._merge_release(releases, 'album', {
            'title': 'Back To Square One',
            'type': 'album', 'playlistId': 'OLAK5uyPLAIN',
        })
        self.assertEqual(releases['album']['title'], 'Back To Square One')
        self.assertEqual(releases['album']['playlistId'], 'OLAK5uyPLAIN')

        ytm._merge_release(releases, 'album', {
            'title': 'Back To Square One (Bluuwuu Edition)',
            'type': 'album', 'playlistId': 'OLAK5uyEDITION',
        })
        self.assertEqual(releases['album']['title'], 'Back To Square One')

    def test_build_merges_channel_releases_into_artist_index(self):
        artist_page = {'contents': [
            tile('Old Album', 'Album • 2020', 'MPREbOLD', 'vold',
                 'OLAK5uyOLD'),
            songs_shelf('VLOLAK5uyARTIST'),
        ]}
        channel_page = {
            'header': {'musicImmersiveHeaderRenderer': {
                'title': {'runs': [{'text': 'Digga'}]},
            }},
            'contents': [
                tile('Back To Square One', 'Album • 2023', 'MPREbBTSO',
                     'vbts', 'OLAK5uyBTSO'),
                songs_shelf('VLOLAK5uyCHANNEL'),
                {'browseEndpoint': {'browseId': 'MPADUCHANNEL',
                                    'params': 'params'}},
            ],
        }
        channel_releases = {'contents': [
            tile('Double Tap', 'Single • 2026', 'MPREbDT', 'vdt',
                 'OLAK5uyDT'),
        ]}

        def fake_api(endpoint, extra, **_kwargs):
            self.assertEqual(endpoint, 'browse')
            browse_id = extra.get('browseId')
            if browse_id == 'UCARTIST':
                return artist_page
            if browse_id == CHANNEL_ID:
                return channel_page
            if browse_id == 'MPADUCHANNEL':
                return channel_releases
            if browse_id == 'VLOLAK5uyARTIST':
                return {'contents': [song_row('Old Song', 'vold', 'Digga')]}
            if browse_id == 'VLOLAK5uyCHANNEL':
                return {'contents': [song_row('PAYG', 'vpayg', 'Digga')]}
            return {}

        with mock.patch.object(ytm, 'api', side_effect=fake_api), \
                mock.patch.object(ytm, 'artist_browse_id',
                                  return_value=('UCARTIST', 'Digga D')):
            index = ytm.build('digga', note=lambda _message: None,
                              channel=CHANNEL)

        self.assertEqual(index['channelBrowseId'], CHANNEL_ID)
        self.assertEqual(index['channelName'], 'Digga')
        self.assertIn('old album', index['albums'])
        self.assertIn('back to square one', index['albums'])
        self.assertEqual(index['albums']['double tap']['type'], 'single')
        self.assertEqual(index['tracks']['double tap']['playlistId'],
                         'OLAK5uyDT')
        self.assertIn('payg', index['tracks'])

    def test_cached_channel_id_is_reused_on_refresh(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'digga-ytm-index.json'
            path.write_text(json.dumps({'channelBrowseId': CHANNEL_ID}),
                            encoding='utf-8')
            self.assertEqual(batch.cached_channel_id(path), CHANNEL_ID)


def chip_cloud(chips):
    return {'chipCloudRenderer': {'chips': [
        {'chipCloudChipRenderer': {
            'text': {'runs': [{'text': label}]},
            'navigationEndpoint': {'browseSectionListReloadEndpoint': {
                'continuation': {'reloadContinuationData': {
                    'continuation': token,
                }},
            }},
        }} for label, token in chips
    ]}}


class SharedSongsPlaylistTest(unittest.TestCase):
    """The channel often points at the same songs playlist as the artist page."""

    def test_shared_songs_playlist_is_fetched_once(self):
        artist_page = {'contents': [songs_shelf('VLOLAK5uySHARED')]}
        channel_page = {
            'header': {'musicImmersiveHeaderRenderer': {
                'title': {'runs': [{'text': 'Artist'}]},
            }},
            'contents': [songs_shelf('VLOLAK5uySHARED')],
        }
        calls = []

        def fake_api(endpoint, extra, **_kwargs):
            browse_id = extra.get('browseId')
            calls.append(browse_id)
            if browse_id == 'UCARTIST':
                return artist_page
            if browse_id == CHANNEL_ID:
                return channel_page
            return {}

        with mock.patch.object(ytm, 'api', side_effect=fake_api), \
                mock.patch.object(ytm, 'artist_browse_id',
                                  return_value=('UCARTIST', 'Artist')):
            ytm.build('artist', note=lambda _message: None, channel=CHANNEL)

        self.assertEqual(calls.count('VLOLAK5uySHARED'), 1)


class AlbumVsEpTest(unittest.TestCase):
    """A title listed as both an album and an EP must resolve to the album."""

    TITLE = "We're All Alone In This Together"
    ALBUM_PLAYLIST = 'OLAK5uyALBUM12'
    EP_PLAYLIST = 'OLAK5uyEP5'

    def _release(self, rtype, playlist_id, video_id):
        return {
            'title': self.TITLE, 'type': rtype, 'kind': rtype,
            'playlistId': playlist_id, 'videoId': video_id, 'year': '2021',
        }

    def test_merge_prefers_album_over_ep_in_either_order(self):
        key = ytm.normkey(self.TITLE)
        releases = {key: self._release('ep', self.EP_PLAYLIST, 'vep')}
        ytm._merge_release(releases, key,
                           self._release('album', self.ALBUM_PLAYLIST, 'valbum'))
        self.assertEqual(releases[key]['playlistId'], self.ALBUM_PLAYLIST)
        ytm._merge_release(releases, key,
                           self._release('ep', self.EP_PLAYLIST, 'vep'))
        self.assertEqual(releases[key]['playlistId'], self.ALBUM_PLAYLIST)

    def test_build_keeps_album_tile_from_albums_chip(self):
        album_tile = tile(self.TITLE, '2021', 'MPREbALBUM', 'valbum',
                          self.ALBUM_PLAYLIST)
        ep_tile = tile(self.TITLE, 'EP • 2021', 'MPREbEP', 'vep',
                       self.EP_PLAYLIST)
        artist_page = {'contents': [
            album_tile,
            songs_shelf('VLOLAK5uySONGS'),
            {'browseEndpoint': {'browseId': 'MPADUFULL', 'params': 'params'}},
        ]}
        full_page = {'contents': [
            ep_tile,
            chip_cloud([('Albums', 'albums-token'),
                        ('Singles and EPs', 'eps-token')]),
        ]}

        def fake_api(endpoint, extra, **_kwargs):
            self.assertEqual(endpoint, 'browse')
            browse_id = extra.get('browseId')
            token = extra.get('continuation')
            if browse_id == 'UCARTIST':
                return artist_page
            if browse_id == 'MPADUFULL':
                return full_page
            if token == 'albums-token':
                return {'contents': [album_tile]}
            if token == 'eps-token':
                return {'contents': [ep_tile]}
            return {}

        with mock.patch.object(ytm, 'api', side_effect=fake_api), \
                mock.patch.object(ytm, 'artist_browse_id',
                                  return_value=('UCARTIST', 'Dave')):
            index = ytm.build('dave', note=lambda _message: None)

        release = index['albums'][ytm.normkey(self.TITLE)]
        self.assertEqual(release['type'], 'album')
        self.assertEqual(release['playlistId'], self.ALBUM_PLAYLIST)
        self.assertEqual(release['videoId'], 'valbum')


if __name__ == '__main__':
    unittest.main()
