#!/usr/bin/env python3
"""Fast YouTube Music artist index using the InnerTube web API.

This mirrors the steps a person takes in music.youtube.com:

    search for the artist -> click the artist card -> artist "Music" page
    -> "Show all" songs playlist (+ continuation pages)

Only the CLI cookie file is used. SAPISIDHASH values are derived from the
SAPISID cookies on every request, so no private session values from a browser
cURL copy are stored here. The response renderer format is the same one the
web app uses, so IDs are exact music.youtube.com video/playlist IDs.
"""
import hashlib
import json
import re
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import ytm_config

ORIGIN = 'https://music.youtube.com'
VERSION = ytm_config.CLIENT_VERSION
COOKIE_FILE = ytm_config.COOKIES
UA = ('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) '
      'Chrome/158.0.0.0 Safari/537.36')
# Discovery is network-bound. A small fixed pool overlaps independent artist,
# channel, chip and member lookups without touching download concurrency.
DISCOVERY_WORKERS = 4


class YtmError(Exception):
    pass


def parallel_map(jobs, workers=DISCOVERY_WORKERS):
    """Run independent callables in a bounded pool, returning results in order.

    Callers apply the results in their original order so release tie-breaking,
    songs first-wins behavior and note messages stay identical to the old
    sequential path.
    """
    jobs = list(jobs)
    if not jobs:
        return []
    if workers <= 1 or len(jobs) == 1:
        return [job() for job in jobs]
    with ThreadPoolExecutor(max_workers=min(workers, len(jobs))) as pool:
        futures = [pool.submit(job) for job in jobs]
        return [future.result() for future in futures]


def safe_call(job):
    """Keep an optional InnerTube lookup from failing the whole index build."""
    try:
        return True, job()
    except YtmError as exc:
        return False, exc


def load_cookies(path=None):
    """Read a Netscape cookie file into a name -> value dict."""
    ck = {}
    p = Path(path) if path else COOKIE_FILE
    for line in p.read_text(encoding='utf-8', errors='replace').splitlines():
        if not line or line.startswith('#'):
            continue
        fields = line.split('\t')
        if len(fields) >= 7:
            ck[fields[5].strip()] = fields[6].strip()
    return ck


def _auth(cookies):
    """Derive the three SAPISID hashes the signed-in web app sends."""
    ts = int(time.time())
    def digest(cookie):
        if not cookie:
            return None
        raw = f'{ts} {cookie} {ORIGIN}'.encode()
        return f'{ts}_{hashlib.sha1(raw).hexdigest()}'
    parts = []
    for name, cookie in (('SAPISIDHASH', cookies.get('SAPISID')),
                         ('SAPISID1PHASH', cookies.get('__Secure-1PAPISID')),
                         ('SAPISID3PHASH', cookies.get('__Secure-3PAPISID'))):
        value = digest(cookie)
        if value:
            parts.append(f'{name} {value}')
    return ' '.join(parts)


def api(endpoint, extra, timeout=90, cookies=None):
    """POST to a youtubei/v1 endpoint with a minimal WEB_REMIX context."""
    ck = cookies if cookies is not None else load_cookies()
    body = {'context': {'client': {
        'clientName': 'WEB_REMIX', 'clientVersion': VERSION,
        'hl': ytm_config.HL, 'gl': ytm_config.GL,
    }}}
    body.update(extra)
    headers = {
        'Content-Type': 'application/json',
        'X-Youtube-Client-Name': '67',
        'X-Youtube-Client-Version': VERSION,
        'X-Origin': ORIGIN,
        'Origin': ORIGIN,
        'Referer': ORIGIN + '/',
        'X-Youtube-Bootstrap-Logged-In': 'true',
        'X-Goog-AuthUser': '0',
        'Authorization': _auth(ck),
        'Cookie': '; '.join(f'{k}={v}' for k, v in ck.items()),
        'User-Agent': UA,
    }
    url = f'{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false'
    req = urllib.request.Request(url, data=json.dumps(body).encode(),
                                 headers=headers, method='POST')
    try:
        with urllib.request.urlopen(req, timeout=timeout) as response:
            return json.loads(response.read())
    except urllib.error.HTTPError as exc:
        raise YtmError(f'{endpoint}: HTTP {exc.code}') from exc
    except (urllib.error.URLError, TimeoutError) as exc:
        raise YtmError(f'{endpoint}: {exc}') from exc


