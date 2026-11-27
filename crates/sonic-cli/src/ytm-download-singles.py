#!/usr/bin/env python3
"""Download resolved YTM single URLs into an artist's SINGLES folder."""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ytm_config
from opus_settags import set_tags


def sanitize(s):
    return re.sub(r'[\\/:*?"<>|]', '_', s).strip()


def norm(s):
    s = s.casefold()
    s = re.sub(r'\([^)]*\)', '', s)
    s = s.replace('&', ' and ').replace("'", '').replace('’', '')
    s = re.sub(r'[^a-z0-9]+', ' ', s)
    return ' '.join(s.split())


def tag(path, name):
    p = subprocess.run(['ffprobe', '-v', 'error',
                        '-show_entries', f'stream_tags={name}',
                        '-of', 'default=noprint_wrappers=1:nokey=1', str(path)],
                       capture_output=True, text=True)
    lines = p.stdout.strip().splitlines()
    return lines[0] if lines else ''


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--resolved', required=True)
    ap.add_argument('--artist', required=True, help='Artist name for filename fallback')
    ap.add_argument('--root', required=True, help='Artist folder')
    ap.add_argument('--log', default='/tmp/ytm_single_downloads.log')
    ap.add_argument('--cli', default=str(ytm_config.CLI))
    ap.add_argument('--cookies', default=str(ytm_config.COOKIES))
    ap.add_argument('--format', default='opus')
    ap.add_argument('--album-tracks', help='Skip singles already present in albums')
    args = ap.parse_args()

    album_titles = set()
    if args.album_tracks:
        for titles in json.load(open(args.album_tracks, encoding='utf-8')).values():
            album_titles.update(norm(t) for t in titles)

    singles = [s for s in json.load(open(args.resolved, encoding='utf-8')) if s.get('url')]
    root = Path(args.root)
    dest = root / 'SINGLES'
    dest.mkdir(parents=True, exist_ok=True)
    base = [args.cli, '--ytdlp-clients', 'web_music', '--cookies', args.cookies,
            '--format', args.format, '--naming', 'artist_title',
            '--output-dir', str(dest), '--skip-missing']

    with open(args.log, 'a', encoding='utf-8') as log:
        for i, s in enumerate(singles, 1):
            title = s['title']
            if norm(title) in album_titles:
                print(f'[{i}/{len(singles)}] SKIP album track: {title}', file=log, flush=True)
                continue
            meta = s.get('meta', {})
            artist = meta.get('artist') or args.artist
            existing = [f for f in dest.glob('*.opus') if title.casefold() in f.name.casefold()]
            if existing:
                print(f'[{i}/{len(singles)}] SKIP exists: {title}', file=log, flush=True)
                continue
            before = set(dest.glob('*.opus'))
            print(f'[{i}/{len(singles)}] {title} -> {s["url"]}', file=log, flush=True)
            p = subprocess.run(base + ['--url', s['url']], capture_output=True, text=True)
            new = set(dest.glob('*.opus')) - before
            if not new:
                err = '\n'.join((p.stdout + p.stderr).splitlines()[-6:])
                print(f'  FAILED: {err}', file=log, flush=True)
                continue
            for f in sorted(new):
                updates = {}
                if artist and artist != tag(f, 'artist'):
                    updates['artist'] = artist
                if title and title != tag(f, 'title'):
                    updates['title'] = title
                if updates:
                    tmp = f.with_suffix('.patched.opus')
                    tmp.write_bytes(set_tags(f.read_bytes(), updates))
                    tmp.replace(f)
                target = dest / f'{sanitize(artist)} - {sanitize(title)}.opus'
                if f != target and not target.exists():
                    f.rename(target)
                print(f'  OK: {target.name}', file=log, flush=True)
    print(f'log: {args.log}', flush=True)


if __name__ == '__main__':
    main()
