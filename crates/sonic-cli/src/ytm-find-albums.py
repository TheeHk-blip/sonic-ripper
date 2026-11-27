#!/usr/bin/env python3
"""Resolve YouTube Music OLAK5uy playlist IDs for an artist's albums.

Usage:
  find_ytm_albums.py --artist "Marvin Gaye" --albums albums.tsv --output out.json

albums.tsv has one album per line: YEAR<TAB>ALBUM TITLE
"""
import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ytm_config
from urllib.parse import quote

BAD = ('tribute', 'karaoke', 'remix', 'mixed', 'dj ', 'mix ', 'covers',
       'instrumental', 'smooth jazz', 'relaxing')


def run(ytdlp, args, timeout=40):
    return subprocess.run([ytdlp, '--no-warnings', *args],
                          capture_output=True, text=True, timeout=timeout)


def find_albums(ytdlp, artist, albums, max_browses=8):
    results = {}
    for i, (year, title) in enumerate(albums, 1):
        print(f'[{i}/{len(albums)}] {title} ({year})', flush=True)
        # Search both with and without the year: YTM's ranking differs and
        # each form misses some original albums.
        browses = []
        for query in (f'{artist} {title} {year}', f'{artist} {title}'):
            p = run(ytdlp, ['--flat-playlist', '--print', '%(title)s\t%(url)s',
                            f'https://music.youtube.com/search?q={quote(query)}'])
            for line in p.stdout.splitlines():
                if '\t' not in line:
                    continue
                url = line.split('\t', 1)[1]
                if '/browse/MPREb' in url and url not in browses:
                    browses.append(url)
        candidates = []
        for url in browses[:max_browses]:
            p2 = run(ytdlp, ['--flat-playlist', '--print',
                             '%(playlist_id)s\t%(playlist_title)s', url])
            first = next((l for l in p2.stdout.splitlines() if '\t' in l), '')
            if not first:
                continue
            pid, ptitle = first.split('\t', 1)
            if not pid.startswith('OLAK5uy'):
                continue
            if any(b in ptitle.casefold() for b in BAD):
                continue
            if (pid, ptitle) not in candidates:
                candidates.append((pid, ptitle))
        results[title] = candidates
        for cid, ctitle in candidates:
            print(f'    {cid} | {ctitle}', flush=True)
        if not candidates:
            print('    NO ALBUM CANDIDATE', flush=True)
    return results


def load_albums(path):
    albums = []
    for line in open(path, encoding='utf-8'):
        line = line.rstrip('\n')
        if not line or line.startswith('#'):
            continue
        parts = line.split('\t', 1) if '\t' in line else line.split('|', 1)
        if len(parts) != 2:
            raise ValueError(f'bad album line: {line!r}')
        albums.append((parts[0].strip(), parts[1].strip()))
    return albums


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--artist', required=True, help='Artist name for searches')
    ap.add_argument('--albums', required=True,
                    help='TSV file of YEAR<TAB>ALBUM TITLE lines')
    ap.add_argument('--output', required=True, help='JSON output path')
    ap.add_argument('--ytdlp', default=str(ytm_config.YTDLP))
    args = ap.parse_args()
    albums = load_albums(args.albums)
    results = find_albums(args.ytdlp, args.artist, albums)
    with open(args.output, 'w', encoding='utf-8') as f:
        json.dump(results, f, indent=2)
    print(f'wrote {args.output} ({len(results)} albums)', flush=True)


if __name__ == '__main__':
    main()