def text(obj):
    """Flatten a renderer text field (simpleText or runs)."""
    if isinstance(obj, str):
        return obj
    if isinstance(obj, dict):
        if 'simpleText' in obj:
            return obj['simpleText']
        runs = obj.get('runs')
        if isinstance(runs, list):
            return ''.join(r.get('text', '') for r in runs if isinstance(r, dict))
    return ''


def walk(obj):
    """Yield every dict in a decoded InnerTube response."""
    if isinstance(obj, dict):
        yield obj
        for value in obj.values():
            yield from walk(value)
    elif isinstance(obj, list):
        for value in obj:
            yield from walk(value)


def normkey(s, artist=''):
    """Same normalization the batch script uses, minus featured parentheses."""
    s = (s or '').casefold()
    if artist:
        s = re.sub(r'^\s*' + re.escape(artist.casefold()) + r'\s*[-–:]\s*', '', s)
    s = re.sub(r'\([^)]*\)', '', s)
    s = s.replace('&', ' and ').replace("'", '').replace('’', '')
    s = re.sub(r'\b(?:pt|part)\s*\d+\b', '', s)
    s = re.sub(r'[^a-z0-9]+', ' ', s)
    return ' '.join(s.split())


def artist_browse_id(artist):
    """Search for the artist and return the artist card's browseId."""
    data = api('search', {'query': artist})
    cards = []
    for node in walk(data):
        card = node.get('musicCardShelfRenderer')
        if not card:
            continue
        title = text(card.get('title')).strip()
        bids = []
        for inner in walk(card):
            be = inner.get('browseEndpoint') or {}
            bid = be.get('browseId') or ''
            if bid.startswith(('UC', 'MPREb')) and bid not in bids:
                bids.append(bid)
        if normkey(title) == normkey(artist) or normkey(artist) in normkey(title):
            cards.append((title, bids))
    if not cards:
        raise YtmError(f'no artist card found for {artist!r}')
    # Prefer a UC channel id; MPREb cards are usually a single release.
    for title, bids in cards:
        for bid in bids:
            if bid.startswith('UC'):
                return bid, title
    title, bids = cards[0]
    if bids:
        return bids[0], title
    raise YtmError(f'no browseId in artist card for {artist!r}')


def _songs_playlist(data):
    """Find the 'Show all' VLOLAK5uy_ playlist on an artist page."""
    for node in walk(data):
        shelf = node.get('musicShelfRenderer')
        if not shelf:
            continue
        bottom = normkey(text(shelf.get('bottomText')))
        if 'show all' not in bottom and 'see all' not in bottom:
            continue
        try:
            endpoint = (shelf['title']['runs'][0]['navigationEndpoint']
                        ['browseEndpoint'])
        except (KeyError, IndexError, TypeError):
            continue
        bid = endpoint.get('browseId') or ''
        if bid.startswith('VLOLAK5uy'):
            return bid, endpoint.get('params')
    return None, None


def _songs_from(data, artist):
    """Extract (key, videoId, title, artist, explicit) rows from song lists."""
    rows = []
    for node in walk(data):
        item = node.get('musicResponsiveListItemRenderer')
        if not item:
            continue
        vid = (item.get('playlistItemData') or {}).get('videoId')
        columns = item.get('flexColumns') or []
        if not vid or not columns:
            continue
        title = text(columns[0].get('musicResponsiveListItemFlexColumnRenderer', {})
                     .get('text')).strip()
        credited = ''
        if len(columns) > 1:
            credited = text(columns[1].get('musicResponsiveListItemFlexColumnRenderer', {})
                            .get('text')).strip()
            credited = re.sub(r'\s+\d[\d.,]*\s+plays.*$', '', credited,
                              flags=re.IGNORECASE).strip()
        key = normkey(title, artist)
        if key:
            rows.append((key, vid, title, credited, _explicit(item)))
    return rows


def _watch_endpoint(item):
    """The play button on a release tile carries the exact video/playlist IDs."""
    overlay = item.get('thumbnailOverlay') or {}
    overlay = overlay.get('musicItemThumbnailOverlayRenderer', {}).get('content', {})
    play = overlay.get('musicPlayButtonRenderer', {}).get('playNavigationEndpoint', {})
    return play.get('watchEndpoint') or {}


def _explicit(item):
    """Read YouTube Music's E badge from a release tile or a track row."""
    badges = item.get('subtitleBadges') or item.get('badges') or []
    for badge in badges:
        renderer = badge.get('musicInlineBadgeRenderer') or {}
        icon = json.dumps(renderer.get('icon') or {}).upper()
        accessibility = (((renderer.get('accessibilityData') or {})
                          .get('accessibilityData') or {}).get('label') or '')
        if 'MUSIC_EXPLICIT_BADGE' in icon or accessibility.casefold() == 'explicit':
            return True
    return False


