#!/usr/bin/env python3
"""Resolve artist singles to exact YouTube Music watch URLs.

Usage:
  resolve_singles.py --artist "Marvin Gaye" --singles singles.json \
      --album-tracks tracks.json --output resolved.json

singles.json: [{"title": "...", "year": "..."}, ...]
album-tracks.json: {"album folder": ["track title", ...], ...} (optional)
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ytm_config
from urllib.parse import quote

BAD = ('karaoke', 'cover', 'tribute', 'remix', 'instrumental', 'reaction',
       'parody', 'full album', 'vinyl', 'dj ')


def norm(s):
    s = s.casefold()
    s = re.sub(r'\([^)]*\)', '', s)
    s = s.replace('&', ' and ').replace("'", '').replace('’', '')
    s = re.sub(r'\b(?:pt|part)\s*\d+\b', '', s)
    s = re.sub(r'[^a-z0-9]+', ' ', s)
    return ' '.join(s.split())


def run(ytdlp, args, timeout=30):
    return subprocess.run([ytdlp, '--no-warnings', *args],
                          capture_output=True, text=True, timeout=timeout)


def search(ytdlp, artist, song):
    query = f'{artist} {song}'
    p = run(ytdlp, ['--flat-playlist', '--print', '%(title)s\t%(url)s',
                    f'https://music.youtube.com/search?q={quote(query)}'])
    return [(line.split('\t', 1)[0], line.split('\t', 1)[1])
            for line in p.stdout.splitlines()
            if '\t' in line and 'music.youtube.com/watch?v=' in line]


def metadata(ytdlp, url):
    p = run(ytdlp, ['--extractor-args', 'youtube:player_client=web_music',
                    '--print', '%(title)s\t%(artist)s\t%(album)s\t%(channel)s', url])
    parts = (p.stdout.splitlines()[0] if p.stdout else '').split('\t')
    parts += [''] * (4 - len(parts))
    return dict(zip(('title', 'artist', 'album', 'channel'), parts[:4]))


def rank(title, wanted, artist):
    h = norm(title)
    if h == wanted:
        return 0
    low = title.casefold()
    if any(b in low for b in BAD):
        return 99
    if artist.casefold() not in low and h != wanted:
        return 3
    if h.startswith(wanted + ' ') or wanted.startswith(h + ' '):
        return 1
    if len(wanted) >= 8 and wanted in h:
        return 2
    return 99


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--artist', required=True)
    ap.add_argument('--singles', required=True)
    ap.add_argument('--album-tracks')
    ap.add_argument('--output', required=True)
    ap.add_argument('--ytdlp', default=str(ytm_config.YTDLP))
    args = ap.parse_args()

    singles = json.load(open(args.singles, encoding='utf-8'))
    album_tracks = set()
    if args.album_tracks:
        for titles in json.load(open(args.album_tracks, encoding='utf-8')).values():
            album_tracks.update(norm(t) for t in titles)

    out = []
    for i, s in enumerate(singles, 1):
        title, wanted = s['title'], norm(s['title'])
        if wanted in album_tracks:
            print(f'[{i}/{len(singles)}] SKIP album track: {title}', flush=True)
            continue
        hits = sorted(((rank(t, wanted, args.artist), t, u)
                       for t, u in search(args.ytdlp, args.artist, title)),
                      key=lambda x: x[0])
        candidates = [(t, u) for r, t, u in hits if r < 99][:3]
        if not candidates:
            print(f'[{i}/{len(singles)}] UNRESOLVED: {title}', flush=True)
            out.append({'title': title, 'year': s.get('year'), 'url': None})
            continue
        chosen = None
        for htitle, url in candidates:
            meta = metadata(args.ytdlp, url)
            if args.artist.casefold() in (meta['artist'] + ' ' + meta['channel']).casefold():
                chosen = {'title': title, 'year': s.get('year'), 'url': url,
                          'source_title': htitle, 'meta': meta}
                break
        if not chosen:
            htitle, url = candidates[0]
            chosen = {'title': title, 'year': s.get('year'), 'url': url,
                      'source_title': htitle, 'meta': metadata(args.ytdlp, url)}
        print(f"[{i}/{len(singles)}] {title} -> {chosen['url']} "
              f"({chosen['meta']['artist']} | {chosen['meta']['album']})", flush=True)
        out.append(chosen)
    json.dump(out, open(args.output, 'w', encoding='utf-8'), indent=2)
    print(f"wrote {args.output} ({sum(bool(x['url']) for x in out)} URLs)", flush=True)


if __name__ == '__main__':
    main()
