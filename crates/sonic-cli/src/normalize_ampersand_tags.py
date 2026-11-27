#!/usr/bin/env python3
"""Normalize ampersand-separated artist credits across a music library.

Rules:
  * album artist (`ALBUMARTIST` / `album_artist`): `A & B` -> `A`
  * artist (`ARTIST` / `artist`):               `A & B` -> `A, B`
  * album title is never touched.

Uses sonic-cli's hidden `--tag-file` path so sonic-core performs the rewrite
and keeps Opus cover art. Run without flags to write a dry-run plan; review it,
then rerun with `--apply-plan`.

Examples:
  normalize_ampersand_tags.py --plan /tmp/credits.json
  normalize_ampersand_tags.py --apply-plan /tmp/credits.json --apply
"""
import argparse
import concurrent.futures
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

try:
    import ytm_config
    DEFAULT_CLI = str(ytm_config.CLI)
    DEFAULT_ROOT = str(ytm_config.music_root())
except Exception:
    DEFAULT_CLI = os.environ.get('SONIC_CLI', 'sonic-cli')
    DEFAULT_ROOT = os.environ.get('SONIC_MUSIC_ROOT', str(Path.home() / 'Music'))

EXTS = {'.opus', '.flac', '.mp3', '.m4a', '.ogg', '.wav', '.webm', '.mkv'}
FFPROBE = 'ffprobe'
SEP = ' & '

# Bands/duos whose ampersand is part of the name, not a separator between
# separate artist credits. Matched case-insensitively with flexible spacing.
PROTECTED_NAMES = (
    r'Bob Marley\s*&\s*The Wailers',
    r'Chaka Demus\s*&\s*Pliers',
    r'Clinton Fearon\s*&\s*Boogie Brown Band',
    r'Brick\s*&\s*Lace',
    r'Krept\s*&\s*Konan',
    r'Young T\s*&\s*Bugsey',
    r'Pete\s*&\s*Bas',
)
PROTECTED_PATTERNS = tuple(re.compile(p, re.IGNORECASE)
                           for p in PROTECTED_NAMES)


def _mask_protected(value):
    """Replace protected band names with tokens before splitting on ' & '."""
    held = {}
    work = str(value)
    for i, pattern in enumerate(PROTECTED_PATTERNS):
        def keep(match, i=i):
            token = f'<<PROTECTED_{i}>>'
            held[token] = re.sub(r'\s*&\s*', ' & ', match.group(0))
            return token
        work = pattern.sub(keep, work)
    return work, held


def _unmask_protected(value, held):
    for token, original in held.items():
        value = value.replace(token, original)
    return value


def split_album_artist(value):
    """`A & B` -> `A`, unless the whole credit is a protected band/duo."""
    work, held = _mask_protected(value)
    if SEP in work:
        work = work.split(SEP, 1)[0].strip()
    return _unmask_protected(work, held)


def split_artist(value):
    """`A & B` -> `A, B`, but keep protected band/duo names intact."""
    work, held = _mask_protected(value)
    work = work.replace(SEP, ', ')
    work = re.sub(r',\s+', ', ', work)
    return _unmask_protected(work, held)


def key_id(key):
    """`ALBUMARTIST`, `album_artist` and `album-artist` are the same key."""
    return ''.join(c.lower() for c in str(key) if c.isalnum())


ALBUM_ARTIST_IDS = {'albumartist'}
ARTIST_IDS = {'artist'}


def audio_files(root):
    for base, dirs, names in os.walk(root):
        for name in names:
            if Path(name).suffix.lower() in EXTS:
                yield Path(base) / name