def _is_edition_title(title):
    """True for parenthetical edition/version suffixes of the same release."""
    return bool(re.search(
        r'\([^)]*\b(?:edition|version|remaster(?:ed)?|deluxe|limited|'
        r'collector(?:’s|\'s)?)\b[^)]*\)', title or '', re.I))


# When YouTube Music lists the same title as both a full album and a shorter
# EP/single, the artist page shows the album. Downloads must follow that tile.
_RELEASE_TYPE_RANK = {'album': 3, '': 2, 'ep': 1, 'single': 0}


def _release_rank(release):
    rtype = str(release.get('type') or '').casefold()
    return _RELEASE_TYPE_RANK.get(rtype, 2)


def _merge_release(releases, key, release):
    """Prefer full albums, then exact IDs, then plain titles over editions."""
    previous = releases.get(key)
    if previous is None:
        releases[key] = release
        return
    previous_edition = _is_edition_title(previous.get('title'))
    release_edition = _is_edition_title(release.get('title'))
    if previous_edition and not release_edition:
        releases[key] = release
        return
    if release_edition and not previous_edition:
        # Keep the plain tile unless it has no exact playlist to download.
        if not previous.get('playlistId') and release.get('playlistId'):
            releases[key] = release
        return
    if (_release_rank(release) > _release_rank(previous)
            or (release.get('playlistId') and not previous.get('playlistId'))):
        releases[key] = release


def channel_browse_id(value):
    """Extract a UC channel id from a channel URL or a bare id."""
    match = re.search(r'UC[A-Za-z0-9_-]{22}', str(value or ''))
    return match.group(0) if match else None


def _header_title(data):
    header = data.get('header') or {}
    for name in ('musicImmersiveHeaderRenderer', 'musicBrowsePageHeaderRenderer'):
        renderer = header.get(name)
        if renderer:
            title = text(renderer.get('title')).strip()
            if title:
                return title
    return ''


def _release_endpoints(data):
    """All full release-list endpoints on an artist/channel page."""
    out = []
    for node in walk(data):
        endpoint = node.get('browseEndpoint')
        if not isinstance(endpoint, dict):
            continue
        browse_id = endpoint.get('browseId') or ''
        if not browse_id.startswith('MPADU'):
            continue
        item = (browse_id, endpoint.get('params'))
        if item not in out:
            out.append(item)
    return out


def _chip_continuations(data):
    """All reload tokens from release chips such as Albums or Singles and EPs."""
    out, seen = [], set()
    for node in walk(data):
        chips = node.get('chipCloudRenderer')
        if not chips:
            continue
        for chip in chips.get('chips', []):
            renderer = chip.get('chipCloudChipRenderer') or {}
            label = normkey(text(renderer.get('text')))
            token = (((renderer.get('navigationEndpoint', {})
                       .get('browseSectionListReloadEndpoint', {})
                       .get('continuation', {}) or {})
                      .get('reloadContinuationData', {}) or {})
                     .get('continuation'))
            if label and token and token not in seen:
                seen.add(token)
                out.append((label, token))
    return out


def _add_video(videos, video_id, values):
    if not video_id:
        return
    record = videos.setdefault(video_id, {'videoId': video_id})
    for key, value in values.items():
        if key not in record or record[key] in (None, '', False):
            record[key] = value


def _release_entries(data, artist, default_kind=''):
    """Ordered (key, release) pairs from one response page."""
    return list(_releases_from(data, artist, default_kind).items())


def _release_entries_from_chain(data, artist, default_kind=''):
    """Release entries from a first page plus its continuation pages."""
    entries = []
    seen = set()
    page = data
    while True:
        entries.extend(_release_entries(page, artist, default_kind))
        token = _continuation_token(page)
        if not token or token in seen:
            break
        seen.add(token)
        page = api('browse', {'continuation': token})
    return entries


def _apply_releases(entries, releases):
    for key, release in entries:
        _merge_release(releases, key, release)


def _collect_releases(data, releases, artist, default_kind='', follow=True):
    """Merge release tiles from a browse response into `releases`."""
    entries = (_release_entries_from_chain(data, artist, default_kind)
               if follow else _release_entries(data, artist, default_kind))
    _apply_releases(entries, releases)


def _song_entries(data, artist):
    """Ordered song rows from one response page."""
    return _songs_from(data, artist)


