#!/usr/bin/env python3
"""Validate album candidates: track count + first-track metadata from YTM."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ytm_config


def run(ytdlp, args, timeout=40):
    return subprocess.run([ytdlp, '--no-warnings', *args],
                          capture_output=True, text=True, timeout=timeout)


def describe(ytdlp, pid):
    url = f'https://music.youtube.com/playlist?list={pid}'
    p = run(ytdlp, ['--flat-playlist', '--print', '%(title)s\t%(url)s', url])
    lines = [l for l in p.stdout.splitlines() if '\t' in l]
    if not lines:
        return {'tracks': 0}
    first_title, first_url = lines[0].split('\t', 1)
    m = run(ytdlp, ['--extractor-args', 'youtube:player_client=web_music',
                    '--print', '%(title)s\t%(album)s\t%(date)s\t%(artist)s', first_url])
    parts = (m.stdout.splitlines()[0] if m.stdout else '').split('\t')
    parts += [''] * (4 - len(parts))
    return {'tracks': len(lines), 'title': parts[0] or first_title,
            'album': parts[1], 'date': parts[2], 'artist': parts[3]}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--candidates', required=True)
    ap.add_argument('--output', required=True)
    ap.add_argument('--ytdlp', default=str(ytm_config.YTDLP))
    args = ap.parse_args()
    candidates = json.load(open(args.candidates, encoding='utf-8'))
    out = {}
    for album, cands in candidates.items():
        print(f'=== {album}', flush=True)
        out[album] = []
        for pid, ptitle in cands:
            try:
                d = describe(args.ytdlp, pid)
            except Exception as e:
                print(f'{pid} ERROR {e}', flush=True)
                continue
            d.update({'id': pid, 'playlist': ptitle})
            out[album].append(d)
            print(f"  {pid} | {ptitle}\n    tracks={d['tracks']} "
                  f"album={d.get('album')!r} date={d.get('date')!r} "
                  f"artist={d.get('artist')!r}", flush=True)
    json.dump(out, open(args.output, 'w', encoding='utf-8'), indent=2)
    print(f'wrote {args.output}', flush=True)


if __name__ == '__main__':
    main()