def probe(path):
    try:
        p = subprocess.run(
            [FFPROBE, '-v', 'error', '-select_streams', 'a:0',
             '-show_entries', 'stream_tags:format_tags', '-of', 'json',
             str(path)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, timeout=30)
        data = json.loads(p.stdout or '{}')
    except Exception as exc:
        return path, None, f'probe failed: {exc}'

    groups = {}
    fmt = data.get('format') or {}
    groups.update((fmt.get('tags') or {}).items())
    streams = data.get('streams') or []
    if streams:
        groups.update((streams[0].get('tags') or {}).items())
    return path, groups, None


def plan_for(path, tags):
    """Return (updates, unhandled) for one file."""
    updates, unhandled = [], []
    by_id = {}
    for key, value in (tags or {}).items():
        if not isinstance(value, str) or '&' not in value:
            continue
        by_id.setdefault(key_id(key), []).append((key, value))

    for kind, idents in (('albumartist', ALBUM_ARTIST_IDS),
                         ('artist', ARTIST_IDS)):
        matching = [pair for ident in idents
                    for pair in by_id.get(ident, [])]
        for key, value in matching:
            if SEP not in value:
                unhandled.append({'file': str(path), 'key': key,
                                  'value': value, 'kind': kind})
                continue
            new = (split_album_artist(value) if kind == 'albumartist'
                   else split_artist(value))
            if not new or new == value:
                continue
            updates.append({'file': str(path), 'key': key, 'old': value,
                            'new': new, 'kind': kind})
    return updates, unhandled


def write_plan(args):
    files = list(audio_files(Path(args.root)))
    plan, unhandled, errors = [], [], []
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        for path, tags, error in pool.map(probe, files):
            if error:
                errors.append({'file': str(path), 'error': error})
                continue
            updates, odd = plan_for(path, tags)
            plan.extend(updates)
            unhandled.extend(odd)

    st = {}
    for rec in plan:
        path = Path(rec['file'])
        s = path.stat()
        st[str(path)] = [s.st_size, s.st_mtime_ns]
    out = {
        'root': str(Path(args.root).resolve()),
        'cli': args.cli,
        'created': time.strftime('%Y-%m-%dT%H:%M:%S'),
        'files_scanned': len(files),
        'updates': plan,
        'unhandled': unhandled,
        'errors': errors,
        'stat': st,
    }
    Path(args.plan).write_text(json.dumps(out, indent=2, ensure_ascii=False),
                               encoding='utf-8')
    print(f'scanned {len(files)} file(s) in {time.monotonic() - started:.1f}s')
    print(f'planned {len(plan)} tag change(s) in '
          f'{len(set(r["file"] for r in plan))} file(s)')
    print(f'unhandled ampersand forms: {len(unhandled)}')
    print(f'probe errors: {len(errors)}')
    print(f'plan: {args.plan}')
    by_kind = {}
    for rec in plan:
        by_kind.setdefault(rec['kind'], []).append(rec)
    for kind in ('albumartist', 'artist'):
        recs = by_kind.get(kind, [])
        print(f'\n{kind}: {len(recs)} change(s)')
        seen = set()
        for rec in recs:
            pair = (rec['old'], rec['new'])
            if pair in seen:
                continue
            seen.add(pair)
            print(f'  {rec["old"]!r} -> {rec["new"]!r}')
            if len(seen) >= 25:
                if len(set((r['old'], r['new']) for r in recs)) > 25:
                    print('  ...')
                break
    if unhandled:
        print('\nUnhandled examples (left unchanged):')
        for rec in unhandled[:20]:
            print(f'  {rec["kind"]}: {rec["value"]!r}  ({rec["file"]})')


def apply_plan(args):
    data = json.loads(Path(args.plan).read_text(encoding='utf-8'))
    plan = data.get('updates') or []
    cli = args.cli or data.get('cli') or DEFAULT_CLI
    stat = data.get('stat') or {}

    # Group by file so a file needing two tag changes is rewritten once.
    by_file = {}
    for rec in plan:
        by_file.setdefault(rec['file'], []).append(rec)

    stale = []
    for path, recs in by_file.items():
        s = Path(path).stat()
        if stat.get(path) != [s.st_size, s.st_mtime_ns]:
            stale.append(path)
    if stale:
        print(f'{len(stale)} file(s) changed since the plan was made; '
              'rerun the plan.', file=sys.stderr)
        for path in stale[:10]:
            print(f'  {path}', file=sys.stderr)
        return 2

    if not args.apply:
        print(f'{len(plan)} change(s) ready; pass --apply to write them.')
        return 0

    ok = failed = 0
    started = time.monotonic()
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {}
        for path, recs in by_file.items():
            # Combine both changes into one sonic-cli call.
            futures[pool.submit(retag_one_group, recs, cli, path)] = path
        for future in concurrent.futures.as_completed(futures):
            path = futures[future]
            good, message = future.result()
            if good:
                ok += 1
            else:
                failed += 1
                print(f'FAILED {path}: {message}', file=sys.stderr)
    print(f'updated {ok} file(s); {failed} failure(s) in '
          f'{time.monotonic() - started:.1f}s')

    if not args.no_verify:
        bad = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
            futures = {pool.submit(probe, Path(path)): path for path in by_file}
            for future in concurrent.futures.as_completed(futures):
                path = futures[future]
                _, tags, error = future.result()
                if error:
                    bad.append((path, error))
                    continue
                current = {key_id(k): v for k, v in (tags or {}).items()}
                for rec in by_file[path]:
                    if current.get(key_id(rec['key'])) != rec['new']:
                        bad.append((path, f'{rec["key"]}: expected '
                                    f'{rec["new"]!r}, got '
                                    f'{current.get(key_id(rec["key"]))!r}'))
        if bad:
            print(f'verification failed for {len(bad)} check(s):',
                  file=sys.stderr)
            for path, message in bad[:20]:
                print(f'  {path}: {message}', file=sys.stderr)
            return 1
        print(f'verified {len(by_file)} file(s)')
    return 1 if failed else 0


def correct_plan(args):
    """Re-fix an already-applied plan using the protected-name rules.

    The original plan stores each tag's pre-change value, so the corrected
    target can be recomputed without rescanning the whole library.
    """
    data = json.loads(Path(args.plan).read_text(encoding='utf-8'))
    cli = args.cli or data.get('cli') or DEFAULT_CLI
    fixes = []
    for rec in data.get('updates') or []:
        want = (split_album_artist(rec['old']) if rec['kind'] == 'albumartist'
                else split_artist(rec['old']))
        if want != rec['new']:
            fixes.append({**rec, 'new': want})

    by_file = {}
    for rec in fixes:
        by_file.setdefault(rec['file'], []).append(rec)
    print(f'{len(fixes)} tag(s) in {len(by_file)} file(s) need correction')

    if not args.apply:
        for rec in fixes[:25]:
            print(f'  {rec["kind"]}: {rec["new"]!r}  ({rec["file"]})')
        print('pass --apply to write corrections')
        return 0

    ok = failed = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(retag_one_group, recs, cli, path): path
                   for path, recs in by_file.items()}
        for future in concurrent.futures.as_completed(futures):
            path = futures[future]
            good, message = future.result()
            if good:
                ok += 1
            else:
                failed += 1
                print(f'FAILED {path}: {message}', file=sys.stderr)

    print(f'corrected {ok} file(s); {failed} failure(s)')
    if args.no_verify:
        return 1 if failed else 0

    bad = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = {pool.submit(probe, Path(path)): path for path in by_file}
        for future in concurrent.futures.as_completed(futures):
            path = futures[future]
            _, tags, error = future.result()
            if error:
                bad.append((path, error))
                continue
            current = {key_id(k): v for k, v in (tags or {}).items()}
            for rec in by_file[path]:
                got = current.get(key_id(rec['key']))
                if got != rec['new']:
                    bad.append((path, f'{rec["key"]}: expected '
                                f'{rec["new"]!r}, got {got!r}'))
    if bad:
        print(f'verification failed for {len(bad)} check(s):', file=sys.stderr)
        for path, message in bad[:20]:
            print(f'  {path}: {message}', file=sys.stderr)
        return 1
    print(f'verified {len(by_file)} corrected file(s)')
    return 1 if failed else 0