def _song_entries_from_chain(data, artist):
    """Song rows from a first page plus its continuation pages."""
    entries = []
    seen = set()
    page = data
    for _page_no in range(20):
        entries.extend(_song_entries(page, artist))
        token = _continuation_token(page)
        if not token or token in seen:
            break
        seen.add(token)
        page = api('browse', {'continuation': token})
    return entries


def _apply_songs(entries, tracks, videos, artist):
    for key, video_id, title, credited, explicit in entries:
        _add_video(videos, video_id, {
            'title': title, 'artist': credited, 'explicit': explicit})
        if video_id and key not in tracks:
            tracks[key] = {'videoId': video_id, 'title': title,
                           'artist': credited, 'explicit': explicit}


def _collect_songs(data, tracks, videos, artist):
    """Merge song rows from a playlist response into tracks/videos."""
    _apply_songs(_song_entries_from_chain(data, artist), tracks, videos, artist)


def _releases_from(data, artist, default_kind=''):
    """Extract releases with exact playlist/track IDs from the artist page.

    Every release tile has both a videoId (the representative track) and a
    playlistId (the OLAK5uy_ audio playlist) in its play button, plus a year
    in the subtitle. The subtitle also labels "Single" and "EP"; anything on
    the Albums tab is an album. The explicit badge is per release tile.
    """
    releases = {}
    for node in walk(data):
        item = node.get('musicTwoRowItemRenderer')
        if not item:
            continue
        endpoint = (item.get('navigationEndpoint') or {}).get('browseEndpoint') or {}
        bid = endpoint.get('browseId') or ''
        configs = endpoint.get('browseEndpointContextSupportedConfigs', {})
        page_type = configs.get('browseEndpointContextMusicConfig', {}).get('pageType')
        if not bid.startswith('MPREb') or page_type != 'MUSIC_PAGE_TYPE_ALBUM':
            continue
        title = text(item.get('title')).strip()
        key = normkey(title, artist)
        if not title or not key:
            continue
        watch = _watch_endpoint(item)
        subtitle = text(item.get('subtitle')).strip()
        years = re.findall(r'\b(?:19|20)\d{2}\b', subtitle)
        label = subtitle.split(' • ', 1)[0].casefold() if ' • ' in subtitle else ''
        rtype = label if label in ('single', 'ep') else (default_kind or '')
        release = {
            'title': title,
            'browseId': bid,
            'params': endpoint.get('params'),
            'videoId': watch.get('videoId'),
            'playlistId': watch.get('playlistId'),
            'year': years[-1] if years else '',
            'kind': label,
            'type': rtype,
            'explicit': _explicit(item),
        }
        _merge_release(releases, key, release)
    return releases


def _releases_endpoint(data):
    """Find the artist's full release-list endpoint (MPADU... + params)."""
    for node in walk(data):
        endpoint = node.get('browseEndpoint')
        if not isinstance(endpoint, dict):
            continue
        bid = endpoint.get('browseId') or ''
        if bid.startswith('MPADU'):
            return bid, endpoint.get('params')
    return None, None


def _chip_continuation(data, label):
    """Get the reload token for a chip such as 'Albums' on the release tabs."""
    for node in walk(data):
        chips = node.get('chipCloudRenderer')
        if not chips:
            continue
        for chip in chips.get('chips', []):
            renderer = chip.get('chipCloudChipRenderer') or {}
            if normkey(text(renderer.get('text'))) != normkey(label):
                continue
            continuation = (renderer.get('navigationEndpoint', {})
                            .get('browseSectionListReloadEndpoint', {})
                            .get('continuation', {})
                            .get('reloadContinuationData', {})
                            .get('continuation'))
            if continuation:
                return continuation
    return None


def _continuation_token(data):
    for node in walk(data):
        item = node.get('continuationItemRenderer')
        if not item:
            continue
        command = (item.get('continuationEndpoint', {})
                   .get('continuationCommand', {}))
        if command.get('token'):
            return command['token']
    return None


