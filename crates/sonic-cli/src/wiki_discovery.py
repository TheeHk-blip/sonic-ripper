#!/usr/bin/env python3
"""Extract studio albums and singles from a Wikipedia discography page."""
import html as htmlmod
import json
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from functools import lru_cache


def text(cell):
    s = re.sub(r'<[^>]+>', ' ', cell)
    s = htmlmod.unescape(s)
    s = s.replace('\u00a0', ' ')
    return ' '.join(s.split())


def cells(row):
    return [text(c) for c in re.findall(r'<t[hd][^>]*>(.*?)</t[hd]>', row, re.S)]


def section(html, start_id, stop_ids=()):
    m = re.search(r'id="' + re.escape(start_id) + r'"', html)
    if not m:
        return ''
    rest = html[m.end():]
    # Stop ids are prefixes: "Posthumous" must match the real heading
    # "Posthumous_releases_and_compilations", otherwise the section runs to the
    # bottom of the page and picks up unrelated tables (e.g. authority data).
    stops = []
    for stop in stop_ids:
        hit = re.search(r'id="' + re.escape(stop) + r'(?:_|")', rest)
        if hit:
            stops.append(hit.start())
    end = min(stops) if stops else len(rest)
    return rest[:end]


MAX_RATE_LIMIT_WAIT = 5.0
RATE_LIMIT_ATTEMPTS = 2


def _rate_limit_wait(exc, attempt):
    """Honour Retry-After, but never block a catalog run for minutes."""
    fallback = min(3 * 2 ** attempt, 30)
    try:
        wait = float(exc.headers.get('Retry-After') or fallback)
    except (TypeError, ValueError):
        wait = fallback
    return min(max(wait, 0), MAX_RATE_LIMIT_WAIT)


def api(params, tries=5):
    url = 'https://en.wikipedia.org/w/api.php?' + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers={
        'User-Agent': 'sonic-ytm-batch/1.0 (local music library use)'})
    last: Exception = RuntimeError('request failed')
    for attempt in range(tries):
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                return json.load(r)
        except urllib.error.HTTPError as e:
            last = e
            if e.code in (429, 503):
                # Wikipedia is only a reference list. One short retry is enough;
                # a longer Retry-After must not make a detached run look hung.
                if attempt + 1 >= min(RATE_LIMIT_ATTEMPTS, tries):
                    raise
                time.sleep(_rate_limit_wait(e, attempt))
                continue
            raise
        except Exception as e:
            last = e
            time.sleep(1 + attempt)
    raise last


def page_variants(page):
    out = [page]
    base = re.sub(r'\s+discography\s*$', '', page, flags=re.I).strip()
    bases = [base]
    if ' & ' in base or base.startswith('& '):
        bases.append(base.replace(' & ', ' and ').replace('& ', 'and ', 1))
    if re.search(r'\s+and\s+', base, re.I):
        bases.append(re.sub(r'\s+and\s+', ' & ', base, flags=re.I))
    for b in bases:
        cap = b[:1].upper() + b[1:]
        out += [f'{b} discography', f'{cap} discography', cap,
                b.title(), f'{b.title()} discography', b]
    return list(dict.fromkeys(x for x in out if x))


@lru_cache(maxsize=64)
def parse_html(candidate):
    data = api({'action': 'parse', 'page': candidate,
                'prop': 'text', 'format': 'json'})
    return data['parse']['text']['*'] if 'parse' in data else None


def normalize_artist_title(value):
    """Normalize a Wikipedia title to the bare artist name.

    Discography suffixes and disambiguation parentheticals are removed, and
    ``and``/``&`` are treated as the same word. Nothing else is fuzzy-matched.
    """
    text = value.casefold().strip()
    text = re.sub(r'\([^)]*\)', ' ', text)
    text = re.sub(r'\s+discography\s*$', '', text)
    text = re.sub(r'\s+and\s+', ' & ', text)
    text = re.sub(r'\s*&\s*', ' & ', text)
    return re.sub(r'\s+', ' ', text).strip(' ,;:')


def artist_title_matches(title, artist):
    """Accept only the exact requested artist name, never a fuzzy hit.

    "digga d" must not resolve to "Rah Digga discography" just because the
    letters appear in order. Allow only the same name, optionally with a
    "discography" suffix or a Wikipedia disambiguation parenthetical.
    """
    wanted = normalize_artist_title(artist)
    return bool(wanted) and normalize_artist_title(title) == wanted


@lru_cache(maxsize=32)
def discography_search(page):
    base = re.sub(r'\s+discography\s*$', '', page, flags=re.I).strip()
    data = api({'action': 'query', 'list': 'search',
                'srsearch': f'{base} discography', 'srlimit': 6, 'format': 'json'})
    hits = [h['title'] for h in data.get('query', {}).get('search', [])]
    # Never fall back to an unrelated discography page. Without this filter a
    # search for "Cutty Ranks discography" could accept "Shaggy discography",
    # and a search for "Digga D" could accept "Rah Digga discography".
    return [h for h in hits if artist_title_matches(h, base)]


@lru_cache(maxsize=32)
def wiki_html(page, need=()):
    """Prefer a dedicated discography page with the requested sections."""
    def usable(h):
        return all(f'id="{s}"' in h for s in need)

    hits = discography_search(page)
    key = re.sub(r'\s+discography\s*$', '', page, flags=re.I).strip().casefold()
    ordered = ([h for h in hits if 'discography' in h.casefold() and key in h.casefold()]
               + page_variants(page) + hits)
    fallback = None
    for candidate in ordered:
        h = parse_html(candidate)
        if not h:
            continue
        fallback = fallback or h
        if usable(h):
            return h
    if fallback:
        return fallback
    raise RuntimeError(f'no Wikipedia page found for {page!r}')