def retag_one_group(recs, cli, path):
    cmd = [cli, '--tag-file', str(path)]
    for rec in recs:
        cmd += ['--tag', f'{rec["key"]}={rec["new"]}']
    try:
        p = subprocess.run(cmd, stdout=subprocess.PIPE,
                           stderr=subprocess.STDOUT, text=True, timeout=180)
    except Exception as exc:
        return False, f'{type(exc).__name__}: {exc}'
    if p.returncode != 0:
        return False, (p.stdout or '').strip().splitlines()[-1][:300]
    return True, ''


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--root', default=DEFAULT_ROOT, help='library root')
    ap.add_argument('--cli', default=DEFAULT_CLI, help='sonic-cli binary')
    ap.add_argument('--plan', default='/path/to/artist-credit-plan.json',
                    help='where to write/read the dry-run plan')
    ap.add_argument('--workers', type=int, default=8, help='parallel probes/retags')
    ap.add_argument('--apply-plan', action='store_true',
                    help='apply an existing plan instead of scanning')
    ap.add_argument('--correct-plan', action='store_true',
                    help='re-fix an applied plan using protected band/duo names')
    ap.add_argument('--apply', action='store_true',
                    help='actually write tags (required with --apply-plan)')
    ap.add_argument('--no-verify', action='store_true',
                    help='skip post-apply ffprobe verification')
    args = ap.parse_args()
    if args.correct_plan:
        return correct_plan(args)
    if args.apply_plan:
        return apply_plan(args)
    return write_plan(args)


if __name__ == '__main__':
    sys.exit(main())