def build(artist, note=print, channel=None):
    """Build title/exact-ID indexes for one artist.

    An optional YouTube Music channel URL/ID is merged into the same index.
    Topic channels often carry albums and singles that the artist page omits
    (for example "Back To Square One" for Digga).

    Independent InnerTube lookups are fetched in a small bounded pool, then
    applied in the original order so tile tie-breaking and first-wins song
    behavior stay exactly as they were in the sequential implementation.
    """
    say = note or (lambda _message: None)

    # Stage 1: artist search and the requested channel page do not depend on
    # each other. The artist page itself needs the search result.
    first_jobs = [lambda: artist_browse_id(artist)]
    if channel:
        channel_id = channel_browse_id(channel)
        if not channel_id:
            raise YtmError(f'no UC channel id in {channel!r}')
        first_jobs.append(lambda cid=channel_id: api('browse', {'browseId': cid}))
    first = parallel_map(first_jobs)
    bid, display_name = first[0]
    channel_page = first[1] if channel else None
    say(f'index: artist page {bid}')

    channel_name = None
    if channel_page is not None:
        channel_name = _header_title(channel_page)
        say(f'index: channel {channel_id}'
            + (f' ({channel_name})' if channel_name else ''))
        if channel_name and normkey(channel_name) != normkey(artist):
            say(f'index: channel title {channel_name!r} does not exactly '
                f'match {artist!r}; using it because it was requested')

    # Stage 2: artist page plus every channel release-list page. Channel
    # endpoints are known from the channel page and independent of the artist.
    second_jobs = [lambda: safe_call(lambda: api('browse', {'browseId': bid}))]
    endpoint_extras = []
    if channel_page is not None:
        for endpoint_bid, endpoint_params in _release_endpoints(channel_page):
            extra = {'browseId': endpoint_bid}
            if endpoint_params:
                extra['params'] = endpoint_params
            endpoint_extras.append(extra)
            second_jobs.append(
                lambda extra=extra: safe_call(lambda extra=extra: api('browse', extra)))
    second = parallel_map(second_jobs)
    artist_ok, artist_value = second[0]
    if not artist_ok:
        raise artist_value
    page = artist_value
    endpoint_results = second[1:]

    albums = {}
    _apply_releases(_release_entries(page, artist), albums)

    full_bid, full_params = _releases_endpoint(page)
    full_extra = None
    if full_bid:
        full_extra = {'browseId': full_bid}
        if full_params:
            full_extra['params'] = full_params

    playlist, params = _songs_playlist(page)
    channel_playlist = channel_params = None
    if channel_page is not None:
        channel_playlist, channel_params = _songs_playlist(channel_page)

    # Stage 3: full release list and song playlists. A channel pointing at the
    # same songs playlist is fetched once and reused.
    third_specs = []
    if full_extra:
        third_specs.append((
            'full',
            lambda extra=full_extra: safe_call(lambda extra=extra: api('browse', extra))))
    if playlist:
        extra = {'browseId': playlist}
        if params:
            extra['params'] = params
        third_specs.append((
            'artist_songs',
            lambda extra=extra: safe_call(
                lambda extra=extra: _song_entries_from_chain(api('browse', extra), artist))))
    if channel_page is not None and channel_playlist and channel_playlist != playlist:
        extra = {'browseId': channel_playlist}
        if channel_params:
            extra['params'] = channel_params
        third_specs.append((
            'channel_songs',
            lambda extra=extra: safe_call(
                lambda extra=extra: _song_entries_from_chain(api('browse', extra), artist))))
    third_values = parallel_map([job for _name, job in third_specs])
    third = dict(zip([name for name, _job in third_specs], third_values))

    if playlist:
        say(f'index: songs playlist {playlist}')
    if channel_page is not None:
        if channel_playlist == playlist:
            say(f'index: channel songs playlist {channel_playlist} matches '
                'the artist songs playlist; reusing it')
        elif channel_playlist:
            say(f'index: channel songs playlist {channel_playlist}')

    full_page = None
    if full_extra:
        full_ok, full_value = third['full']
        if full_ok:
            full_page = full_value
            _apply_releases(_release_entries_from_chain(full_page, artist), albums)
        else:
            say(f'index: full release list unavailable ({full_value})')

    endpoint_pages = []
    for extra, (ok, value) in zip(endpoint_extras, endpoint_results):
        if ok:
            endpoint_pages.append(value)
        else:
            say(f'index: channel release list unavailable ({value})')

    # Stage 4: every chip continuation from the full list and from each
    # channel endpoint can be fetched together, but is applied in chip order.
    chip_jobs = []
    full_chips = []
    if full_page is not None:
        for label, token in _chip_continuations(full_page):
            full_chips.append(label)
            chip_jobs.append(_chip_entries_job(label, token, artist))
    endpoint_chips = []
    for release_page in endpoint_pages:
        labels = []
        for label, token in _chip_continuations(release_page):
            labels.append(label)
            chip_jobs.append(_chip_entries_job(label, token, artist))
        endpoint_chips.append(labels)
    chip_results = parallel_map(chip_jobs)

    cursor = 0
    for label in full_chips:
        ok, entries = chip_results[cursor]
        cursor += 1
        if ok:
            _apply_releases(entries, albums)
        else:
            say(f'index: full release list unavailable ({entries})')

    channel_releases = {}
    if channel_page is not None:
        _apply_releases(_release_entries(channel_page, artist), channel_releases)
        for release_page, labels in zip(endpoint_pages, endpoint_chips):
            _apply_releases(_release_entries_from_chain(release_page, artist),
                            channel_releases)
            for label in labels:
                ok, entries = chip_results[cursor]
                cursor += 1
                if ok:
                    _apply_releases(entries, channel_releases)
                else:
                    say(f'index: channel release list unavailable ({entries})')
        before = len(albums)
        for key, release in channel_releases.items():
            _merge_release(albums, key, release)
        say(f'index: channel added {len(albums) - before} release(s)')

    # tracks is keyed by normalized title (for title lookups); videos is keyed
    # by exact videoId so clean/explicit versions of the same title stay apart.
    tracks, videos = {}, {}

    # Release tiles already carry exact track + playlist IDs, so seed the
    # index with them; the "Show all" songs lists only fill the gaps.
    for key, release in albums.items():
        if not release.get('videoId'):
            continue
        tracks[key] = {
            'videoId': release['videoId'],
            'playlistId': release.get('playlistId'),
            'title': release['title'],
            'artist': display_name,
            'year': release.get('year', ''),
            'type': release.get('type') or '',
            'explicit': release.get('explicit', False),
        }
        _add_video(videos, release['videoId'], {
            'title': release['title'], 'artist': display_name,
            'playlistId': release.get('playlistId'),
            'year': release.get('year', ''),
            'type': release.get('type') or '',
            'explicit': release.get('explicit', False),
            'release': release['title'], 'releaseKey': key,
        })

    for name in ('artist_songs', 'channel_songs'):
        if name not in third:
            continue
        ok, entries = third[name]
        if ok:
            _apply_songs(entries, tracks, videos, artist)
        else:
            say(f'index: {name.replace("_", " ")} playlist unavailable ({entries})')

    say(f'index: {len(tracks)} tracks, {len(videos)} videos, '
        f'{len(albums)} releases')
    return {
        'artist': artist,
        'artistName': display_name,
        'artistBrowseId': bid,
        'channelBrowseId': channel_id if channel_page is not None else None,
        'channelName': channel_name,
        'songsPlaylist': playlist,
        'tracks': tracks,
        'videos': videos,
        'albums': albums,
        'members': {},
        'fetched': time.time(),
    }


