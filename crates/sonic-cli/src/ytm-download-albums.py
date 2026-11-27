#!/usr/bin/env python3
"""Download YTM album playlists into an artist folder, then export track titles."""
import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ytm_config


def ffprobe_tag(path, tag):
    p = subprocess.run(['ffprobe', '-v', 'error',
                        '-show_entries', f'stream_tags={tag}',
                        '-of', 'default=noprint_wrappers=1:nokey=1', str(path)],
                       capture_output=True, text=True)
    lines = p.stdout.strip().splitlines()
    return lines[0] if lines else ''


def folder_titles(folder):
    return [ffprobe_tag(f, 'title') for f in sorted(folder.glob('*.opus'))
            if ffprobe_tag(f, 'title')]


def playlist_titles(ytdlp, pid):
    p = subprocess.run([ytdlp, '--no-warnings', '--flat-playlist',
                        '--print', '%(title)s',
                        f'https://music.youtube.com/playlist?list={pid}'],
                       capture_output=True, text=True, timeout=40)
    return [l for l in p.stdout.splitlines() if l.strip()]


def already_present(root, titles):
    if not titles:
        return False
    want = set(t.casefold() for t in titles)
    for folder in root.iterdir():
        if not folder.is_dir() or folder.name == 'SINGLES':
            continue
        have = set(t.casefold() for t in folder_titles(folder))
        if len(want & have) / len(want) >= 0.7:
            return True
    return False


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--map', required=True, help='JSON: album title -> OLAK playlist ID')
    ap.add_argument('--root', required=True, help='Artist folder')
    ap.add_argument('--tracks-output', required=True)
    ap.add_argument('--log', default='/tmp/ytm_album_downloads.log')
    ap.add_argument('--cli', default=str(ytm_config.CLI))
    ap.add_argument('--ytdlp', default=str(ytm_config.YTDLP))
    ap.add_argument('--cookies', default=str(ytm_config.COOKIES))
    ap.add_argument('--format', default='opus')
    args = ap.parse_args()

    album_map = json.load(open(args.map, encoding='utf-8'))
    root = Path(args.root)
    root.mkdir(parents=True, exist_ok=True)
    base = [args.cli, '--ytdlp-clients', 'web_music', '--cookies', args.cookies,
            '--format', args.format, '--is-album', '--folder-pattern', 'year_album',
            '--naming', 'number_title', '--output-dir', str(root), '--skip-missing']

    tracks = {}
    for folder in root.iterdir():
        if folder.is_dir() and folder.name != 'SINGLES':
            names = folder_titles(folder)
            if names:
                tracks[folder.name] = names

    with open(args.log, 'a', encoding='utf-8') as log:
        for i, (album, pid) in enumerate(album_map.items(), 1):
            titles = playlist_titles(args.ytdlp, pid)
            if already_present(root, titles):
                print(f'[{i}/{len(album_map)}] SKIP already present: {album}',
                      file=log, flush=True)
                continue
            print(f'[{i}/{len(album_map)}] {album} ({pid}, {len(titles)} tracks)',
                  file=log, flush=True)
            p = subprocess.run(base + ['--url',
                               f'https://music.youtube.com/playlist?list={pid}'],
                               capture_output=True, text=True)
            out = p.stdout + p.stderr
            lines = [l for l in out.splitlines()
                     if 'Done:' in l or '[error]' in l or '[warn]' in l]
            print('\n'.join(lines[-12:]) or out[-1500:], file=log, flush=True)
            print(f'exit={p.returncode}', file=log, flush=True)

    for folder in root.iterdir():
        if folder.is_dir() and folder.name != 'SINGLES':
            names = folder_titles(folder)
            if names:
                tracks[folder.name] = names
    json.dump(tracks, open(args.tracks_output, 'w', encoding='utf-8'), indent=2)
    print(f'wrote {args.tracks_output} ({len(tracks)} albums)', flush=True)


if __name__ == '__main__':
    main()