def album_from_line(value):
    """Parse a bullet-style studio album entry such as "1975 – Deep Down"."""
    m = re.match(r'^\s*((?:19|20)\d{2})\s*[-–—:]\s*(.+)$', value)
    if not m:
        return None
    year, title = m.groups()
    title = re.sub(r'\[\s*\d+\s*\]', '', title)
    # Parentheticals are labels, reissue notes or references, not album titles.
    previous = None
    while previous != title:
        previous = title
        title = re.sub(r'\([^()]*\)', '', title)
    title = re.split(r'\s+(?:aka|reissued)\b', title, flags=re.I)[0]
    title = title.strip(' ,;:\t "\'')
    if not title or re.fullmatch(r'(?:19|20)\d{2}', title):
        return None
    return {'year': year, 'title': title}


def clean_table_title(value):
    """Title cell -> release title, without note refs or collaborator notes."""
    title = re.sub(r'\[\s*(?:nb\s*)?\d*\s*\]', '', value)
    title = re.sub(r'\s*\((?:with|feat\.?|featuring)\b[^)]*\)', '', title,
                   flags=re.I)
    title = re.split(r'\s+(?:aka|reissued)\b', title, flags=re.I)[0]
    return title.strip(' ,;:\t "\'')


def table_releases(sec):
    """All year-bearing release rows in a section's tables."""
    rows = []
    for table in re.findall(r'<table.*?</table>', sec, re.S):
        for row in re.findall(r'<tr.*?</tr>', table, re.S):
            c = cells(row)
            if len(c) < 2 or c[0].casefold() in ('title', 'album'):
                continue
            title = clean_table_title(c[0])
            if not title or re.fullmatch(
                    r'(?:19|20)\d{2}\s*[-–]\s*(?:(?:19|20)?\d{2,4})', title):
                continue
            year = re.search(r'\b(?:19|20)\d{2}\b', c[1])
            if year:
                rows.append((year.group(0), title))
    return rows


def albums(page):
    """Wikipedia's non-single releases: albums, mixtapes and EPs.

    YTM calls some mixtapes albums and some EPs albums, so restricting this to
    the "Studio albums" section hides real indexed releases (e.g. Central Cee
    only had one studio album on Wikipedia while YTM had four albums and two
    EPs). Every release table in these sections is a reference candidate; YTM
    still decides which ones are downloaded.
    """
    html = wiki_html(page, need=('Studio_albums',))
    sections = [
        section(html, 'Studio_albums',
                ('Posthumous', 'Collaborative_albums', 'Mixtapes', 'EPs')),
        section(html, 'Mixtapes', ('Singles', 'Music_videos',
                                   'Other_appearances', 'EPs')),
        section(html, 'EPs', ('Singles', 'Music_videos', 'Other_appearances')),
    ]
    out = []
    seen = set()

    def add(year, title):
        if not title or not year:
            return
        key = (year, title.casefold())
        if key not in seen:
            seen.add(key)
            out.append({'year': year, 'title': title})

    studio_rows = table_releases(sections[0])
    for year, title in studio_rows:
        add(year, title)
    if not studio_rows:
        # Many discographies use a bullet list instead of a table (Dennis Brown,
        # for example). Without this branch the scraper silently returns zero
        # albums and the catalog falls back to every YTM re-release.
        for item in re.findall(r'<li[^>]*>(.*?)</li>', sections[0], re.S):
            rec = album_from_line(text(item))
            if rec:
                add(rec['year'], rec['title'])
    for sec in sections[1:]:
        for year, title in table_releases(sec):
            add(year, title)
    return out


def singles(page):
    sec = section(wiki_html(page, need=('Singles',)),
                  'Singles', ('Music_videos', 'Other_appearances'))
    out = []
    seen = set()
    year = ''
    for table in re.findall(r'<table.*?</table>', sec, re.S):
        for row in re.findall(r'<tr.*?</tr>', table, re.S):
            c = cells(row)
            if len(c) < 2 or c[0].casefold() in ('year', 'title'):
                continue
            m = re.match(r'^\s*((?:19|20)\d{2})\s*$', c[0])
            if m:
                year = m.group(1)
                title_cell = next((x for x in c[1:3] if x.startswith('"')), '')
            else:
                title_cell = next((x for x in c[:2] if x.startswith('"')), '')
            title = htmlmod.unescape(title_cell)
            title = re.sub(r'\[\s*[A-Za-z]?\s*\d*\s*\]', '', title)
            title = title.replace('"', '').replace('\u201c', '').replace('\u201d', '')
            title = re.sub(r'\s*\([^)]*\b(?:featuring|feat\.?|with)\b[^)]*\)', '',
                           title, flags=re.I)
            title = re.sub(r'\s+', ' ', title).strip(' "\'')
            if not title or re.fullmatch(r'[\d—–\-\s]+', title):
                continue
            if not title or len(title) > 120:
                continue
            key = title.casefold()
            if key not in seen:
                seen.add(key)
                out.append({'year': year, 'title': title})
    return out


if __name__ == '__main__':
    page = sys.argv[1] if len(sys.argv) > 1 else 'Marvin Gaye discography'
    print('ALBUMS', len(albums(page)))
    for a in albums(page):
        print(a['year'], '|', a['title'])
    s = singles(page)
    print('SINGLES', len(s))
    for x in s[:15]:
        print(x['year'], '|', x['title'])