def _chip_entries_job(label, token, artist):
    kind = 'album' if label == 'albums' else ''
    def job():
        page = api('browse', {'continuation': token})
        return _release_entries_from_chain(page, artist, kind)
    return lambda: safe_call(job)


def read(path):
    """Read and upgrade a cached index regardless of its age."""
    try:
        data = json.loads(Path(path).read_text(encoding='utf-8'))
    except (OSError, ValueError, AttributeError):
        return None
    if not isinstance(data, dict):
        return None
    # Older caches predate the videos/members maps; upgrade in place so a
    # rebuilt index is not required to gain per-video explicit flags.
    data.setdefault('videos', {})
    for track in data.get('tracks', {}).values():
        vid = track.get('videoId')
        if vid:
            data['videos'].setdefault(vid, {'videoId': vid, **track})
    data.setdefault('members', {})
    return data


def is_fresh(index, max_age=7 * 86400):
    if not index:
        return False
    return not max_age or time.time() - index.get('fetched', 0) <= max_age


def load(path, max_age=7 * 86400):
    """Load a cached index unless it is missing or older than max_age seconds."""
    data = read(path)
    return data if is_fresh(data, max_age) else None


def save(path, index):
    p = Path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(index, ensure_ascii=False, indent=1), encoding='utf-8')


def _member_rows(data):
    rows = []
    for node in walk(data):
        item = node.get('musicResponsiveListItemRenderer')
        if not item:
            continue
        vid = (item.get('playlistItemData') or {}).get('videoId')
        if not vid:
            continue
        columns = item.get('flexColumns') or []
        title = text(columns[0].get('musicResponsiveListItemFlexColumnRenderer', {})
                     .get('text')).strip() if columns else ''
        rows.append({'videoId': vid, 'title': title,
                     'explicit': _explicit(item)})
    return rows


def fetch_release_members(index, keys=None, note=None, workers=DISCOVERY_WORKERS):
    """Cache missing member lists for several releases in a bounded pool.

    Results are written back in the requested key order, so ownership ranking
    and note ordering are unchanged from sequential discovery.
    """
    members = index.setdefault('members', {})
    albums = index.get('albums') or {}
    keys = list(keys if keys is not None else albums)
    missing = []
    for key in keys:
        if key in members:
            continue
        entry = albums.get(key) or {}
        if entry.get('browseId'):
            missing.append((key, entry))

    if not missing:
        return members

    def job(key, entry):
        data = api('browse', {'browseId': entry['browseId']})
        return key, entry, _member_rows(data)

    results = parallel_map([lambda key=key, entry=entry: job(key, entry)
                            for key, entry in missing], workers=workers)
    for key, entry, rows in results:
        members[key] = rows
        if note:
            note(f'index: {entry.get("title") or key}: '
                 f'{len(rows)} member track(s)')
    return members


def refresh_release_members(index, keys=None, note=None, workers=DISCOVERY_WORKERS):
    """Refresh selected release members, merging rows new since the cache.

    A cached browse result can predate a playlist edit. Refetching only the
    releases in this run keeps the small discovery pool bounded, while merging
    preserves rows already cached so a bad response cannot shrink the list.
    """
    albums = index.get('albums') or {}
    keys = [key for key in (keys if keys is not None else albums)
            if (albums.get(key) or {}).get('browseId')]
    if not keys:
        return index.setdefault('members', {})

    def job(key, entry):
        data = api('browse', {'browseId': entry['browseId']})
        return key, entry, _member_rows(data)

    results = parallel_map(
        [lambda key=key, entry=albums[key]: job(key, entry) for key in keys],
        workers=workers)
    members = index.setdefault('members', {})
    for key, entry, rows in results:
        current = [row for row in (members.get(key) or [])
                   if row.get('videoId') or row.get('title')]
        seen = {(row.get('videoId'), row.get('title')) for row in current}
        merged = list(current)
        added = 0
        for row in rows:
            identity = (row.get('videoId'), row.get('title'))
            if identity in seen:
                continue
            merged.append(row)
            seen.add(identity)
            added += 1
        if added or len(merged) != len(members.get(key) or []):
            members[key] = merged
        if note and added:
            note(f'index: {entry.get("title") or key}: '
                 f'added {added} member track(s) since the cache')
    return members


def release_members(index, key, note=None):
    """Exact track IDs (and E badges) inside one album/EP release.

    Cached in the index because the batch script asks for these repeatedly and
    a fresh browse costs about a second per release.
    """
    members = index.setdefault('members', {})
    if key in members:
        return members[key]
    fetch_release_members(index, [key], note=note)
    return members.get(key, [])


def _release_year_rank(index, key, artist=''):
    """Sort key for release ownership: dated releases beat undated ones."""
    entry = (index.get('albums') or {}).get(key) or {}
    match = re.search(r'(?:19|20)\d{2}', str(entry.get('year') or ''))
    year = int(match.group(0)) if match else 9999
    title = normkey(entry.get('title') or key, artist)
    return (year, title, key)


def _find_token(parent, token):
    root = token
    while parent[root] != root:
        root = parent[root]
    while parent[token] != root:
        next_token = parent[token]
        parent[token] = root
        token = next_token
    return root


def _union_tokens(parent, left, right):
    left, right = _find_token(parent, left), _find_token(parent, right)
    if left != right:
        parent[right] = left


def _member_tokens(row, artist=''):
    """Exact video id plus normalized title, so aliases share one identity."""
    tokens = []
    video_id = str(row.get('videoId') or '')
    if video_id:
        tokens.append(f'v:{video_id}')
    title = normkey(row.get('title'), artist)
    if title:
        tokens.append(f't:{title}')
    return tokens


def _restore_excluded_members(index):
    """Put previously pruned rows back before ownership is recomputed.

    Older indexes let standalone singles own album tracks, so affected album
    rows may be absent from `members` and only present in `excludedMembers`.
    Restoring them keeps cached indexes repairable without a network refetch.
    """
    members = index.setdefault('members', {})
    for release_key, entries in (index.get('excludedMembers') or {}).items():
        rows = members.setdefault(release_key, [])
        seen = {(row.get('videoId'), row.get('title')) for row in rows}
        for entry in entries:
            row = dict(entry.get('row') or {})
            for field in ('ownerKey', 'ownerRelease', 'ownerYear'):
                row.pop(field, None)
            if not row.get('videoId') and not row.get('title'):
                continue
            identity = (row.get('videoId'), row.get('title'))
            if identity in seen:
                continue
            position = entry.get('index')
            try:
                position = max(0, min(int(position), len(rows)))
            except (TypeError, ValueError):
                position = len(rows)
            rows.insert(position, row)
            seen.add(identity)


def flatten_track_ownership(index, artist='', note=None, fetch=True):
    """Assign album/EP member tracks to their earliest album/EP release.

    A track is identified by exact video id and by normalized title, which are
    unioned per row. Thus an alias video id still matches the same song, while
    two different titles with one video id also collapse into one identity.
    Standalone singles never own tracks: an earlier single must not make a full
    album download incomplete. Releases are visited oldest first, so the
    earliest album/EP owns the track and every later copy is reported.
    """
    albums = index.get('albums') or {}
    members = index.setdefault('members', {})
    _restore_excluded_members(index)
    keys = [key for key, release in albums.items()
            if (release.get('type') or 'album') != 'single']
    if fetch:
        fetch_release_members(index, keys)

    parent = {}
    for key in keys:
        for row in members.get(key) or []:
            tokens = _member_tokens(row, artist)
            for token in tokens:
                parent.setdefault(token, token)
            for token in tokens[1:]:
                _union_tokens(parent, tokens[0], token)

    groups = {}
    for key in sorted(keys, key=lambda item: _release_year_rank(index, item, artist)):
        release = albums[key] or {}
        owner = {
            'key': key,
            'title': release.get('title') or '',
            'year': release.get('year') or '',
            'type': release.get('type') or 'album',
            'playlistId': release.get('playlistId') or '',
        }
        for position, row in enumerate(members.get(key) or []):
            tokens = _member_tokens(row, artist)
            if not tokens:
                continue
            canonical = _find_token(parent, tokens[0])
            group = groups.setdefault(canonical, {'owner': None, 'duplicates': []})
            if group['owner'] is None:
                group['owner'] = {
                    **owner,
                    'videoId': row.get('videoId') or '',
                    'trackTitle': row.get('title') or '',
                    'ownerKey': canonical,
                }
                continue
            duplicate = dict(row)
            duplicate.update({
                'ownerKey': canonical,
                'ownerRelease': group['owner']['title'],
                'ownerYear': group['owner']['year'],
            })
            group['duplicates'].append({
                'releaseKey': key, 'index': position, 'row': duplicate})

    owners = {canonical: group['owner'] for canonical, group in groups.items()
              if group['owner'] is not None}
    duplicates = {}
    for group in groups.values():
        for entry in group['duplicates']:
            duplicates.setdefault(entry['releaseKey'], []).append(entry)
    return owners, duplicates


def prune_duplicate_members(index, artist='', note=None, fetch=True):
    """Persist flat ownership into the cached index.

    Duplicate rows are removed from every later release's `members` list. The
    owner map and removed rows stay in the index so the batch runner can skip
    those exact video ids before handing a playlist to sonic-cli. Rows removed
    by an earlier ownership policy are restored first, so a cached index can be
    repaired without fetching the release lists again.
    """
    owners, duplicates = flatten_track_ownership(
        index, artist=artist, note=note, fetch=fetch)
    removed = 0
    for release_key, entries in duplicates.items():
        drop = {entry['index'] for entry in entries}
        rows = (index.get('members') or {}).get(release_key) or []
        index.setdefault('members', {})[release_key] = [
            row for position, row in enumerate(rows) if position not in drop]
        removed += len(drop)
    index['trackOwners'] = owners
    index['excludedMembers'] = {
        release_key: [entry['row'] for entry in entries]
        for release_key, entries in duplicates.items()
    }
    index['flatDedupe'] = True
    if note and removed:
        note(f'flat YTM dedupe: {removed} duplicate track(s) removed from '
             'later release(s); earliest release keeps each one')
    return owners, index['excludedMembers']


def album_playlist(browse_id):
    """Turn an MPREb_ release ID into its OLAK5uy_ playlist and exact tracks."""
    data = api('browse', {'browseId': browse_id})
    pid = None
    for node in walk(data):
        for value in node.values():
            if isinstance(value, str) and value.startswith('OLAK5uy'):
                pid = value
                break
        if pid:
            break
    titles, video_ids = [], []
    for node in walk(data):
        item = node.get('musicResponsiveListItemRenderer')
        if not item:
            continue
        columns = item.get('flexColumns') or []
        if columns:
            title = text(columns[0].get('musicResponsiveListItemFlexColumnRenderer', {})
                         .get('text')).strip()
            if title:
                titles.append(title)
        vid = (item.get('playlistItemData') or {}).get('videoId')
        if vid and vid not in video_ids:
            video_ids.append(vid)
    if not pid or not titles:
        raise YtmError(f'release {browse_id} has no playlist/tracks')
    return {'playlist': pid, 'titles': titles, 'video_ids': video_ids}
