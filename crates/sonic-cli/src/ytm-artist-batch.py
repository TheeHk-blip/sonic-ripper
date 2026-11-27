#!/usr/bin/env python3
"""Incremental YouTube Music artist batch downloader.

Albums and singles are discovered, validated, downloaded and recorded one at
a time. Re-running the same command skips anything already downloaded.

Usage:
  ytm-artist-batch.py --artist 'Marvin Gaye' \
      --albums albums.tsv --singles singles.json \
      --root '/path/to/MUSIC/SOUL/MARVIN GAYE' \
      --state /path/to/state.json
"""
import argparse
import atexit
import ast
import fcntl
import json
import copy
from concurrent.futures import ThreadPoolExecutor
import os
import signal
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from pathlib import Path
from urllib.parse import quote

sys.path.insert(0, str(Path(__file__).resolve().parent))
from wiki_discovery import albums as fetch_wiki_albums, singles as fetch_wiki_singles
import ytm_config
import ytm_index

DEFAULT_YTDLP = str(ytm_config.YTDLP)
DEFAULT_CLI = str(ytm_config.CLI)
DEFAULT_COOKIES = str(ytm_config.COOKIES)
BAD = ('tribute', 'karaoke', 'remix', 'mixed', 'dj ', 'mix ', 'covers',
       'instrumental', 'smooth jazz', 'relaxing', 'full album',
       'best of', 'very best', 'greatest hits', 'anthology',
       'compilation', 'collection', 'classic albums', 'box set',
       'the complete', 'ultimate', 'definitive')
AUDIO_EXTS = ('opus', 'mp3', 'flac', 'm4a', 'wav', 'ogg', 'mp4', 'webm', 'mkv')
TAG_FIELDS = ('title', 'date', 'release_type', 'musicbrainz_albumtype',
              'explicit', 'itunesadvisory')
RETAG_WORKERS = 4
# YTM serves Opus-in-WebM around 300 kbit/s. Flat playlist durations times this
# give a good pre-download total without a per-track metadata call.
SOURCE_BPS = 37500
DEFAULT_ALBUM_BYTES = 45 * 1024 * 1024
DEFAULT_SINGLE_BYTES = 5 * 1024 * 1024


from progress_ui import Display, PROGRESS
from checkbox_select import (Choice, SelectionCancelled, choose_run_mode,
                             select_items)

RETAG_CLI = DEFAULT_CLI
RETAG_CONFIG_DIR = None
DISPLAY = None
ABORT = threading.Event()
CHILD_PROCS = set()
_CHILD_LOCK = threading.Lock()
TEMP_DIRS = set()
_TEMP_LOCK = threading.Lock()
STATE_LOCK = threading.RLock()
RUN_CONTEXT = threading.local()
# One ffprobe JSON per file per run replaces a separate process for every tag
# lookup. The (size, mtime_ns) signature invalidates it after a download/retag.
TAG_CACHE = {}


def log(text='', flush=True):
    PROGRESS.message(text)


def audio_files(folder):
    return sorted(f for f in folder.iterdir()
                  if f.is_file() and f.suffix.lower().lstrip('.') in AUDIO_EXTS)


def is_library_folder(folder):
    """Skip SINGLES and in-flight .download-* work dirs during library scans.

    Album jobs share the artist root with their private work dirs, so scanning
    root/EP while other jobs run must ignore those transient directories (they
    may disappear between is_dir() and iterdir()).
    """
    return (folder.is_dir() and folder.name != 'SINGLES'
            and not folder.name.startswith('.download-'))


def remove_new_dirs(parent, before):
    """Remove directories a failed download created, so reruns do not skip them."""
    if not parent.is_dir():
        return
    after = set(f.name for f in parent.iterdir() if f.is_dir())
    for name in after - before:
        shutil.rmtree(parent / name, ignore_errors=True)


def parse_titles(values, flag):
    out = []
    for value in values or []:
        value = value.strip()
        if not value:
            continue
        if value.startswith('['):
            try:
                data = json.loads(value)
            except json.JSONDecodeError as e:
                raise SystemExit(f'{flag} JSON error: {e}')
            if not isinstance(data, list):
                raise SystemExit(f'{flag} JSON must be an array of titles')
            out += [str(x).strip() for x in data if str(x).strip()]
        else:
            out += [x.strip() for x in value.split(',') if x.strip()]
    return list(dict.fromkeys(out))


def parse_songs(values):
    return parse_titles(values, '--songs')


def album_request_keys(value):
    value = str(value).strip()
    keys = {norm(value), album_key(value)}
    pid = playlist_id_from(value)
    if pid:
        keys.add(pid)
    m = re.match(r'^\s*(?:(?:19|20)\d{2})\s*[-–_: ]\s*(.+)$', value)
    if m:
        keys.update((norm(m.group(1)), album_key(m.group(1))))
    keys.discard('')
    return keys


def album_entry_keys(entry):
    year, title, pinned = entry
    keys = {norm(title), album_key(title),
            norm(f'{year} {title}'), album_key(f'{year} - {title}')}
    if pinned:
        keys.add(pinned)
        pid = playlist_id_from(pinned)
        if pid:
            keys.add(pid)
    keys.discard('')
    return keys


def loose_list(text):
    """Best-effort parser for bracket lists whose quotes the shell removed."""
    text = text.strip()
    if text.startswith('[') and text.endswith(']'):
        text = text[1:-1]
    parts, buf, depth = [], '', 0
    for ch in text:
        if ch == '[':
            depth += 1
            buf += ch
        elif ch == ']':
            depth -= 1
            buf += ch
        elif ch == ',' and depth == 0:
            parts.append(buf)
            buf = ''
        else:
            buf += ch
    parts.append(buf)
    out = []
    for part in parts:
        part = part.strip()
        if part.startswith('[') and part.endswith(']'):
            out.append(loose_list(part))
        else:
            out.append(part.strip(' \'"'))
    return out


def parse_album_matrix(values, flag):
    """Parse --albums[...] into one title list (or None for all) per artist."""
    result = []
    for value in values or []:
        try:
            data = json.loads(value)
        except json.JSONDecodeError:
            try:
                data = ast.literal_eval(value)
            except (ValueError, SyntaxError):
                data = loose_list(value)
        if not isinstance(data, list):
            raise SystemExit(f'{flag} must be a list of per-artist album lists')
        # A flat list with one artist is shorthand for that artist's selection.
        if data and not any(isinstance(x, (list, tuple)) for x in data):
            data = [data]
        for item in data:
            if item is None:
                result.append(None)
            elif isinstance(item, (list, tuple)):
                values = list(item)
                if (not values or
                        (len(values) == 1 and
                         (values[0] is None or
                          (isinstance(values[0], str) and
                           values[0].strip().casefold() in
                           ('', 'all', '*', 'null'))))):
                    result.append(None)
                else:
                    result.append([str(x).strip() for x in values if str(x).strip()])
            elif isinstance(item, str):
                text = item.strip()
                if text.casefold() in ('', 'all', '*', 'null'):
                    result.append(None)
                else:
                    result.append(parse_titles([text], flag))
            else:
                raise SystemExit(f'{flag}: each artist selection must be a list of titles')
    return result


def select_albums(albums, wanted):
    """Keep only requested albums. Accepts titles, indices, years, or playlist IDs."""
    expanded = []
    for value in wanted:
        text = str(value).strip()
        if re.fullmatch(r'\d+', text):
            idx = int(text) - 1
            if not 0 <= idx < len(albums):
                raise SystemExit(f'--select-albums: album index {text} out of range '
                                 f'(1-{len(albums)})')
            expanded.append(albums[idx][1])
        elif re.fullmatch(r'\d+\s*-\s*\d+', text):
            start, end = (int(x) for x in re.split(r'\s*-\s*', text))
            if not 1 <= start <= end <= len(albums):
                raise SystemExit(f'--select-albums: bad album range {text} '
                                 f'(1-{len(albums)})')
            expanded += [albums[i - 1][1] for i in range(start, end + 1)]
        else:
            expanded.append(text)
    wanted = expanded
    wanted_keys = [album_request_keys(w) for w in wanted]
    selected = [entry for entry in albums
                if any(k & album_entry_keys(entry) for k in wanted_keys)]
    missing = []
    for value, keys in zip(wanted, wanted_keys):
        if not any(keys & album_entry_keys(entry) for entry in selected):
            missing.append(value)
    if missing:
        available = ', '.join(t for _, t, _ in albums)
        raise SystemExit(
            f'--select-albums: no catalog match for: {", ".join(missing)}\n'
            f'Available albums: {available}')
    return selected


def cli_opts(args, extra=(), config_dir=None, concurrency=None):
    """Base sonic-cli flags shared by album and single downloads."""
    opts = [args.cli, '--config-dir', str(config_dir or args.cli_config_dir),
            '--in-memory-settings',
            '--binaries-dir',
            str(getattr(args, 'binaries_dir', ytm_config.TOOLS_DIR)),
            '--ytdlp-clients', 'web_music', '--cookies', args.cookies,
            '--format', args.format, '--bitrate', args.bitrate]
    if args.sample_rate:
        opts += ['--sample-rate', args.sample_rate]
    if args.video_quality:
        opts += ['--video-quality', args.video_quality]
    cli_args = shlex.split(args.cli_args or '')
    if concurrency is not None:
        filtered = []
        skip_value = False
        for value in cli_args:
            if skip_value:
                skip_value = False
            elif value == '--concurrency':
                skip_value = True
            elif not value.startswith('--concurrency='):
                filtered.append(value)
        cli_args = filtered + ['--concurrency', str(concurrency)]
    opts += cli_args
    return opts + list(extra)


def explicit_concurrency(cli_args):
    m = re.search(r'(?:^|\s)--concurrency(?:=|\s)+(\d+)', cli_args or '')
    return int(m.group(1)) if m else None


# Hard ceiling on in-flight track downloads. Album pool size * per-album track
# workers may never exceed this, so N*N process explosion cannot recur.
MAX_TOTAL_SLOTS = 16


def worker_slots(args):
    """Album-pool size for this artist (not the per-album track count)."""
    return (getattr(args, 'batch_slots', None)
            or getattr(args, 'worker_concurrency', None)
            or 6)


def track_slots(args):
    """Total tracks this artist can download at once."""
    workers = getattr(args, 'batch_track_concurrency', None) or 1
    return max(1, int(worker_slots(args)) * int(workers))


def split_evenly(total, count):
    base, extra = divmod(total, count)
    return [base + (1 if i < extra else 0) for i in range(count)]


def parse_rate(rate):
    m = re.fullmatch(r'(\d+(?:\.\d+)?)([KMGkmg]?)', rate)
    if not m:
        raise ValueError(rate)
    mult = {'': 1, 'K': 1024, 'M': 1024 ** 2, 'G': 1024 ** 3}
    return int(float(m.group(1)) * mult[m.group(2).upper()])


def format_rate(nbytes):
    for suffix, unit in (('M', 1024 ** 2), ('K', 1024)):
        if nbytes >= unit and nbytes % unit == 0:
            return f'{nbytes // unit}{suffix}'
    return str(nbytes)


def _global_paths(global_dir):
    d = Path(global_dir)
    return d / 'bandwidth.json', d / '.lock'


def _global_write(path, state):
    tmp = path.with_suffix('.tmp')
    tmp.write_text(json.dumps(state, indent=2), encoding='utf-8')
    os.replace(tmp, path)


def _global_edit(global_dir, callback):
    state_path, lock_path = _global_paths(global_dir)
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with open(lock_path, 'a+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = (json.loads(state_path.read_text(encoding='utf-8'))
                 if state_path.exists() else {'total': 0, 'workers': {}})
        result = callback(state)
        _global_write(state_path, state)
        fcntl.flock(lock, fcntl.LOCK_UN)
    return result


def _recompute_global(state):
    workers = state.setdefault('workers', {})
    active = [(wid, max(1, int(w.get('concurrency', 1))))
              for wid, w in workers.items() if w.get('active')]
    pool = active or [(wid, max(1, int(w.get('concurrency', 1))))
                      for wid, w in workers.items()]
    if not pool:
        return
    slots = sum(c for _, c in pool)
    per_slot = int(state['total']) // slots
    for wid, _ in pool:
        workers[wid]['rate'] = per_slot


def init_global_bandwidth(global_dir, total_rate, worker_ids, concurrency=None):
    concurrency = concurrency or {}
    _global_edit(global_dir, lambda st: st.update(
        total=int(total_rate),
        workers={wid: {'active': False,
                       'concurrency': int(concurrency.get(wid, 1)),
                       'rate': int(total_rate)} for wid in worker_ids}
    ) or _recompute_global(st))


def set_global_worker(global_dir, worker_id, active, concurrency=None):
    def edit(st):
        st.setdefault('workers', {}).setdefault(
            worker_id, {'active': False, 'concurrency': 1, 'rate': 0})
        st['workers'][worker_id]['active'] = bool(active)
        if concurrency:
            st['workers'][worker_id]['concurrency'] = int(concurrency)
        _recompute_global(st)
        return st['workers'][worker_id]['rate']
    return _global_edit(global_dir, edit)


def clear_global_worker(global_dir, worker_id):
    def edit(st):
        st.setdefault('workers', {}).pop(worker_id, None)
        _recompute_global(st)
    _global_edit(global_dir, edit)


_BW_LOCK = threading.Lock()
_BW_DEPTH = {}


def configure_ytdlp_rate(rate, state_dir, name):
    """Write a private yt-dlp config and return its XDG_CONFIG_HOME.

    The path is returned instead of exported so concurrent threads/workers can
    each pass their own environment to sonic-cli.
    """
    xdg_home = state_dir / f'{name}-ytdlp-config'
    cfg = xdg_home / 'yt-dlp' / 'config'
    cfg.parent.mkdir(parents=True, exist_ok=True)
    tmp = cfg.with_name(f'.config.{uuid.uuid4().hex}.tmp')
    with STATE_LOCK:
        tmp.write_text(f'--limit-rate {rate}\n', encoding='utf-8')
        os.replace(tmp, cfg)
    return xdg_home


def set_local_rate_env(args, rate, state_dir, name):
    xdg_home = configure_ytdlp_rate(format_rate(rate), state_dir, name)
    args.download_env = dict(getattr(args, 'download_env', {}) or {})
    args.download_env['XDG_CONFIG_HOME'] = str(xdg_home)
    return xdg_home


def start_download_bandwidth(args):
    """Mark this worker active and return per-download environment overrides."""
    global_dir = getattr(args, 'global_limit_dir', None)
    if not global_dir:
        return {}
    key = (str(global_dir), args.worker_id)
    with _BW_LOCK:
        _BW_DEPTH[key] = _BW_DEPTH.get(key, 0) + 1
    rate = set_global_worker(global_dir, args.worker_id, True,
                             track_slots(args))
    xdg_home = configure_ytdlp_rate(
        format_rate(rate),
        ytm_config.STATE_DIR,
        args.worker_id)
    return {'XDG_CONFIG_HOME': str(xdg_home)}


def end_download_bandwidth(args):
    """Release one slot; only the last active download deactivates the worker."""
    global_dir = getattr(args, 'global_limit_dir', None)
    if not global_dir:
        return
    key = (str(global_dir), args.worker_id)
    with _BW_LOCK:
        depth = _BW_DEPTH.get(key, 1) - 1
        if depth <= 0:
            _BW_DEPTH.pop(key, None)
            last = True
        else:
            _BW_DEPTH[key] = depth
            last = False
    if last:
        set_global_worker(global_dir, args.worker_id, False)


def register_child(proc):
    with _CHILD_LOCK:
        CHILD_PROCS.add(proc)


def unregister_child(proc):
    with _CHILD_LOCK:
        CHILD_PROCS.discard(proc)


def kill_all_children():
    with _CHILD_LOCK:
        procs = list(CHILD_PROCS)
    for proc in procs:
        kill_proc_group(proc)


def register_temp_dir(path):
    """Track a transient download directory so SIGTERM can remove it."""
    with _TEMP_LOCK:
        TEMP_DIRS.add(Path(path))


def unregister_temp_dir(path):
    with _TEMP_LOCK:
        TEMP_DIRS.discard(Path(path))


def cleanup_registered_temp_dirs():
    """Best-effort immediate cleanup before a forced process exit."""
    with _TEMP_LOCK:
        paths = list(TEMP_DIRS)
    for path in paths:
        shutil.rmtree(path, ignore_errors=True)


def install_signal_handlers(args):
    """One interrupt handler per process; safe with worker threads."""
    worker_ids = list(getattr(args, 'batch_worker_ids', None) or
                      ([args.worker_id] if getattr(args, 'worker_id', None) else []))
    global_dir = getattr(args, 'global_limit_dir', None)

    def clear_ledger():
        if global_dir:
            for wid in worker_ids:
                try:
                    clear_global_worker(global_dir, wid)
                except OSError:
                    pass

    if global_dir:
        atexit.register(clear_ledger)

    detached_slug = getattr(args, 'detached_run', None)
    if detached_slug:
        signal_state_dir = (Path(args.state).parent
                            if getattr(args, 'state', None) else None)
        signal_run_id = getattr(args, 'run_id', None)

    def on_signal(signum, _frame):
        ABORT.set()
        kill_all_children()
        # SIGTERM used to os._exit here, skipping every finally block and
        # leaving run-*/download dirs behind. Foreground Ctrl-C unwinds and
        # cleans them; do the same before a detached worker exits.
        cleanup_registered_temp_dirs()
        clear_ledger()
        if detached_slug:
            remove_detached_pidfile(detached_slug, signal_run_id,
                                    signal_state_dir)
        if signum == signal.SIGINT:
            raise KeyboardInterrupt
        os._exit(128 + signum)

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)


class TempByteMonitor:
    """Track the growth of sonic-cli's private Opus/WebM download files.

    A unique token is passed to sonic-cli in the environment, and the spawned
    child is registered with its private TMPDIR. Only those registered children
    are inspected; scanning all of /proc on every display tick was the most
    expensive part of a catalog run. The last size per temp directory is kept
    because sonic-cli removes it as soon as the track is converted. Work files
    follow TMPDIR, which can be on an external drive, so roots come from the
    registered child instead of being hard-coded to /tmp.
    """

    DIR_PREFIX = 'sonic-download-'
    SAMPLE_INTERVAL = 0.75

    def __init__(self, token):
        self.marker = f'SONIC_BATCH_TOKEN={token}'.encode()
        self.started = time.monotonic()
        self.start_wall = time.time()
        self.seen: dict[str, int] = {}
        self.owned: set[str] = set()
        self.created: dict[str, float] = {}
        self.roots: set[str] = set()
        self.registered: dict[int, str | None] = {}

    def register(self, pid, tmp_root=None, env=None):
        """Learn one child's TMPDIR once, verifying its batch token."""
        if env is None:
            try:
                env = Path(f'/proc/{pid}/environ').read_bytes()
            except OSError:
                env = b''
        if env and self.marker not in env:
            return False
        root = self.tmp_root(env) or tmp_root
        self.registered[int(pid)] = root
        if root:
            self.roots.add(root)
        return True

    @classmethod
    def folder_from_target(cls, target):
        """Return the sonic-download-* directory a descriptor points into."""
        marker = '/' + cls.DIR_PREFIX
        start = target.find(marker)
        if start < 0:
            return None
        end = start + len(marker)
        while end < len(target) and not target[end].isspace() and target[end] != '/':
            end += 1
        return target[:end]

    @staticmethod
    def tmp_root(env):
        for item in env.split(b'\0'):
            if item.startswith(b'TMPDIR='):
                return os.fsdecode(item[len(b'TMPDIR='):])
        return None

    def _remember(self, folder):
        if folder not in self.created:
            try:
                self.created[folder] = os.stat(folder).st_ctime
            except OSError:
                self.created[folder] = self.start_wall

    def _dirs(self):
        dirs = set()
        for pid, known_root in self.registered.items():
            if known_root:
                self.roots.add(known_root)
            fd_dir = f'/proc/{pid}/fd'
            try:
                names = os.listdir(fd_dir)
            except OSError:
                continue
            for fd in names:
                try:
                    target = os.readlink(f'{fd_dir}/{fd}')
                except OSError:
                    continue
                folder = self.folder_from_target(target)
                if folder:
                    dirs.add(folder)
                    self.owned.add(folder)
                    self._remember(folder)
        if not dirs:
            # During conversion there may be no open part descriptor; fall back
            # to the TMPDIRs the marked children actually use.
            for root in sorted(self.roots | {tempfile.gettempdir()}):
                try:
                    names = os.listdir(root)
                except OSError:
                    continue
                for name in names:
                    if not name.startswith(self.DIR_PREFIX):
                        continue
                    path = os.path.join(root, name)
                    try:
                        st = os.stat(path)
                    except OSError:
                        continue
                    if st.st_uid == os.getuid() and st.st_ctime >= self.start_wall - 2:
                        dirs.add(path)
                        self.owned.add(path)
                        self._remember(path)
        return dirs

    def sample(self):
        for folder in self._dirs():
            try:
                names = os.listdir(folder)
            except OSError:
                continue
            size = 0
            for name in names:
                if not name.startswith('audio.'):
                    continue
                try:
                    size = max(size, os.path.getsize(f'{folder}/{name}'))
                except OSError:
                    pass
            if size:
                self.seen[folder] = max(self.seen.get(folder, 0), size)
        return sum(self.seen.values())

    def file_sizes(self):
        """Per-track sizes in the order sonic-cli created their temp dirs."""
        return [self.seen[d] for d in sorted(self.created,
                                            key=self.created.__getitem__)
                if d in self.seen]

    def cleanup(self):
        """Remove only temp dirs this download still owns after an interrupt."""
        for folder in self.owned:
            try:
                shutil.rmtree(folder)
            except OSError:
                pass


def kill_proc_group(proc):
    """Stop a child tree gracefully before forcing it down."""
    try:
        os.killpg(proc.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError):
        try:
            proc.terminate()
        except OSError:
            pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(proc.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        if proc.poll() is None:
            try:
                proc.kill()
            except OSError:
                pass


# ---------------- detached session control ----------------

def detached_pid_path(slug_name, state_dir=None):
    """Pidfile for one artist's detached catalog worker."""
    base = Path(state_dir) if state_dir else ytm_config.STATE_DIR
    return base / f'{slug(slug_name)}.catalog.pid'


def write_detached_pidfile(slug_name, pid, run_id, state_dir=None, pgid=None):
    path = detached_pid_path(slug_name, state_dir)
    path.parent.mkdir(parents=True, exist_ok=True)
    record = {
        'pid': int(pid),
        'pgid': int(pgid or pid),
        'slug': slug(slug_name),
        'run_id': run_id,
    }
    tmp = path.with_name(path.name + '.tmp')
    tmp.write_text(json.dumps(record, indent=1) + '\n', encoding='utf-8')
    os.replace(tmp, path)
    return path


def read_detached_pidfile(path):
    try:
        record = json.loads(Path(path).read_text(encoding='utf-8'))
        int(record['pid'])
        int(record.get('pgid') or record['pid'])
        return record
    except (OSError, ValueError, TypeError, KeyError):
        return None


def remove_detached_pidfile(slug_name, run_id, state_dir=None):
    """Remove a pidfile only when it still describes this run."""
    path = detached_pid_path(slug_name, state_dir)
    record = read_detached_pidfile(path)
    if record and run_id and record.get('run_id') != run_id:
        return False
    try:
        path.unlink()
        return True
    except FileNotFoundError:
        return False


def set_process_title(slug_name):
    """Set a short Linux process title like `cat-strandz` (cosmetic only)."""
    if os.name != 'posix':
        return
    try:
        import ctypes
        libc = ctypes.CDLL(None, use_errno=True)
        title = f'cat-{slug(slug_name)}'[:15].encode('ascii', 'ignore')
        libc.prctl.argtypes = [ctypes.c_int, ctypes.c_void_p,
                               ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
        libc.prctl.restype = ctypes.c_int
        libc.prctl(15, ctypes.c_char_p(title), 0, 0, 0)  # PR_SET_NAME
    except Exception:
        # prctl is Linux-only and never essential to cancellation.
        pass


def register_detached_worker(args):
    """Called inside a detached child: claim its pidfile and clean it on exit."""
    slug_name = getattr(args, 'detached_run', None)
    if not slug_name:
        return
    set_process_title(slug_name)
    state_dir = (Path(args.state).parent if getattr(args, 'state', None)
                 else None)
    run_id = getattr(args, 'run_id', None) or uuid.uuid4().hex
    args.run_id = run_id
    write_detached_pidfile(slug_name, os.getpid(), run_id,
                           state_dir=state_dir, pgid=os.getpgrp())
    atexit.register(remove_detached_pidfile, slug_name, run_id, state_dir)


def spawn_detached_process(cmd, slug_name, logfile=None, state_dir=None):
    """Start a detached worker in its own session and record it for --cancel."""
    target = Path(logfile) if logfile else (
        ytm_config.LOG_DIR / f'{slug(slug_name)}.log')
    target.parent.mkdir(parents=True, exist_ok=True)
    run_id = uuid.uuid4().hex
    child_cmd = list(cmd) + ['--run-id', run_id,
                             '--detached-run', slug(slug_name)]
    with open(target, 'ab') as handle:
        proc = subprocess.Popen(child_cmd, stdout=handle,
                                stderr=subprocess.STDOUT,
                                start_new_session=True)
    # The child also claims this file at startup; the parent's copy makes a
    # cancel during interpreter startup safe.
    write_detached_pidfile(slug_name, proc.pid, run_id,
                           state_dir=state_dir, pgid=proc.pid)
    return proc, run_id


def _read_cmdline(pid, proc_root='/proc'):
    raw = (Path(proc_root) / str(pid) / 'cmdline').read_bytes()
    return [part.decode('utf-8', 'replace')
            for part in raw.split(b'\0') if part]


def _is_catalog_cmdline(args):
    script = str(Path(__file__).resolve())
    return any(arg == script or Path(arg).name == 'ytm-artist-batch.py'
               for arg in args)


def _cmdline_values(args):
    values = {}
    for index, arg in enumerate(args):
        if not arg.startswith('--'):
            continue
        name = arg[2:].split('=', 1)[0]
        if name not in ('artist', 'detached-run', 'run-id'):
            continue
        key = name.replace('-', '_')
        if '=' in arg:
            values[key] = arg.split('=', 1)[1]
        elif index + 1 < len(args):
            values[key] = args[index + 1]
    return values


def _cmdline_slug(values):
    return slug(values.get('detached_run') or values.get('artist') or '')


def _pidfile_candidate(record, target_slug, proc_root='/proc'):
    """Validate a pidfile against a live catalog worker before killing it."""
    if not record or slug(record.get('slug') or '') != target_slug:
        return None
    try:
        pid = int(record['pid'])
        if pid == os.getpid():
            return None
        os.kill(pid, 0)
        args = _read_cmdline(pid, proc_root)
    except (OSError, ValueError, TypeError, KeyError):
        return None
    if not args or not _is_catalog_cmdline(args):
        return None
    values = _cmdline_values(args)
    if record.get('run_id') and values.get('run_id') != record['run_id']:
        return None
    if _cmdline_slug(values) != target_slug:
        return None
    try:
        pgid, sid = os.getpgid(pid), os.getsid(pid)
    except OSError:
        return None
    # Detached workers are session leaders. Refusing anything else avoids
    # killing a foreground run or an unrelated recycled PID.
    if pgid != pid or sid != pid:
        return None
    return {'pid': pid, 'pgid': pgid, 'run_id': values.get('run_id'),
            'source': 'pidfile'}


def find_detached_sessions(slug_name, proc_root='/proc'):
    """Find live detached workers, including sessions started before pidfiles."""
    target = slug(slug_name)
    found = []
    try:
        pids = [int(entry.name) for entry in Path(proc_root).iterdir()
                if entry.name.isdigit()]
    except OSError:
        return found
    for pid in pids:
        if pid == os.getpid():
            continue
        try:
            args = _read_cmdline(pid, proc_root)
        except OSError:
            continue
        if not args or not _is_catalog_cmdline(args):
            continue
        # The launcher carries --detach; its worker deliberately does not.
        if '--detach' in args or '--cancel' in args:
            continue
        values = _cmdline_values(args)
        if _cmdline_slug(values) != target:
            continue
        try:
            pgid, sid = os.getpgid(pid), os.getsid(pid)
        except OSError:
            continue
        if pgid != pid or sid != pid:
            continue
        found.append({'pid': pid, 'pgid': pgid,
                      'run_id': values.get('run_id'), 'source': 'proc'})
    return found


def process_group_alive(pgid):
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def terminate_catalog_group(session, say, timeout=5.0):
    pid, pgid = session['pid'], session['pgid']
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        return True
    except PermissionError as exc:
        say(f'no permission to signal pid {pid}: {exc}', flush=True)
        return False
    if not process_group_alive(pgid):
        return True
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if not process_group_alive(pgid):
            return True
        time.sleep(0.05)
    say(f'session {pid} still running; sending SIGKILL', flush=True)
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        return True
    except PermissionError as exc:
        say(f'no permission to force pid {pid}: {exc}', flush=True)
        return False
    for _ in range(10):
        if not process_group_alive(pgid):
            return True
        time.sleep(0.05)
    return False


def cancel_catalog_session(name, state_dir=None, say=None, proc_root='/proc',
                           timeout=5.0):
    """Cancel detached catalog worker(s) for one artist."""
    emit = say or log
    target = slug(name)
    if not target:
        emit('--cancel needs an artist name', flush=True)
        return False

    pid_path = detached_pid_path(target, state_dir)
    record = read_detached_pidfile(pid_path)
    from_pidfile = _pidfile_candidate(record, target, proc_root)
    if record and not from_pidfile:
        # Stale or recycled pidfile: never signal it.
        try:
            pid_path.unlink()
        except OSError:
            pass

    sessions = []
    if from_pidfile:
        sessions.append(from_pidfile)
    for session in find_detached_sessions(target, proc_root=proc_root):
        if not any(item['pid'] == session['pid'] for item in sessions):
            sessions.append(session)

    if not sessions:
        emit(f'no detached catalog session found for {name!r}', flush=True)
        return False

    ok = True
    for session in sessions:
        emit(f"cancelling detached catalog session for '{target}' "
             f"(pid {session['pid']}, pgid {session['pgid']})", flush=True)
        if not terminate_catalog_group(session, emit, timeout=timeout):
            ok = False
    try:
        pid_path.unlink()
    except OSError:
        pass
    if ok:
        emit(f'cancelled {len(sessions)} session(s) for {target!r}', flush=True)
    else:
        emit(f'some sessions for {target!r} did not stop; check ps', flush=True)
    return ok


def download(args, extra, timeout, total_bytes=0, file_totals=None, label='',
             concurrency=None):
    if ABORT.is_set():
        raise KeyboardInterrupt
    token = uuid.uuid4().hex
    env = os.environ.copy()
    env.update(getattr(args, 'download_env', {}) or {})
    env['SONIC_BATCH_TOKEN'] = token
    monitor = TempByteMonitor(token)
    key = PROGRESS.task_begin(total_bytes, file_totals, key=token, label=label)
    # Catalog downloads pass --in-memory-settings, so sonic-cli never writes
    # settings.json. One config dir is safe for parallel workers now; cloning
    # and deleting a private copy per download was pure filesystem churn.
    config_dir = Path(args.cli_config_dir)
    config_dir.mkdir(parents=True, exist_ok=True)
    try:
        env.update(start_download_bandwidth(args))
        result = run(cli_opts(args, extra, config_dir=config_dir,
                              concurrency=concurrency), timeout=timeout,
                     env=env, monitor=monitor, progress_key=key)
        if ABORT.is_set():
            raise KeyboardInterrupt
        return result
    except BaseException:
        monitor.cleanup()
        raise
    finally:
        end_download_bandwidth(args)
        PROGRESS.task_end(key=key)


def run(cmd, timeout=60, env=None, monitor=None, progress_key=None):
    """Run a child in its own process group so Ctrl-C kills the whole tree."""
    if ABORT.is_set():
        raise KeyboardInterrupt
    child_env = os.environ.copy()
    if env:
        child_env.update(env)
    tmp_dir = getattr(RUN_CONTEXT, 'tmp_dir', None)
    process_tmp = None
    if tmp_dir:
        process_tmp = Path(tempfile.mkdtemp(
            prefix=f'run-{os.getpid()}-', dir=tmp_dir))
        child_env['TMPDIR'] = str(process_tmp)
        register_temp_dir(process_tmp)
    proc = None
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, text=True, env=child_env,
                                start_new_session=True)
        register_child(proc)
        if monitor is not None:
            # Read the child's environment once here instead of once per tick.
            monitor.register(proc.pid,
                             tmp_root=str(process_tmp) if process_tmp else None)
        if ABORT.is_set():
            raise KeyboardInterrupt
        deadline = time.monotonic() + timeout
        try:
            if monitor is None:
                out, err = proc.communicate(timeout=timeout)
            else:
                next_sample = time.monotonic()
                while proc.poll() is None:
                    if ABORT.is_set():
                        raise KeyboardInterrupt
                    now = time.monotonic()
                    if now >= deadline:
                        raise subprocess.TimeoutExpired(cmd, timeout)
                    if now >= next_sample:
                        total = monitor.sample()
                        PROGRESS.task_progress(total, monitor.file_sizes(),
                                               key=progress_key)
                        next_sample = time.monotonic() + monitor.SAMPLE_INTERVAL
                    try:
                        proc.wait(timeout=0.25)
                    except subprocess.TimeoutExpired:
                        pass
                out, err = proc.communicate()
        except BaseException:
            kill_proc_group(proc)
            try:
                proc.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                pass
            raise
        return subprocess.CompletedProcess(cmd, proc.returncode, out, err)
    finally:
        if proc is not None:
            unregister_child(proc)
        if process_tmp:
            unregister_temp_dir(process_tmp)
            shutil.rmtree(process_tmp, ignore_errors=True)


def clean_stale_tmp_runs(tmp_dir):
    """Remove per-process temp directories left by interrupted prior runs."""
    for path in tmp_dir.glob('run-*'):
        match = re.match(r'^run-(\d+)-', path.name)
        if not match or not path.is_dir():
            continue
        try:
            os.kill(int(match.group(1)), 0)
        except ProcessLookupError:
            shutil.rmtree(path)
        except PermissionError:
            continue


def cleanup_artist_temp_after_cancel(artist, root_arg=None, genre=None, say=None):
    """Reap stale run/work dirs for one artist after a cancellation request."""
    emit = say or log
    root = resolve_root(artist, root_arg, genre)
    removed = 0
    tmp_dir = root.resolve() / '.sonic-tmp'
    if tmp_dir.is_dir():
        before = {path for path in tmp_dir.glob('run-*') if path.is_dir()}
        clean_stale_tmp_runs(tmp_dir)
        removed += sum(1 for path in before if not path.exists())
        if removed:
            emit(f'removed {removed} stale temp dir(s) under {tmp_dir}', flush=True)

    # A detached worker that was killed before this cleanup existed could also
    # leave an empty .download-* folder in the library. Only remove them when
    # no live detached worker remains for this artist.
    if not find_detached_sessions(slug(artist)):
        work_dirs = []
        for base in (root, root / 'EP', root / 'SINGLES'):
            if not base.is_dir():
                continue
            work_dirs += [path for path in base.glob('.download-*')
                          if path.is_dir() and not path.is_symlink()]
        work_removed = 0
        for path in work_dirs:
            shutil.rmtree(path, ignore_errors=True)
            if not path.exists():
                work_removed += 1
        removed += work_removed
        if work_removed:
            emit(f'removed {work_removed} stale download dir(s) under {root}',
                 flush=True)
    return removed


def norm(s):
    s = (s or '').casefold()
    s = re.sub(r'\([^)]*\)', '', s)
    s = s.replace('&', ' and ').replace("'", '').replace('’', '')
    s = re.sub(r'\b(?:pt|part)\s*\d+\b', '', s)
    s = re.sub(r'[^a-z0-9]+', ' ', s)
    return ' '.join(s.split())


def file_tags(path):
    """Read title/date tags once per file, cached by size and mtime.

    Album scoring compares a candidate's track titles against every other
    album folder. Probing each file once per comparison is the largest local
    cost of a fresh run; a signature-keyed cache keeps that work to one
    ffprobe per file while still noticing downloads and retags.
    """
    try:
        st = path.stat()
    except OSError:
        return {}
    signature = (st.st_size, st.st_mtime_ns)
    key = str(path)
    cached = TAG_CACHE.get(key)
    if cached and cached[0] == signature:
        return cached[1]

    fields = ','.join(TAG_FIELDS)
    p = run(['ffprobe', '-v', 'error', '-show_entries',
             f'stream_tags={fields}:format_tags={fields}',
             '-of', 'json', str(path)])
    try:
        data = json.loads(p.stdout or '{}')
    except (ValueError, TypeError):
        data = {}

    tags = {}
    # Keep ffprobe's original precedence: stream tags before format tags.
    for stream in data.get('streams') or []:
        for name, value in (stream.get('tags') or {}).items():
            key_name = str(name).casefold()
            if key_name in TAG_FIELDS and value and key_name not in tags:
                tags[key_name] = str(value)
    fmt = data.get('format') or {}
    for name, value in (fmt.get('tags') or {}).items():
        key_name = str(name).casefold()
        if key_name in TAG_FIELDS and value and key_name not in tags:
            tags[key_name] = str(value)

    TAG_CACHE[key] = (signature, tags)
    return tags


def tag(path, name):
    return file_tags(path).get(str(name).casefold(), '')


def tag_flags(updates):
    """sonic-cli --tag flags for a set of metadata values.

    The equals form keeps a value starting with '-' from being parsed as an
    option by clap, even if a future metadata key ever begins with one.
    """
    flags = []
    for key, value in updates.items():
        if value is not None:
            flags.append(f'--tag={key}={value}')
    return flags


def performer_hint_flags(names):
    """Hidden sonic-cli flags for performers that must never be filtered out."""
    flags = []
    seen = set()
    for name in names:
        key = norm(name)
        if key and key not in seen:
            seen.add(key)
            flags.append(f'--performer-hint={name}')
    return flags


def exclude_track_flags(video_ids):
    """Hidden sonic-cli flags for exact video ids owned by an earlier album.

    The equals form is required because a YouTube id may start with '-'.
    """
    flags = []
    seen = set()
    for video_id in video_ids:
        if video_id and video_id not in seen:
            seen.add(video_id)
            flags.append(f'--exclude-track={video_id}')
    return flags


def index_signature(index):
    """Stable representation used to avoid rewriting an unchanged index."""
    return json.dumps(index or {}, sort_keys=True, ensure_ascii=False)


def cached_channel_id(index_path, index=None):
    """Reuse a pinned YTM channel when refreshing an existing index.

    A loaded index is passed in when available so a refresh does not read the
    cache file twice. A stale index still works because the caller reads the
    raw snapshot separately.
    """
    if index is None:
        try:
            index = json.loads(index_path.read_text(encoding='utf-8'))
        except (OSError, ValueError):
            return None
    return ytm_index.channel_browse_id((index or {}).get('channelBrowseId') or '')


def resolve_channel_argument(value, ytdlp=None, note=None):
    """Turn --ytm-channel into a canonical UC channel id.

    Bare ids and URLs containing a UC id need no network. YouTube handle URLs
    (``https://music.youtube.com/@Artist`` or ``@Artist``) are resolved once
    with yt-dlp; music.youtube.com handles are rewritten to www.youtube.com
    because the music frontend does not expose channel_id in yt-dlp output.
    """
    if not value:
        return None
    text = str(value).strip()
    direct = ytm_index.channel_browse_id(text)
    if direct:
        return direct

    handle = re.search(r'(?:music\.)?youtube\.com/@([A-Za-z0-9._-]+)',
                       text, re.I)
    if handle:
        target = f'https://www.youtube.com/@{handle.group(1)}'
    elif re.fullmatch(r'@[A-Za-z0-9._-]+', text):
        target = f'https://www.youtube.com/{text}'
    elif re.search(r'(?:music\.)?youtube\.com/|youtu\.be/', text, re.I):
        target = text
    else:
        return None

    cmd = [ytdlp or DEFAULT_YTDLP, '--no-warnings', '--skip-download',
           '--playlist-end', '1', '--print', '%(channel_id)s', target]
    try:
        result = run(cmd, timeout=60)
    except Exception as exc:
        if note:
            note(f'--ytm-channel could not resolve {text!r}: {exc}', flush=True)
        return None
    if result.returncode != 0:
        if note:
            detail = (result.stderr or '').strip().splitlines()
            suffix = f' ({detail[-1]})' if detail else ''
            note(f'--ytm-channel resolution failed for {text!r}{suffix}',
                 flush=True)
        return None
    for line in result.stdout.splitlines():
        found = ytm_index.channel_browse_id(line.strip())
        if found:
            if note:
                note(f'--ytm-channel resolved {text!r} to {found}', flush=True)
            return found
    return None


def track_tag_flags(key, updates):
    """sonic-cli --track-tag flags keyed by video id (or track title).

    Video ids can begin with '-' (e.g. -BOc6-sh_IU). A separate argv value
    would be read as an unknown short option, so always use --track-tag=...
    """
    flags = []
    for name, value in updates.items():
        if value is not None:
            flags.append(f'--track-tag={key}={name}={value}')
    return flags


def pending_tag_updates(path, updates):
    """Only the values that differ from the file's cached ffprobe snapshot."""
    current = file_tags(path)
    pending = {}
    for key, value in updates.items():
        if value is None:
            continue
        if str(current.get(str(key).casefold(), '')) != str(value):
            pending[key] = value
    return pending


def retag(path, updates):
    """Rewrite tags with sonic-core, which also preserves Opus cover art."""
    if not updates:
        return
    cmd = [RETAG_CLI]
    if RETAG_CONFIG_DIR:
        cmd += ['--config-dir', str(RETAG_CONFIG_DIR)]
    cmd += ['--tag-file', str(path)] + tag_flags(updates)
    p = run(cmd, timeout=180)
    TAG_CACHE.pop(str(path), None)
    if p.returncode != 0:
        log(f'    retag failed: {path.name}: '
              f'{(p.stderr or p.stdout).strip()[-200:]}', flush=True)


MB_TYPES = {'album': 'Album', 'ep': 'EP', 'single': 'Single'}


def explicit_updates(explicit):
    """Boolean explicit flag written as tags, not an RATING=E string."""
    if explicit is None:
        return {}
    return {'EXPLICIT': '1' if explicit else '0',
            'ITUNESADVISORY': '1' if explicit else '2'}


def release_updates(rtype, explicit):
    """Tags written for one release (download time or --retag)."""
    updates = {}
    if rtype in MB_TYPES:
        updates['RELEASE_TYPE'] = rtype.upper()
        updates['MUSICBRAINZ_ALBUMTYPE'] = MB_TYPES[rtype]
    updates.update(explicit_updates(explicit))
    return updates


def album_updates(rtype, year, explicit):
    """Album tags. The per-track artist is left to sonic-core/YTM."""
    updates = {'RELEASE_TYPE': rtype.upper(),
               'MUSICBRAINZ_ALBUMTYPE': MB_TYPES.get(rtype, 'Album'),
               'date': year}
    updates.update(explicit_updates(explicit))
    return updates


def single_updates(title, explicit):
    """Single tags. `artist` is deliberately absent so YTM credits survive."""
    updates = release_updates('single', explicit)
    updates['title'] = title
    return updates


def video_id_from(url):
    m = re.search(r'[?&]v=([A-Za-z0-9_-]+)', url or '')
    return m.group(1) if m else None


def explicit_for_video(index, vid):
    rec = (index or {}).get('videos', {}).get(vid) or {}
    if 'explicit' in rec:
        return bool(rec['explicit'])
    return None


def member_explicit(members, f):
    """Find the E flag for a downloaded album track by its title."""
    want = norm(tag(f, 'title') or re.sub(r'^\d+\s*[-–]\s*', '', f.stem))
    for row in members or []:
        if norm(row.get('title')) == want:
            return row.get('explicit')
    return None


def folder_titles(folder):
    out = []
    for f in audio_files(folder):
        t = tag(f, 'title')
        if t:
            out.append(t)
    return out


# ---------------- album discovery / validation ----------------

def album_browses(ytdlp, artist, title, year):
    urls = []
    for query in (f'{artist} {title} {year}', f'{artist} {title}'):
        p = run([ytdlp, '--no-warnings', '--flat-playlist', '--print',
                 '%(title)s\t%(url)s',
                 f'https://music.youtube.com/search?q={quote(query)}'])
        for line in p.stdout.splitlines():
            if '\t' not in line:
                continue
            url = line.split('\t', 1)[1]
            if '/browse/MPREb' in url and url not in urls:
                urls.append(url)
    return urls[:8]


def playlist_id(ytdlp, browse_url):
    p = run([ytdlp, '--no-warnings', '--flat-playlist', '--print',
             '%(playlist_id)s\t%(playlist_title)s', browse_url])
    line = next((l for l in p.stdout.splitlines() if '\t' in l), '')
    if not line:
        return None
    pid, ptitle = line.split('\t', 1)
    if not pid.startswith('OLAK5uy') or any(b in ptitle.casefold() for b in BAD):
        return None
    return pid, ptitle


def playlist_rows(ytdlp, pid):
    """Flat playlist rows as (duration, title, url) without extra calls."""
    p = run([ytdlp, '--no-warnings', '--flat-playlist', '--print',
             '%(duration)s\t%(title)s\t%(url)s',
             f'https://music.youtube.com/playlist?list={pid}'])
    rows = []
    for line in p.stdout.splitlines():
        parts = line.split('\t', 2)
        if len(parts) < 3:
            continue
        dur, title, url = parts
        try:
            dur = float(dur)
        except ValueError:
            dur = 0.0
        rows.append((dur, title, url))
    return rows


def playlist_sizes(ytdlp, pid):
    """Estimated raw stream bytes per track, in playlist order."""
    return [int(r[0] * SOURCE_BPS) for r in playlist_rows(ytdlp, pid)]


def playlist_size(ytdlp, pid):
    """Estimated raw stream bytes for a whole playlist."""
    return sum(playlist_sizes(ytdlp, pid))


def validate(ytdlp, pid, ptitle):
    rows = playlist_rows(ytdlp, pid)
    if not rows:
        return None
    tracks = [r[1] for r in rows]
    first_url = rows[0][2]
    meta = {}
    if first_url:
        m = run([ytdlp, '--no-warnings', '--extractor-args',
                 'youtube:player_client=web_music', '--print',
                 '%(title)s\t%(album)s\t%(date)s\t%(artist)s', first_url])
        parts = (m.stdout.splitlines()[0] if m.stdout else '').split('\t')
        parts += [''] * (4 - len(parts))
        meta = dict(zip(('title', 'album', 'date', 'artist'), parts[:4]))
    sizes = [int(r[0] * SOURCE_BPS) for r in rows]
    return {'playlist': ptitle, 'tracks': len(tracks), 'titles': tracks,
            'bytes': sum(sizes), 'file_bytes': sizes, **meta}


def _release_title(text, artist=''):
    """Normalized release title without YTM's 'Album -' prefix or artist name."""
    s = norm(text)
    s = re.sub(r'^(?:album|ep|single)\s+', '', s)
    if artist:
        who = re.sub(r'\s+', r'\\s+', norm(artist))
        s = re.sub(r'\b' + who + r'\b', ' ', s)
    return ' '.join(s.split())


def _compilation_penalty(expected, *texts):
    """Penalize compilations unless the curated title is one itself."""
    want = norm(expected)
    for phrase in BAD:
        if phrase in want:
            continue
        if any(phrase in norm(t) for t in texts if t):
            return -40
    return 0


def score(expected, year, artist, info):
    want = norm(expected)
    s = 0
    exact = {_release_title(expected), _release_title(expected, artist)}
    got = {_release_title(info.get('playlist'), artist),
           _release_title(info.get('album'), artist)}
    if exact & got:
        s += 24
    elif any(w and w in g for w in exact for g in got):
        s += 10
    years = re.findall(r'(?:19|20)\d{2}', str(info.get('date') or ''))
    if years:
        if str(year) in years:
            s += 10
        else:
            s -= 12
    credited = norm(info.get('artist'))
    if credited:
        who = norm(artist)
        if who in credited or credited in who:
            s += 6
        else:
            s -= 25
    s += _compilation_penalty(expected, info.get('playlist'), info.get('album'))
    tracks = int(info.get('tracks') or 0)
    if tracks > 32:
        s -= 25
    elif tracks > 24:
        s -= 8
    return s


def album_key(s, artist=''):
    """Normalized album key: strip year prefix and any artist name in the name."""
    s = re.sub(r'^\s*(?:19|20)\d{2}\s*[-–_:]\s*', '', s or '')
    key = norm(s)
    if artist:
        key = re.sub(r'\b' + re.escape(norm(artist)).replace(r'\ ', r'\s+') + r'\b',
                     ' ', key)
    return ' '.join(key.split())


def find_existing_album(root, title, year, artist=''):
    want = album_key(title, artist)
    for base in (root, root / 'EP'):
        if not base.is_dir():
            continue
        for folder in base.iterdir():
            if not is_library_folder(folder):
                continue
            if not audio_files(folder):
                continue
            if album_key(folder.name, artist) == want:
                return folder
    return None


def fix_year(root, folder, year, title=None):
    """Rename folder and retag files to the discography year/title.

    Renaming happens in the folder's own parent so EPs stay under ARTIST/EP.
    """
    year = str(year)
    name = title or re.sub(r'^\s*(?:19|20)\d{2}\s*[-–_:]\s*', '', folder.name)
    name = re.sub(r'[\\/:*?"<>|]', '_', name).strip()
    target = folder.parent / f'{year} - {name}'
    if folder != target:
        if target.exists() and target != folder:
            log(f'    year-fix skipped (target exists): {target.name}', flush=True)
        else:
            folder.rename(target)
            log(f'    year-fix: {folder.name} -> {target.name}', flush=True)
            folder = target
    for f in audio_files(folder):
        if tag(f, 'date') != year:
            retag(f, {'date': year})
    return folder


# ---------------- single discovery / validation ----------------

def resolve_single(ytdlp, artist, title):
    p = run([ytdlp, '--no-warnings', '--flat-playlist', '--print',
             '%(title)s\t%(url)s',
             f'https://music.youtube.com/search?q={quote(artist + " " + title)}'])
    hits = []
    for line in p.stdout.splitlines():
        if '\t' not in line or 'music.youtube.com/watch?v=' not in line:
            continue
        htitle, url = line.split('\t', 1)
        h = norm(htitle)
        w = norm(title)
        rank = 0 if h == w else (1 if h.startswith(w + ' ') or w in h else 99)
        if rank < 99:
            hits.append((rank, htitle, url))
    hits.sort(key=lambda x: x[0])
    for rank, htitle, url in hits[:3]:
        m = run([ytdlp, '--no-warnings', '--extractor-args',
                 'youtube:player_client=web_music', '--print',
                 '%(title)s\t%(artist)s\t%(album)s\t%(channel)s', url])
        line = m.stdout.splitlines()[0] if m.stdout else ''
        parts = line.split('\t')
        parts += [''] * (4 - len(parts))
        meta = dict(zip(('title', 'artist', 'album', 'channel'), parts[:4]))
        if artist.casefold() in (meta['artist'] + ' ' + meta['channel']).casefold():
            return url, meta
    # Never fall back to an unverified hit: that is how a cover by another
    # artist once won for "Heading South". The artist-page index handles most
    # titles up front; genuine misses should be reported, not mis-downloaded.
    return (None, {})


def index_single(index, artist, title):
    """Resolve a single from the cached artist-page index (no network)."""
    if not index:
        return None
    key = ytm_index.normkey(title, artist)
    hit = index.get('tracks', {}).get(key)
    if not hit or not hit.get('videoId'):
        return None
    vid = hit['videoId']
    video = (index.get('videos') or {}).get(vid) or {}
    return (f'https://music.youtube.com/watch?v={vid}',
            {'title': hit.get('title') or title,
             'artist': hit.get('artist') or artist,
             'playlist': hit.get('playlistId') or '',
             'date': hit.get('year') or '',
             'video_id': vid,
             'explicit': video.get('explicit'),
             'type': video.get('type') or 'single',
             'release': video.get('release') or ''})


def album_index_key(title, artist):
    plain = re.sub(r'^\s*(?:19|20)\d{2}\s*[-–_:]\s*', '', title or '')
    return ytm_index.normkey(plain, artist)


def index_album(index, ytdlp, artist, title, year):
    """Resolve an album from the index into the same shape validate() returns.

    The artist page already contains the exact OLAK5uy_ playlist ID, a
    representative video ID and the year for every release. sonic-cli enriches
    the ID with full metadata during download, so no extra lookup is needed.
    """
    if not index:
        return None
    key = album_index_key(title, artist)
    entry = index.get('albums', {}).get(key)
    if not entry or not entry.get('playlistId'):
        return None
    info = {'playlist_id': entry['playlistId'],
            'playlist': entry.get('title') or title,
            'album': entry.get('title') or title,
            'date': entry.get('year') or '',
            'artist': index.get('artistName') or artist,
            'video_id': entry.get('videoId'),
            'type': entry.get('type') or 'album',
            'explicit': entry.get('explicit'),
            'release_key': key,
            'tracks': 0, 'titles': []}
    return info


DUB_RE = re.compile(
    r'\b(?:dub|dubs|dubwise|dubplate|version|versions|'
    r'riddim|riddims|instrumental|instrumentals|beats?)\b', re.I)

REISSUE_RE = re.compile(
    r'\b\d+\s*(?:st|nd|rd|th)?\s*anniversary\b'
    r'|\b(?:anniversary|remaster(?:ed)?|reissue(?:d)?|'
    r're-record(?:ed|ing)?|re-recording)\b', re.I)


def is_dub_release(title):
    """Reggae dub/riddim releases are instrumental versions, not albums."""
    return bool(DUB_RE.search(title or ''))


def is_rerelease(title):
    """Anniversary, remastered, reissued and re-recorded editions are repeats.

    Deluxe editions are deliberately kept: YTM often has no plain copy, and
    the deluxe still contains the original album.
    """
    return bool(REISSUE_RE.search(title or ''))


def catalog_from_index(index):
    """Build the downloadable catalog from the artist's indexed releases."""
    albums, singles = [], []
    for release in (index or {}).get('albums', {}).values():
        title = (release.get('title') or '').strip()
        if not title:
            continue
        year = release.get('year') or ''
        if release.get('type') == 'single':
            singles.append({'title': title, 'year': year})
        else:
            albums.append((year, title, ''))
    return albums, singles


def catalog_from_wiki(index, artist, wiki_albums, wiki_singles, log=None):
    """Use Wikipedia as a reference list, with YTM as the source of truth.

    A Wikipedia title is only downloaded when the same release exists in the
    artist's YTM index; wiki-only titles are dropped without triggering a
    search. Dub/riddim/version releases are instrumental beats, and anniversary
    or re-recorded editions are repeats, so both are skipped. Deluxe editions
    are kept because they are often the only indexed copy of an album. If Wikipedia
    has no list for a category, that category falls back to the remaining YTM
    releases.
    """
    def say(message):
        if log:
            log(message, flush=True)

    def keep(title):
        return not is_dub_release(title) and not is_rerelease(title)

    full_albums, full_singles = catalog_from_index(index)
    # The index entry may be the re-release even when Wikipedia lists the
    # original title, so filter the YTM side before matching as well.
    indexed_albums = {key: release
                      for key, release in (index or {}).get('albums', {}).items()
                      if keep(release.get('title') or key)}
    indexed_tracks = (index or {}).get('tracks', {})

    if wiki_albums:
        candidates = [entry for entry in wiki_albums if keep(entry[1])]
        albums = [entry for entry in candidates
                  if album_index_key(entry[2] or entry[1], artist) in indexed_albums]
        say(f'Wikipedia release reference: kept {len(albums)}/{len(wiki_albums)} '
            f'with exact YTM matches; skipped '
            f'{len(wiki_albums) - len(candidates)} dub/re-release and '
            f'{len(candidates) - len(albums)} wiki-only title(s)')
        if not albums:
            albums = [entry for entry in full_albums if keep(entry[1])]
            say('Wikipedia had no exact YTM album match; using the YTM index '
                f'list instead ({len(albums)} album(s))')
    else:
        albums = [entry for entry in full_albums if keep(entry[1])]
        say(f'Wikipedia has no album list; using {len(albums)} of '
            f'{len(full_albums)} YTM album(s) after skipping '
            f'{len(full_albums) - len(albums)} dub/re-release(s)')

    if wiki_singles:
        candidates = [entry for entry in wiki_singles
                      if keep(entry.get('title', ''))]
        singles = [entry for entry in candidates
                   if ytm_index.normkey(entry.get('title', ''), artist)
                   in indexed_tracks]
        say(f'Wikipedia single reference: kept {len(singles)}/{len(wiki_singles)} '
            f'with exact YTM matches; skipped '
            f'{len(wiki_singles) - len(candidates)} dub/re-release and '
            f'{len(candidates) - len(singles)} wiki-only title(s)')
        if not singles:
            singles = [entry for entry in full_singles if keep(entry.get('title', ''))]
            say('Wikipedia had no exact YTM single match; using the YTM index '
                f'list instead ({len(singles)} single(s))')
    else:
        singles = [entry for entry in full_singles
                   if keep(entry.get('title', ''))]
        say(f'Wikipedia has no single list; using {len(singles)} of '
            f'{len(full_singles)} YTM single(s) after skipping '
            f'{len(full_singles) - len(singles)} dub/re-release(s)')
    return albums, singles


def wiki_reference_lists(artist, state_dir, log=None):
    """Load the cached Wikipedia lists, discovering them on first use."""
    a_path, s_path = auto_inputs(artist, state_dir, log)
    albums = load_albums(a_path)
    try:
        singles = json.loads(s_path.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        singles = []
    if log:
        log(f'Wikipedia reference loaded: {len(albums)} release(s), '
            f'{len(singles)} single(s)', flush=True)
    return albums, singles


def build_membership(index, artist, album_entries, log=None):
    """Map exact video IDs to an album/EP that is actually in this run.

    A single is an album/EP track only when that album/EP is selected here.
    Releases that are merely present in the index (including standalone
    singles that own themselves) must not hide a single from the queue. Member
    lists are cached in the index, so this needs no extra network lookup on a
    warm run.
    """
    video_release = {}
    if not index:
        return video_release
    # Rebuild the title map as well: an older index may still carry entries for
    # self-owned singles that would otherwise keep suppressing them.
    index['memberTitles'] = {}
    title_release = index['memberTitles']
    keys = set()
    for year, title, pinned in album_entries:
        plain = re.sub(r'^\s*(?:19|20)\d{2}\s*[-–_:]\s*', '', title or '')
        key = ytm_index.normkey(plain, artist)
        if key in index.get('albums', {}):
            keys.add(key)
    ordered_keys = sorted(keys)
    ytm_index.fetch_release_members(index, ordered_keys, note=log)
    for key in ordered_keys:
        rel = index['albums'][key]
        if (rel.get('type') or 'album') == 'single':
            continue
        rows = (index.get('members') or {}).get(key) or []
        rec = {'title': rel.get('title') or '',
               'type': rel.get('type') or 'album',
               'year': rel.get('year') or '',
               'playlistId': rel.get('playlistId') or '',
               'key': key}
        for row in rows:
            vid = row.get('videoId')
            if not vid:
                continue
            previous = video_release.get(vid)
            if previous is None or (previous['type'] == 'ep' and rec['type'] == 'album'):
                video_release[vid] = {**rec, 'explicit': row.get('explicit')}
            video = index.setdefault('videos', {}).setdefault(vid, {'videoId': vid})
            if 'explicit' not in video:
                video['explicit'] = row.get('explicit')
            tkey = ytm_index.normkey(row.get('title'))
            if tkey:
                earlier = title_release.get(tkey)
                if earlier is None or (earlier['type'] == 'ep' and rec['type'] == 'album'):
                    title_release[tkey] = {**rec, 'explicit': row.get('explicit')}
    return video_release


def dedupe_singles_for_albums(index, artist, albums, singles, disk_titles=None,
                              video_release=None, log=None, song_titles=False):
    """Drop singles already in a selected/downloaded album/EP before queueing.

    The download path still checks membership as a second line of defence, but
    the YTM index already has the exact member video IDs and titles, so the
    duplicate never needs to reach a worker. A library scan is included too:
    a track may already exist inside an album folder without being in the
    cached member list.
    """
    if not singles or song_titles:
        return list(singles)
    if video_release is None:
        video_release = build_membership(index, artist, albums, log)
    member_titles = (index or {}).get('memberTitles', {})
    indexed_tracks = (index or {}).get('tracks', {})
    indexed_albums = (index or {}).get('albums', {})
    disk_titles = disk_titles or set()

    kept, removed = [], []
    for single in singles:
        title = single.get('title') or ''
        key = ytm_index.normkey(title, artist)
        release = indexed_tracks.get(key) or indexed_albums.get(key) or {}
        vid = release.get('videoId')
        rel = video_release.get(vid) if vid else None
        reason = None
        if rel:
            reason = f'in {rel.get("type", "album")} {rel.get("title", "")}'.strip()
        else:
            by_title = member_titles.get(ytm_index.normkey(title))
            if by_title:
                reason = (f'in {by_title.get("type", "album")} '
                          f'{by_title.get("title", "")}'.strip())
            elif norm(title) in disk_titles:
                reason = 'already in an album folder'
        if reason:
            removed.append((title, reason))
        else:
            kept.append(single)

    if removed:
        def say(message):
            if log:
                log(message, flush=True)
        say(f'Pre-download duplicate filter: removed {len(removed)} '
            'single(s) already in album(s)')
        for title, reason in removed:
            say(f'  SKIP duplicate single: {title} ({reason})')
    return kept


def album_duplicate_exclusions(index, album_keys, disk_album_titles):
    """Return track IDs to exclude from selected album/EP downloads.

    Two sources are combined: flat YTM ownership (only when the owner is in
    this run or already on disk) and a title scan of library album folders.
    The scan catches tracks a cached member list does not know about, such as
    a song added to a playlist after the index was built.
    """
    owners = (index or {}).get('trackOwners') or {}
    excluded_members = (index or {}).get('excludedMembers') or {}
    indexed_members = (index or {}).get('members') or {}
    exclusions = {}

    def exclude_track(release_key, video_id):
        if video_id and video_id not in exclusions.setdefault(release_key, []):
            exclusions[release_key].append(video_id)

    for release_key, rows in excluded_members.items():
        for row in rows:
            owner = owners.get(row.get('ownerKey')) or {}
            owner_key = owner.get('key')
            on_disk = norm(owner.get('trackTitle') or row.get('title')) in disk_album_titles
            if owner_key:
                if owner_key not in album_keys and not on_disk:
                    continue
            elif not on_disk:
                continue
            exclude_track(release_key, row.get('videoId'))
    flat_count = sum(len(ids) for ids in exclusions.values())

    disk_count = 0
    for release_key in sorted(album_keys):
        for row in indexed_members.get(release_key) or []:
            title_key = norm(row.get('title'))
            if title_key and title_key in disk_album_titles:
                before = len(exclusions.get(release_key, []))
                exclude_track(release_key, row.get('videoId'))
                disk_count += len(exclusions.get(release_key, [])) - before
    return exclusions, flat_count, disk_count


def interactive_catalog_filter(albums, singles, artist=''):
    """Let the user tick releases and singles, then pick the run mode."""
    choices = []
    for index, (year, title, _pinned) in enumerate(albums):
        label = f'ALBUM {year} {title}'.strip()
        choices.append(Choice(label, ('album', index), group='album'))
    for index, single in enumerate(singles):
        label = f'SINGLE {single.get("year", "")} {single["title"]}'.strip()
        choices.append(Choice(label, ('single', index), group='single'))
    picked = select_items(
        choices, title=f'Select what to download for {artist}')
    mode = choose_run_mode()
    kept_albums = [albums[c.value[1]] for c in picked if c.value[0] == 'album']
    kept_singles = [singles[c.value[1]] for c in picked if c.value[0] == 'single']
    return kept_albums, kept_singles, mode


def spawn_detached_selection(args, name, albums, singles, disk_titles=None):
    """Hand the confirmed selection to a detached child of this script."""
    skip = ('--interactive', '--select', '--detach', '--status-only', '--retag')
    cmd = []
    argv = sys.argv[1:]
    index = 0
    while index < len(argv):
        value = argv[index]
        if value in skip or any(value.startswith(flag + '=') for flag in skip):
            index += 1
            continue
        cmd.append(value)
        index += 1
    cmd += ['--selected-albums', json.dumps(albums),
            '--selected-singles', json.dumps(singles)]
    if disk_titles:
        cmd += ['--disk-album-titles', json.dumps(sorted(disk_titles))]
    state_dir = Path(args.state).parent if getattr(args, 'state', None) else None
    _proc, _run_id = spawn_detached_process(
        [sys.executable, str(Path(__file__).resolve())] + cmd,
        name, state_dir=state_dir)
    logfile = ytm_config.LOG_DIR / f'{name}.log'
    log(f'started in background with {len(albums)} album(s) and '
        f'{len(singles)} single(s); log: {logfile}', flush=True)


def explicit_variant(index, title, explicit):
    """Add (Explicit)/(Clean) only when both versions of a title exist."""
    if explicit is None:
        return ''
    base = ytm_index.normkey(title)
    flags = set()
    for rec in (index or {}).get('videos', {}).values():
        if ytm_index.normkey(rec.get('title')) == base and 'explicit' in rec:
            flags.add(bool(rec['explicit']))
    if True in flags and False in flags:
        return '(Explicit)' if explicit else '(Clean)'
    return ''


def retag_library(args, state, root, state_path, index, log):
    """Backfill release-type and explicit tags without re-downloading."""
    dest = root / 'SINGLES'
    jobs = []
    unchanged = 0
    for title, rec in state['albums'].items():
        folder = (root / rec['folder'] if rec.get('folder') else
                  find_existing_album(root, title, rec.get('year', ''), args.artist))
        if not folder or not folder.is_dir():
            continue
        rtype = rec.get('type') or ('ep' if folder.parent.name == 'EP' else 'album')
        key = rec.get('release_key') or ytm_index.normkey(title, args.artist)
        members = (index or {}).get('members', {}).get(key, [])
        for f in audio_files(folder):
            wanted = release_updates(rtype, member_explicit(members, f))
            changes = pending_tag_updates(f, wanted)
            if changes:
                jobs.append((f, changes))
            else:
                unchanged += 1
        rec['type'] = rtype
    for title, rec in state['singles'].items():
        f = (dest / rec['file'] if rec.get('file') else
             find_existing_single(root, title))
        if not f or not f.is_file():
            continue
        vid = (rec.get('meta') or {}).get('video_id') or video_id_from(rec.get('url'))
        wanted = release_updates('single', explicit_for_video(index, vid))
        changes = pending_tag_updates(f, wanted)
        if changes:
            jobs.append((f, changes))
        else:
            unchanged += 1

    def apply(job):
        path, changes = job
        retag(path, changes)

    if len(jobs) > 1:
        with ThreadPoolExecutor(max_workers=min(RETAG_WORKERS, len(jobs))) as pool:
            futures = [pool.submit(apply, job) for job in jobs]
            for future in futures:
                try:
                    future.result()
                except (KeyboardInterrupt, InterruptedError):
                    raise
                except BaseException as exc:
                    log(f'retag worker error: {exc}', flush=True)
    else:
        for job in jobs:
            apply(job)

    save_state(state_path, state)
    detail = f'; {unchanged} unchanged skipped' if unchanged else ''
    log(f'retagged {len(jobs)} file(s){detail}', flush=True)


def find_existing_single(root, title):
    dest = root / 'SINGLES'
    if not dest.exists():
        return None
    want = norm(title)
    for f in audio_files(dest):
        if want in norm(f.stem):
            return f
    return None


def sanitize(s):
    return re.sub(r'[\\/:*?"<>|]', '_', s).strip()


# ---------------- state ----------------

def load_state(path):
    if path.exists():
        with open(path, encoding='utf-8') as fh:
            return json.load(fh)
    return {'albums': {}, 'singles': {}}


def save_state(path, state):
    # Several single threads can finish at once; serialize whole snapshots and
    # use a private temp name so two writers can never clobber each other.
    with STATE_LOCK:
        tmp = path.with_name(f'.{path.name}.{uuid.uuid4().hex}.tmp')
        tmp.write_text(json.dumps(state, indent=2), encoding='utf-8')
        os.replace(tmp, path)


# ---------------- main ----------------

def slug(s):
    return re.sub(r'[^a-z0-9]+', '-', s.casefold()).strip('-')


def normalize_album_args(argv):
    """Turn --albums[[1,2], [3]] into --albums-per-artist '[[1,2],[3]]'.

    The shell may split it at unquoted spaces, so rejoin until there is an
    extra closing ']' for the option itself.
    """
    out, i = [], 0
    while i < len(argv):
        arg = argv[i]
        if arg == '--albums[]':
            out.append(arg)
            i += 1
            continue
        if not arg.startswith('--albums['):
            out.append(arg)
            i += 1
            continue
        value = arg[len('--albums['):]
        while i + 1 < len(argv) and value.count(']') <= value.count('['):
            i += 1
            value += ' ' + argv[i]
        if value.count(']') <= value.count('['):
            raise SystemExit('--albums[...]: missing closing ]')
        if not value.endswith(']'):
            raise SystemExit('--albums[...]: expected closing ]')
        content = value[:-1].strip()
        out += ['--albums-per-artist', '[' + content + ']']
        i += 1
    return out


def split_artist_value(value):
    value = value.strip()
    candidate = value if value.startswith('[') and value.endswith(']') else f'[{value}]'
    try:
        parsed = ast.literal_eval(candidate)
        if isinstance(parsed, (list, tuple)):
            return [str(x).strip() for x in parsed if str(x).strip()]
    except (ValueError, SyntaxError):
        pass
    out = []
    for part in value.split(','):
        item = part.strip()
        if len(item) >= 2 and item[0] in ('"', "'") and item[-1] == item[0]:
            item = item[1:-1].strip()
        if item:
            out.append(item)
    return out


def normalize_artist_args(argv):
    """Turn --artists['a', 'b'] (or --artist[...]) into --artists '["a","b"]'.

    The shell may split the bracket form at its unquoted space, so also rejoin
    tokens until the closing ']' is found.
    """
    out, i = [], 0
    while i < len(argv):
        arg = argv[i]
        if arg in ('--artist[]', '--artists[]'):
            out.append(arg)
            i += 1
            continue
        m = re.match(r'^(--(?:artist|artists)(?:\[\])?)([=\[])(.*)$', arg)
        if not m:
            out.append(arg)
            i += 1
            continue
        _, delim, value = m.groups()
        if delim == '[':
            if not value.endswith(']'):
                parts = [value]
                i += 1
                while i < len(argv) and not argv[i].endswith(']'):
                    parts.append(argv[i])
                    i += 1
                if i >= len(argv):
                    raise SystemExit('--artists: missing closing ]')
                parts.append(argv[i])
                value = ' '.join(parts)
            value = value[:-1].strip()
        out += ['--artists', json.dumps(split_artist_value(value))]
        i += 1
    return out


def library_root():
    # The CLI rewrites its own downloadFolder on every run, so don't use it here.
    # $SONIC_MUSIC_ROOT, else the platform's default Music folder.
    return ytm_config.music_root()


def resolve_root(artist, root_arg, genre, state=None):
    if root_arg:
        return Path(root_arg)
    if state:
        saved = state.get('root')
        if saved:
            candidate = Path(saved)
            if candidate.is_dir():
                return candidate

    lib = library_root()
    want = norm(artist)

    # Most libraries are MUSIC/GENRE/ARTIST. Checking the first two levels is
    # far cheaper than rglob over every album folder.
    try:
        genre_dirs = [p for p in lib.iterdir() if p.is_dir()]
    except OSError:
        genre_dirs = []
    for folder in [lib, *genre_dirs]:
        if folder.is_dir() and (norm(folder.name) == want
                                or folder.name == sanitize(artist).upper()):
            return folder
    for genre_dir in genre_dirs:
        candidate = genre_dir / sanitize(artist).upper()
        if candidate.is_dir():
            return candidate
        try:
            children = list(genre_dir.iterdir())
        except OSError:
            continue
        for folder in children:
            if folder.is_dir() and norm(folder.name) == want:
                return folder

    # Existing state can point at an album folder even when no root was saved.
    # Try shallow paths first; only the final fallback walks the whole library.
    if state:
        folders = []
        for rec in (state.get('albums') or {}).values():
            rel = rec.get('folder')
            if rel and rel not in folders:
                folders.append(rel)
        for rel in folders:
            parts = Path(rel).parts
            for depth in (1, 2, 3):
                pattern = '/'.join(('*',) * depth + (rel.replace(os.sep, '/'),))
                try:
                    matches = lib.glob(pattern)
                except (OSError, ValueError):
                    continue
                for match in matches:
                    if not match.is_dir():
                        continue
                    root = match
                    for _part in parts:
                        root = root.parent
                    if root.is_dir() and root != lib:
                        return root

    for folder in lib.rglob('*'):
        if folder.is_dir() and norm(folder.name) == want:
            return folder
    return lib / (genre or 'UNCATEGORIZED') / sanitize(artist).upper()


def auto_inputs(artist, state_dir, log=None):
    page = f'{artist} discography'
    work = state_dir / slug(artist)
    work.mkdir(parents=True, exist_ok=True)
    a_path = work / 'albums.tsv'
    s_path = work / 'singles.json'
    if not a_path.exists() or not s_path.exists():
        if log:
            log(f'Wikipedia cache missing; discovering {page!r} '
                '(rate limits may briefly delay this)', flush=True)
        albs = fetch_wiki_albums(page)
        sngs = fetch_wiki_singles(page)
        a_path.write_text(''.join(f'{a["year"]}\t{a["title"]}\n' for a in albs),
                          encoding='utf-8')
        s_path.write_text(json.dumps(sngs, indent=2), encoding='utf-8')
        log(f'auto-discovered {len(albs)} release(s), {len(sngs)} singles from Wikipedia',
              flush=True)
    return a_path, s_path


def load_albums(path):
    out = []
    with open(path, encoding='utf-8') as fh:
        for line in fh:
            line = line.rstrip('\n')
            if not line or line.startswith('#'):
                continue
            parts = line.split('\t') if '\t' in line else line.split('|')
            if len(parts) >= 2:
                pinned = parts[2].strip() if len(parts) > 2 else ''
                title = parts[1].strip()
                # Tolerate a playlist URL pasted after the title with only a space.
                m = re.search(r'\s+(https?://\S+)$', title)
                if m:
                    pinned = pinned or m.group(1)
                    title = title[:m.start()].strip()
                out.append((parts[0].strip(), title, pinned))
    return out


def playlist_id_from(value):
    m = re.search(r'OLAK5uy_[A-Za-z0-9_-]+', value or '')
    return m.group(0) if m else None


def main():
    global DISPLAY, RETAG_CLI, RETAG_CONFIG_DIR
    epilog = """examples:
  get-catalog --artist 'Sade' --detach
      Download Sade's full catalog in the background.

  get-catalog --artists 'cleo sol, sade' --bandwidth 500k --detach
      Run both artists, each isolated by its own id; 500k is their COMBINED cap.

  get-catalog --artists['cleo sol', 'sade'] --bandwidth 500k --detach
      Bracket form works too. If both are downloading, each gets ~250k; if one
      finishes, the other picks up the full 500k on its next item.

  get-catalog --artist 'Cleo Sol' --status-only
      Show what is on disk, pending, already on an album, or unresolved.

  get-catalog --artist 'Bob Marley' --interactive
      Tick the exact albums/singles to keep from the full YTM index, then
      choose F for foreground or D for detached. A selects all, N clears all,
      Space toggles the cursor. The picker shows every release, but flat
      duplicate tracks are removed after confirmation and before downloading.

  get-catalog --artist 'Zach Bryan' --refresh-index --detach
      Rebuild the cached artist-page ID index, then use exact IDs instead of
      searching each track. Use --no-artist-index for the older resolver.

  get-catalog --artist 'Artist' --refresh-index --select \
      --ytm-channel 'https://music.youtube.com/channel/UC...'
      Merge the linked YouTube Music channel into the index, then pick from
      the combined list. This is for Topic-channel singles that the artist
      page omits. The channel id is cached for later refreshes. A handle URL
      such as --ytm-channel 'https://music.youtube.com/@Artist' is
      resolved to its UC id automatically.

  get-catalog --artist 'Zach Bryan' --retag
      Write RELEASE_TYPE, MUSICBRAINZ_ALBUMTYPE, EXPLICIT and ITUNESADVISORY
      tags to already-downloaded files without downloading anything. Normal
      downloads pass these to sonic-cli as --tag/--track-tag, so sonic-core
      writes them (and keeps Opus cover art) at download time.

  get-catalog --artist 'Cleo Sol'
      Foreground run with one row per in-flight download
      (N  label  downloaded/total [bar] %) above a TOTAL row with
      byte-weighted progress, elapsed clock and speed, all fed by sonic-cli's
      actual download bytes. The block grows downward as downloads start.

  get-catalog --artist 'Cleo Sol' --songs '["Butterfly", "One"]' --detach
      Download only those two tracks into CLEO SOL/SINGLES.

  get-catalog --artist 'Marvin Gaye' --select-albums '["Moods of Marvin Gaye", "M.P.G."]' --detach
      Download only those albums; singles are skipped.

  get-catalog --artist 'Marvin Gaye' --select-albums "1971 - What's Going On" --detach
      One album, matched by "YEAR - TITLE".

  get-catalog --artists['cleo sol', 'sade'] --albums[[1,2,3], [1,2]] --detach
      Per-artist album choices. Numbers are 1-based positions in each artist's
      discovered catalog; titles work too.

  get-catalog --artist 'Sade' --format mp3 --bitrate 320k --detach
      Override the output format and bitrate.

  get-catalog --artist 'Sade' --format mp4 --video-quality 1080p --detach
      Cap video resolution (the CLI calls this --video-quality).

  get-catalog --artist 'Sade' --cli-args '--concurrency 2 --no-id3' --detach
      Forward any other sonic-cli flag.

  get-catalog --artist 'Sade' --bandwidth 500K --detach
      Cap downloads at 500 KB/s total.

  get-catalog --artists['cleo sol', 'sade'] --concurrency 6 --detach
      Six downloads total: three per artist. No bandwidth cap here.

  get-catalog --artists['cleo sol', 'sade'] --concurrency 6 --bandwidth 500K --detach
      Six downloads total sharing one 500 KB/s cap (~83 KB/s per slot).

  get-catalog --artists['cleo sol', 'sade'] --concurrency 6
      Foreground multi-artist run: both artists start together, each with its
      own download rows and [artist] summary, and their singles
      download in parallel inside each lane.

  get-catalog --cancel strandz
      Cancel a detached catalog worker by artist name or slug. This signals
      the whole process group, including yt-dlp/ffmpeg children.

  get-catalog --artist 'Sade' -- --status-only
      An optional '--' separator is accepted before extra flags.

files (defaults; every location can be changed with an environment variable):
  state      $SONIC_STATE_DIR/<artist>.json
  pidfile    $SONIC_STATE_DIR/<artist>.catalog.pid
  log        $SONIC_LOG_DIR/<artist>.log
  cli config $SONIC_STATE_DIR/<artist>-cli-config
  cookies    $YTM_COOKIE_FILE            (/path/to/music.youtube.com_cookies.txt)
  sonic-cli  $SONIC_CLI                  (/path/to/tools/sonic-cli)
  yt-dlp     $SONIC_YTDLP                (/path/to/tools/sonic-yt-dlp)
  library    $SONIC_MUSIC_ROOT           (/path/to/music/library)

Run `python3 ytm_config.py` to print the paths that are currently in effect.

With --artists, <artist> above is the slug id (e.g. "cleo sol" -> "cleo-sol"),
so each artist's state, log, CLI config and yt-dlp config stay isolated.

Foreground and concurrency:
  Without --detach, every artist runs at the same time in one process. The
  display is drawn inline, like pacman: one row per in-flight download (with
  its own size and bracket bar) sits above a single TOTAL row, and the block
  grows downward as downloads start and shrinks as they finish. With
  --artists, each artist has its own rows and an [artist] A/S summary; log
  lines go to scrollback above the block and carry the same prefix. A
  worker processes up to its --album-concurrency share of albums at once; each
  album downloads --track-concurrency tracks in parallel. The product is the
  number of in-flight downloads and is capped at 16, so raising the track count
  can never multiply the album count into a process explosion. For multiple
  artists the album count is split evenly, except for track-only requests where
  the track count is split and each artist runs one album. A timed-out album is
  logged as FAILED (its partial folder is removed and state is saved) and the
  next item continues instead of aborting the run. With --detach, one detached
  worker still starts per artist and writes its own log file.

  --concurrency N (or --track-c N) means 1 album with N tracks. --album-c N
  alone means N albums with 1 track each. --album-c A --track-c T means A albums
  with T tracks each (A * T total). The default is A=6, T=1. A --concurrency
  inside --cli-args is a track-only request (1 album * N tracks). The full
  track budget also sizes the singles pool: several watch?v= downloads run at
  once, each in a private .download-* directory before being moved into
  SINGLES. Ctrl-C stops all artists and all in-flight downloads cleanly.
"""
    ap = argparse.ArgumentParser(
        prog='get-catalog',
        description='Download an artist\'s Wikipedia-approved albums and true '
                    'singles from YouTube Music. YTM supplies exact IDs, release '
                    'types and explicit flags: albums go under ARTIST/YYYY - Title, '
                    'EPs under ARTIST/EP, singles under ARTIST/SINGLES. Tracks that '
                    'belong to an album/EP are skipped as singles.',
        epilog=epilog,
        formatter_class=argparse.RawDescriptionHelpFormatter)

    core = ap.add_argument_group('core options')
    core.add_argument('--artist', metavar='NAME',
                      help='one artist name; used for Wikipedia lookup and to find '
                           'or create the library folder')
    core.add_argument('--artists', '--artist[]', '--artists[]', dest='artists',
                      action='append', metavar='LIST',
                      help='one or more artists: JSON array, comma-separated list, '
                           'or repeat the flag. The bracket form '
                           '--artists[\'a\', \'b\'] is also accepted. Each artist '
                           'gets its own id (slug), state, log, CLI config and '
                           'yt-dlp config under $SONIC_STATE_DIR')
    core.add_argument('--songs', action='append', metavar='TITLES',
                      help='download only these titles and skip the album catalog; '
                           'accepts a JSON array, a comma-separated list, or '
                           'repeat the flag. Album-track skipping is bypassed.')
    core.add_argument('--select-albums', '--albums[]', dest='select_albums',
                      action='append', metavar='TITLES',
                      help='download only these albums and skip singles; accepts '
                           'a JSON array, comma-separated titles, 1-based album '
                           'numbers, or repeat the flag. Titles, "YEAR - TITLE", '
                           'and OLAK5uy_ IDs work. With --artists, use '
                           '--albums[[...], [...]] for a per-artist list. '
                           '(\'--albums[]\' is accepted as an alias.)')
    core.add_argument('--interactive', '--select', dest='interactive_pick',
                      action='store_true',
                      help='show a checkbox picker before downloading: A selects '
                           'all, N clears all, Space toggles, Enter confirms; '
                           'then choose foreground or detached. Requires a '
                           'terminal and one artist.')
    core.add_argument('--cancel', metavar='NAME_OR_SLUG',
                      help='cancel a detached catalog session by artist name or '
                           'slug, then exit; signals the whole worker process '
                           'group')
    core.add_argument('--status-only', action='store_true',
                      help='print status without downloading: downloaded, on-disk, '
                           'pending, album-track, unresolved, failed')
    core.add_argument('--detach', action='store_true',
                      help='run in the background and append to the artist log '
                           'instead of printing to this terminal')

    place = ap.add_argument_group('library placement')
    place.add_argument('--root', metavar='DIR',
                       help='artist folder (downloads use a hidden .sonic-tmp '
                            'inside it; auto-detected if omitted)')
    place.add_argument('--genre', metavar='NAME',
                       help='genre folder to use only when a brand-new artist '
                            'folder has to be created (default: UNCATEGORIZED)')

    source = ap.add_argument_group('catalog sources')
    source.add_argument('--albums', metavar='TSV',
                        help='override the Wikipedia album list; each line is '
                             'YEAR<TAB>ALBUM TITLE[<TAB>OPTIONAL_YTM_PLAYLIST_URL]. '
                             'A pinned OLAK5uy_ URL/ID is used instead of search.')
    source.add_argument('--singles', metavar='JSON',
                        help='override the Wikipedia single list; '
                             'JSON array of {"title", "year"} objects')
    source.add_argument('--no-artist-index', action='store_true',
                        help='skip the fast YTM artist-page index and use the '
                             'older per-track search resolver')
    source.add_argument('--refresh-index', action='store_true',
                        help='rebuild the cached YTM artist-page index even if '
                             'it is still fresh')
    source.add_argument('--index-age', type=int, default=7, metavar='DAYS',
                        help='maximum age of a cached artist index (default: 7)')
    source.add_argument('--ytm-channel', metavar='URL_OR_ID_OR_HANDLE',
                        help='merge a YouTube Music channel into the artist '
                             'index; accepts a UC id, a channel URL, or an '
                             '@handle URL such as https://music.youtube.com/@Artist. '
                             'Topic channels may list singles that the artist '
                             'page omits. The resolved channel id is cached and '
                             'reused by later --refresh-index runs.')
    source.add_argument('--retag', action='store_true',
                        help='write RELEASE_TYPE, MUSICBRAINZ_ALBUMTYPE and '
                             'EXPLICIT tags to already-downloaded files; no '
                             'downloads are performed')
    source.add_argument('--albums-per-artist', action='append', metavar='JSON',
                        help=argparse.SUPPRESS)

    dl_group = ap.add_argument_group('sonic-cli download settings')
    dl_group.add_argument('--format', default='opus', metavar='FMT',
                          help='output format: opus, mp3, m4a, flac, wav, mp4, '
                               'webm, mkv (default: opus)')
    dl_group.add_argument('--bitrate', default='lossless', metavar='RATE',
                          help='bitrate for lossy formats: 128k/192k/256k/320k/'
                               'lossless (default: lossless)')
    dl_group.add_argument('--sample-rate', metavar='HZ',
                          help='lossless sample rate, e.g. 44100 or 48000')
    dl_group.add_argument('--video-quality', metavar='QUALITY',
                          help='video resolution cap for mp4: 360p..2160p or best')
    dl_group.add_argument('--cli-args', default='', metavar='FLAGS',
                          help='additional sonic-cli flags as one quoted string, '
                               'e.g. "--concurrency 2 --no-id3"')
    dl_group.add_argument('--cookies', default=DEFAULT_COOKIES, metavar='FILE',
                          help='Netscape cookies file for YouTube Music')
    dl_group.add_argument('--concurrency', '--global-concurrency',
                          dest='global_concurrency', type=int, metavar='N',
                          help='one album at a time with N tracks downloading '
                               'inside it (alias: --track-c N). With --artists '
                               'N is split across artists, one album each. '
                               'Total downloads are capped at 16.')
    dl_group.add_argument('--album-concurrency', '--album-c',
                          dest='album_concurrency', type=int, metavar='A',
                          help='A albums at once with 1 track each (default: 6). '
                               'Combined with --track-concurrency T it runs '
                               'A albums * T tracks, capped at 16; split evenly '
                               'with --artists')
    dl_group.add_argument('--track-concurrency', '--track-c',
                          dest='track_concurrency', type=int, metavar='T',
                          help='tracks downloaded at once inside each album '
                               '(alone: 1 album * T tracks; with '
                               '--album-concurrency A: A * T, max 16)')
    dl_group.add_argument('--bandwidth', '--limit-rate', dest='limit_rate',
                          metavar='RATE',
                          help='TOTAL download bandwidth cap, e.g. 500K or 2M '
                               '(bytes/second; K/M suffixes accepted). With one '
                               'artist it is the whole cap; with --artists it is '
                               'shared globally. The rate is divided by the '
                               'number of active track slots (albums * tracks '
                               'per album), so with 6 slots each download gets '
                               '~1/6 of the cap. An explicit --concurrency in '
                               '--cli-args is treated as --track-concurrency.')
    dl_group.add_argument('--config-dir', metavar='DIR',
                          help='isolated sonic-cli config dir (auto: one per artist; '
                               'prevents concurrent runs from crossing folders)')

    internals = ap.add_argument_group('state and binaries')
    internals.add_argument('--state', metavar='FILE',
                           help='state JSON file (auto: one per artist)')
    internals.add_argument('--ytdlp', default=DEFAULT_YTDLP, metavar='PATH',
                           help='yt-dlp binary used for discovery/validation')
    internals.add_argument('--cli', default=DEFAULT_CLI, metavar='PATH',
                           help='sonic-ripper-cli binary used for downloads')
    internals.add_argument('--binaries-dir', default=str(ytm_config.TOOLS_DIR),
                           metavar='DIR',
                           help='folder with sonic-yt-dlp/sonic-ffmpeg/sonic-deno '
                                'for downloads (default: SONIC_TOOLS_DIR)')
    internals.add_argument('--global-limit-dir', help=argparse.SUPPRESS)
    internals.add_argument('--worker-id', help=argparse.SUPPRESS)
    internals.add_argument('--worker-concurrency', type=int, help=argparse.SUPPRESS)
    internals.add_argument('--selected-albums', help=argparse.SUPPRESS)
    internals.add_argument('--selected-singles', help=argparse.SUPPRESS)
    internals.add_argument('--disk-album-titles', help=argparse.SUPPRESS)
    internals.add_argument('--run-id', help=argparse.SUPPRESS)
    internals.add_argument('--detached-run', help=argparse.SUPPRESS)
    # Allow `get-catalog --artist X -- --detach` as well as plain extra flags.
    raw_args = [a for a in sys.argv[1:] if a != '--']
    sys.argv = [sys.argv[0]] + normalize_album_args(normalize_artist_args(raw_args))
    args = ap.parse_args()
    RETAG_CLI = args.cli
    RETAG_CONFIG_DIR = args.config_dir
    if args.cancel is not None:
        state_dir = Path(args.state).parent if args.state else None
        ok = cancel_catalog_session(args.cancel, state_dir=state_dir)
        try:
            cleanup_artist_temp_after_cancel(args.cancel, args.root, args.genre)
        except Exception as exc:
            log(f'could not clean stale temp dirs: {exc}', flush=True)
        if not ok:
            raise SystemExit(1)
        return
    register_detached_worker(args)
    if args.limit_rate and not re.fullmatch(r'\d+(?:\.\d+)?[KMGkmg]?', args.limit_rate):
        ap.error('--bandwidth must be a rate like 500K or 2M (bytes/second)')
    album_matrix = parse_album_matrix(args.albums_per_artist, '--albums[...]')
    if album_matrix and args.select_albums:
        ap.error('use --select-albums or --albums[...] per-artist lists, not both')
    if album_matrix and args.songs:
        ap.error('use --songs or --albums[...] per-artist lists, not both')

    artist_names = ([args.artist] if args.artist else [])
    artist_names += parse_titles(args.artists, '--artists')
    seen, unique_artists = set(), []
    for artist in artist_names:
        key = artist.casefold()
        if key not in seen:
            seen.add(key)
            unique_artists.append(artist)
    artist_names = unique_artists
    if not artist_names:
        ap.error('--artist or --artists is required')
    if args.interactive_pick:
        if len(artist_names) != 1:
            ap.error('--interactive works with one --artist only')
        if args.detach:
            ap.error('--interactive cannot be combined with --detach; choose '
                     'the run mode in the picker instead')
        if args.status_only:
            ap.error('--interactive cannot be combined with --status-only')
        if args.retag:
            ap.error('--interactive cannot be combined with --retag')
        if not sys.stdin.isatty() or not sys.stdout.isatty():
            ap.error('--interactive requires a terminal')

    # ---- concurrency planning ------------------------------------------------
    # Three ways to ask for parallelism, all capped at MAX_TOTAL_SLOTS:
    #   --concurrency N                 1 album, N tracks at once
    #   --album-concurrency A           A albums, 1 track each
    #   --album-concurrency A --track-concurrency T   A albums, T tracks each
    # --track-concurrency T alone is shorthand for --concurrency T. A
    # --concurrency inside --cli-args is also a track-only request.
    explicit = explicit_concurrency(args.cli_args)
    for value, label in ((args.album_concurrency, '--album-concurrency'),
                         (args.track_concurrency, '--track-concurrency'),
                         (args.global_concurrency, '--concurrency')):
        if value is not None and not 1 <= value <= MAX_TOTAL_SLOTS:
            ap.error(f'{label} must be between 1 and {MAX_TOTAL_SLOTS}')

    if args.track_concurrency is not None:
        tracks = args.track_concurrency
        if explicit:
            log('note: --track-concurrency overrides --concurrency in '
                '--cli-args', flush=True)
    elif explicit:
        tracks = explicit
    elif args.global_concurrency is not None:
        tracks = args.global_concurrency
    else:
        tracks = 1
    track_only = (explicit is not None or args.global_concurrency is not None
                  or args.track_concurrency is not None)

    worker_share = getattr(args, 'worker_concurrency', None)
    if args.album_concurrency is not None:
        albums = args.album_concurrency
    elif worker_share:
        albums = worker_share
    elif track_only:
        albums = 1
        if explicit is not None and args.track_concurrency is None:
            log(f'note: --cli-args sets --concurrency {explicit}; using it as '
                'track concurrency with 1 album', flush=True)
    else:
        albums = 6

    # Single artist + bandwidth but no explicit slot setting: keep one at a
    # time so the cap really is the total, as before.
    if (args.limit_rate and not args.global_limit_dir and not track_only
            and args.album_concurrency is None):
        albums = 1

    if len(artist_names) > 1:
        if track_only and not args.album_concurrency and not worker_share:
            # One album per artist; the track budget is split across them so
            # --concurrency N still means N downloads in total.
            if tracks < len(artist_names):
                ap.error(f'--concurrency {tracks} is lower than the number of '
                         f'artists ({len(artist_names)}); raise it')
            per_artist = [1] * len(artist_names)
            per_artist_tracks = split_evenly(tracks, len(artist_names))
            total = tracks
            log(f'track concurrency {tracks} split across '
                  f'{len(artist_names)} artists: '
                  f'{dict(zip((slug(a) for a in artist_names), per_artist_tracks))}; '
                  '1 album each', flush=True)
        else:
            if albums < len(artist_names):
                ap.error(f'--album-concurrency {albums} is lower than the '
                         f'number of artists ({len(artist_names)}); raise it')
            per_artist = split_evenly(albums, len(artist_names))
            per_artist_tracks = [tracks] * len(artist_names)
            total = albums * tracks
            log(f'global album concurrency {albums} split across '
                  f'{len(artist_names)} artists: '
                  f'{dict(zip((slug(a) for a in artist_names), per_artist))}; '
                  f'{tracks} track(s) per album', flush=True)
    else:
        per_artist = [albums]
        per_artist_tracks = [tracks]
        total = albums * tracks

    if total > MAX_TOTAL_SLOTS:
        ap.error(f'requested {total} simultaneous downloads exceeds the '
                 f'{MAX_TOTAL_SLOTS}-download safety cap; lower '
                 '--album-concurrency or --track-concurrency')
    if os.environ.get('GETCATALOG_DEBUG_CONCURRENCY'):
        log(f'DEBUG concurrency: albums={albums} tracks={tracks} '
              f'total={total} per_artist={per_artist} '
              f'per_artist_tracks={per_artist_tracks} '
              f'explicit={explicit} cli_args={args.cli_args!r}', flush=True)

    if album_matrix:
        if len(album_matrix) != len(artist_names):
            ap.error(f'--albums[...] has {len(album_matrix)} artist entries but '
                     f'{len(artist_names)} artists were given')
        if len(artist_names) == 1 and album_matrix[0] is not None:
            args.select_albums = [json.dumps(album_matrix[0])]

    song_titles = parse_songs(args.songs)
    album_titles = parse_titles(args.select_albums, '--select-albums')
    if song_titles and album_titles:
        ap.error('use --songs or --select-albums, not both')

    args.batch_slots = per_artist[0] if len(artist_names) == 1 else albums
    args.batch_album_concurrency = albums
    args.batch_track_concurrency = (per_artist_tracks[0]
                                    if len(artist_names) == 1 else tracks)
    args.batch_total_slots = total
    args.batch_explicit = explicit
    interactive = (not args.detach and not args.status_only
                   and sys.stdout.isatty())

    if len(artist_names) > 1:
        for flag, value in (('--root', args.root), ('--state', args.state),
                            ('--config-dir', args.config_dir),
                            ('--albums', args.albums), ('--singles', args.singles)):
            if value:
                ap.error(f'{flag} cannot be combined with multiple artists; '
                         'each artist uses its own id-based paths')
        state_parent = (Path(args.state).parent if args.state else
                        ytm_config.STATE_DIR)
        ids = [slug(a) for a in artist_names]
        global_dir = None
        if args.limit_rate:
            ledger_tag = '-'.join(sorted(ids))[:100]
            global_dir = state_parent / f'global-{args.limit_rate}-c{total}-{ledger_tag}'
            init_global_bandwidth(
                global_dir, parse_rate(args.limit_rate), ids,
                concurrency={aid: share * t for aid, share, t in
                             zip(ids, per_artist, per_artist_tracks)})

        if args.detach:
            rate_opts = ('--bandwidth', '--limit-rate')
            conc_opts = ('--concurrency', '--global-concurrency')
            skip_opts = ('--artists', '--artist', '--artist[]', '--artists[]',
                         '--albums-per-artist', '--album-concurrency', '--album-c',
                         '--track-concurrency', '--track-c', '--cancel',
                         '--run-id', '--detached-run')
            if album_matrix:
                skip_opts += ('--select-albums', '--albums[]')
            worker_extra, i, argv_tail = [], 0, sys.argv[1:]
            while i < len(argv_tail):
                a = argv_tail[i]
                if a in skip_opts:
                    i += 2
                    continue
                if any(a.startswith(opt + '=') for opt in skip_opts):
                    i += 1
                    continue
                if global_dir and a in rate_opts:
                    i += 2
                    continue
                if global_dir and any(a.startswith(opt + '=') for opt in rate_opts):
                    i += 1
                    continue
                if a in conc_opts:
                    i += 2
                    continue
                if any(a.startswith(opt + '=') for opt in conc_opts):
                    i += 1
                    continue
                if a != '--detach':
                    worker_extra.append(a)
                i += 1

            cmd_base = [sys.executable, str(Path(__file__).resolve())]
            cmd_base += worker_extra + ['--artist']
            logdir = ytm_config.LOG_DIR
            logdir.mkdir(parents=True, exist_ok=True)
            for idx, (artist, aid) in enumerate(zip(artist_names, ids)):
                cmd = cmd_base + [artist]
                if album_matrix and album_matrix[idx] is not None:
                    cmd += ['--select-albums', json.dumps(album_matrix[idx])]
                cmd += ['--worker-concurrency', str(per_artist[idx])]
                cmd += ['--track-concurrency', str(per_artist_tracks[idx])]
                if global_dir:
                    cmd += ['--global-limit-dir', str(global_dir), '--worker-id', aid]
                logfile = logdir / f'{aid}.log'
                spawn_detached_process(cmd, aid, logfile=logfile,
                                       state_dir=state_parent)
                log(f'started {artist} [id: {aid}]; log: {logfile}', flush=True)
        else:
            # Foreground: all artists run at once, each in its own thread with
            # its own progress lane. No subprocesses, no interleaved UI.
            DISPLAY = Display(artist_names, enabled=interactive,
                              slots=[share * t for share, t in
                                     zip(per_artist, per_artist_tracks)])
            args.batch_worker_ids = ids
            args.global_limit_dir = global_dir
            install_signal_handlers(args)

            def worker(idx, artist, wargs):
                try:
                    if DISPLAY and DISPLAY.lanes[idx]:
                        PROGRESS.bind(DISPLAY.lanes[idx])
                    run_artist(wargs, artist, song_titles, wargs.batch_album_titles)
                except (KeyboardInterrupt, InterruptedError):
                    log('interrupted', flush=True)
                except BaseException as exc:
                    log(f'{artist}: ERROR: {exc}', flush=True)
                finally:
                    PROGRESS.unbind()

            threads = []
            for idx, (artist, aid) in enumerate(zip(artist_names, ids)):
                wargs = copy.copy(args)
                wargs.artist = artist
                wargs.worker_id = aid
                wargs.global_limit_dir = global_dir
                wargs.worker_concurrency = per_artist[idx]
                wargs.batch_slots = per_artist[idx]
                wargs.batch_track_concurrency = per_artist_tracks[idx]
                wargs.batch_total_slots = per_artist[idx] * per_artist_tracks[idx]
                if album_matrix and album_matrix[idx] is not None:
                    wargs.select_albums = [json.dumps(album_matrix[idx])]
                    wargs.batch_album_titles = parse_titles(
                        wargs.select_albums, '--select-albums')
                else:
                    wargs.batch_album_titles = [] if album_matrix else album_titles
                t = threading.Thread(target=worker, args=(idx, artist, wargs),
                                     name=aid, daemon=True)
                t.start()
                threads.append(t)
                log(f'started {artist} [id: {aid}]', flush=True)
            interrupted = False
            try:
                for t in threads:
                    t.join()
            except KeyboardInterrupt:
                interrupted = True
                ABORT.set()
                kill_all_children()
                for t in threads:
                    t.join(timeout=5)
            for t in threads:
                t.join(timeout=1)
            if DISPLAY:
                DISPLAY.close()
            if interrupted:
                raise KeyboardInterrupt
        if global_dir:
            log(f'global bandwidth cap {args.limit_rate} split over '
                  f'{total} total slots; state: {global_dir}', flush=True)
        else:
            log(f'global concurrency {total} across {len(ids)} artist(s); '
                  f'no bandwidth cap set', flush=True)
        return

    name = slug(artist_names[0])
    DISPLAY = Display([None], enabled=interactive, slots=track_slots(args))
    if interactive:
        PROGRESS.bind(DISPLAY.lanes[0])
    args.batch_worker_ids = [slug(artist_names[0])]
    install_signal_handlers(args)

    if args.detach:
        cmd = [sys.executable, str(Path(__file__).resolve())]
        skip = ('--detach', '--cancel', '--run-id', '--detached-run')
        cmd += [a for a in sys.argv[1:] if a not in skip]
        logfile = ytm_config.LOG_DIR / f'{name}.log'
        state_dir = Path(args.state).parent if args.state else None
        spawn_detached_process(cmd, name, logfile=logfile, state_dir=state_dir)
        log(f'started in background; log: {logfile}')
        return

    try:
        run_artist(args, artist_names[0], song_titles, album_titles)
    finally:
        if DISPLAY:
            DISPLAY.close()


def run_artist(args, artist, song_titles, album_titles):
    """Run one artist's catalog. Safe to call from a worker thread."""
    args.artist = artist
    name = slug(args.artist)
    state_dir = (Path(args.state).parent if args.state else
                 ytm_config.STATE_DIR)
    if args.limit_rate and not args.global_limit_dir:
        # Single artist: no shared ledger, so divide the total ourselves.
        share = track_slots(args)
        rate = parse_rate(args.limit_rate) // share
        ytdlp_cfg = set_local_rate_env(args, rate, state_dir, name)
        log(f'bandwidth cap {args.limit_rate} B/s total '
              f'({format_rate(rate)} per download x {share}; {ytdlp_cfg})',
              flush=True)

    state_path = (Path(args.state) if args.state else state_dir / f'{name}.json')
    state_path.parent.mkdir(parents=True, exist_ok=True)
    config_dir = (Path(args.config_dir) if args.config_dir else
                  state_path.parent / f'{name}-cli-config')
    config_dir.mkdir(parents=True, exist_ok=True)
    args.cli_config_dir = str(config_dir)
    use_index_catalog = (not args.albums and not args.singles and not song_titles
                         and not args.no_artist_index)
    if not args.albums and not song_titles and not use_index_catalog:
        args.albums, args.singles = auto_inputs(args.artist, state_path.parent, log)
    state = load_state(state_path)
    state.setdefault('albums', {})
    state.setdefault('singles', {})
    root = resolve_root(args.artist, args.root, args.genre, state)
    root.mkdir(parents=True, exist_ok=True)
    if state.get('root') != str(root):
        state['root'] = str(root)
    interactive_full = bool(args.interactive_pick and args.selected_albums is None
                            and use_index_catalog and not album_titles)
    if song_titles:
        albums = []
        singles = [{'title': t, 'year': ''} for t in song_titles]
        log(f'songs mode: {len(song_titles)} requested track(s)', flush=True)
    elif use_index_catalog:
        albums, singles = [], []
    else:
        albums = load_albums(args.albums)
        if args.singles:
            with open(args.singles, encoding='utf-8') as fh:
                singles = json.load(fh)
        else:
            singles = []
        if album_titles:
            if not use_index_catalog:
                albums = select_albums(albums, album_titles)
            singles = []

    disk_album_titles = set()
    if args.selected_albums is not None and args.disk_album_titles is not None:
        # The interactive parent already did this scan; don't make the
        # detached child ffprobe the whole library again.
        disk_album_titles = set(json.loads(args.disk_album_titles))
    else:
        for folder in root.iterdir():
            if is_library_folder(folder):
                disk_album_titles.update(norm(t) for t in folder_titles(folder))

    def report():
        from collections import Counter
        album_status, single_status = Counter(), Counter()
        for year, title, pinned in albums:
            rec = state['albums'].get(title, {})
            st = rec.get('status', 'pending')
            if st in ('pending', 'failed', 'unresolved'):
                found = find_existing_album(root, title, year, args.artist)
                if found:
                    st = f'on-disk ({found.name})'
            album_status[st.split(' ')[0]] += 1
            log(f'ALBUM [{st}] {year} {title}')
        for s in singles:
            title = s['title']
            rec = state['singles'].get(title, {})
            st = rec.get('status', 'pending')
            if st.startswith('skipped-ep-track'):
                st = 'ep-track'
            elif st.startswith('skipped-album-track'):
                st = 'album-track'
            if st in ('pending', 'failed', 'unresolved') and not song_titles:
                if not song_titles and norm(title) in disk_album_titles:
                    st = 'album-track'
                else:
                    found = find_existing_single(root, title)
                    if found:
                        st = f'on-disk ({found.name})'
            single_status[st.split(' ')[0]] += 1
            log(f'SINGLE [{st}] {title}')

        def tally(label, counts, ready):
            ready_n = sum(counts[x] for x in ready)
            detail = ', '.join(f'{k}: {v}' for k, v in sorted(counts.items()) if v)
            log(f'{label} {ready_n}/{sum(counts.values())} done'
                  + (f'  ({detail})' if detail else ''))

        tally('ALBUMS ', album_status, ('downloaded', 'on-disk'))
        tally('SINGLES', single_status,
              ('downloaded', 'on-disk', 'album-track', 'ep-track'))
        ready = (sum(album_status[x] for x in ('downloaded', 'on-disk'))
                 + sum(single_status[x] for x in
                       ('downloaded', 'on-disk', 'album-track', 'ep-track')))
        total = sum(album_status.values()) + sum(single_status.values())
        log(f'TOTAL   {ready}/{total} done')

    # Fast path: mimic a user searching the artist, clicking their profile and
    # reading the exact IDs off the artist page. Cached per artist; the old
    # per-track search remains available only when the index is explicitly disabled.
    artist_index = None
    index_path = state_dir / f'{name}-ytm-index.json'
    if not args.no_artist_index:
        artist_index = ytm_index.load(index_path, args.index_age * 86400)
        if (artist_index and
                (artist_index.get('artist') or '').casefold() != args.artist.casefold()):
            artist_index = None
    # Reuse a cached channel even when the index itself is stale. Passing the
    # loaded index avoids a second file read on the normal cached path.
    cached_channel = cached_channel_id(index_path, artist_index)
    requested_channel = args.ytm_channel or cached_channel
    channel_id = (resolve_channel_argument(requested_channel, args.ytdlp, log)
                  if requested_channel else None)
    if channel_id:
        # Store the canonical id in the index, not the handle the user typed.
        requested_channel = channel_id
    if args.ytm_channel and not channel_id:
        raise SystemExit(f'--ytm-channel could not be resolved to a UC channel '
                         f'id: {args.ytm_channel}')
    if args.no_artist_index:
        log('artist index disabled (--no-artist-index)', flush=True)
    else:
        needs_rebuild = (args.refresh_index or artist_index is None or
                         bool(channel_id and artist_index and
                              artist_index.get('channelBrowseId') != channel_id))
        if args.selected_albums is not None:
            # The detached child already has exact playlist IDs from the
            # picker; only use a cached index for membership checks.
            if artist_index:
                log(f'artist index loaded from cache for membership checks: '
                    f'{index_path}', flush=True)
            else:
                log('artist index unavailable; using the selected playlist IDs '
                    'directly', flush=True)
        elif needs_rebuild:
            started = time.monotonic()
            try:
                artist_index = ytm_index.build(args.artist, note=log,
                                               channel=requested_channel)
                ytm_index.save(index_path, artist_index)
                log(f'artist index built in {time.monotonic() - started:.1f}s: '
                      f'{index_path}', flush=True)
            except Exception as exc:
                artist_index = None
                log(f'artist index unavailable ({exc}); '
                      'Wikipedia entries without an index will be skipped', flush=True)
        else:
            log(f'artist index loaded from cache: {index_path}', flush=True)
            if artist_index.get('channelBrowseId'):
                log(f'index includes YTM channel '
                    f'{artist_index["channelBrowseId"]} '
                    f'({artist_index.get("channelName") or ""})', flush=True)
    # The build path already saved; membership/dedupe may mutate the loaded
    # index later. Compare against this snapshot to skip a redundant rewrite.
    saved_index_signature = index_signature(artist_index)

    if args.selected_albums is not None:
        albums = [tuple(row) for row in json.loads(args.selected_albums)]
        singles = json.loads(args.selected_singles or '[]')
        log(f'interactive selection restored: {len(albums)} album(s) and '
            f'{len(singles)} single(s)', flush=True)
    elif not args.no_artist_index:
        if album_titles:
            # An explicit --select-albums request overrides the Wikipedia
            # reference and selects directly from the complete YTM catalog.
            albums, singles = catalog_from_index(artist_index)
            albums = select_albums(albums, album_titles)
            singles = []
        elif interactive_full:
            # With a picker the user is the curator: show every exact YTM
            # release, including singles that also appear inside an album.
            # Wikipedia filtering and the duplicate skip would hide choices
            # before they can be selected.
            albums, singles = catalog_from_index(artist_index)
            log(f'interactive catalog: showing all {len(albums)} YTM album(s) '
                f'and {len(singles)} single(s)', flush=True)
        elif use_index_catalog:
            try:
                wiki_albums, wiki_singles = wiki_reference_lists(
                    args.artist, state_dir, log)
            except Exception as exc:
                # No reachable Wikipedia list means the documented fallback:
                # take every release YTM knows about, with no search resolver.
                wiki_albums, wiki_singles = [], []
                log(f'Wikipedia reference unavailable ({exc}); '
                    'using full YTM catalog', flush=True)
            albums, singles = catalog_from_wiki(
                artist_index, args.artist, wiki_albums, wiki_singles, log)
        else:
            indexed_albums = (artist_index or {}).get('albums', {})
            indexed_singles = (artist_index or {}).get('tracks', {})
            old_album_count, old_single_count = len(albums), len(singles)
            albums = [entry for entry in albums
                      if album_index_key(entry[2] or entry[1], args.artist) in indexed_albums]
            singles = [entry for entry in singles
                       if ytm_index.normkey(entry['title'], args.artist) in indexed_singles]
            log(f'YTM index filter: kept {len(albums)}/{old_album_count} albums and '
                f'{len(singles)}/{old_single_count} singles', flush=True)

    if album_titles:
        log(f'albums mode: {len(albums)} selected album(s); singles skipped',
            flush=True)

    if args.interactive_pick and args.selected_albums is None:
        if DISPLAY:
            DISPLAY.suspend()
        try:
            albums, singles, run_mode = interactive_catalog_filter(
                albums, singles, args.artist)
        except SelectionCancelled:
            log('selection cancelled; nothing downloaded', flush=True)
            return
        if run_mode == 'detached':
            if DISPLAY:
                DISPLAY.close()
            spawn_detached_selection(args, name, albums, singles,
                                     disk_album_titles)
            return
        if DISPLAY:
            DISPLAY.resume()

    # Flat YTM ownership is computed from the final selection, not from the
    # full interactive catalog. The picker showed everything; after confirmation
    # duplicates are excluded before any worker starts. --songs stays bypassed.
    album_keys = {album_index_key(title, args.artist)
                  for _year, title, _pinned in albums}
    if artist_index and not song_titles:
        # Playlists can gain tracks after a member list was cached. Refresh the
        # selected releases only, then recompute ownership from the merged rows.
        ytm_index.refresh_release_members(
            artist_index, sorted(album_keys), note=log)
        ytm_index.prune_duplicate_members(
            artist_index, args.artist, note=log,
            fetch=not artist_index.get('flatDedupe'))

    # Exact album/EP membership is what tells us a Wikipedia "single" is really
    # an album or EP track. The fetched member lists are cached in the index.
    video_release = build_membership(artist_index, args.artist, albums, log)

    # Only skip a duplicated album track when its owning album/EP is part of
    # this run or already on disk. The library scan also catches playlist edits
    # that the cached member list has not seen yet.
    album_exclusions, excluded_count, disk_count = album_duplicate_exclusions(
        artist_index, album_keys, disk_album_titles)
    owners = (artist_index or {}).get('trackOwners') or {}
    excluded_members = (artist_index or {}).get('excludedMembers') or {}
    if excluded_count:
        log(f'flat YTM dedupe: excluding {excluded_count} duplicate track(s) '
            'from later selected album(s)', flush=True)
    if disk_count:
        log(f'library scan: excluding {disk_count} album/EP track(s) already '
            'on disk', flush=True)

    if (artist_index and index_path.exists()
            and index_signature(artist_index) != saved_index_signature):
        ytm_index.save(index_path, artist_index)

    singles = dedupe_singles_for_albums(
        artist_index, args.artist, albums, singles, disk_album_titles,
        video_release, log, song_titles=bool(song_titles))

    PROGRESS.set_queue([a[1] for a in albums], [s['title'] for s in singles])

    if args.status_only:
        report()
        return

    download_tmp_dir = root.resolve() / '.sonic-tmp'
    download_tmp_dir.mkdir(parents=True, exist_ok=True)
    clean_stale_tmp_runs(download_tmp_dir)
    RUN_CONTEXT.tmp_dir = download_tmp_dir

    if args.retag:
        retag_library(args, state, root, state_path, artist_index, log)
        report()
        log(f'state: {state_path}', flush=True)
        return

    # albums: discover -> validate -> select -> download, with the same worker
    # limit used for singles and split across artists by --concurrency.
    def save_album(title, rec):
        with STATE_LOCK:
            state['albums'][title] = rec
            save_state(state_path, state)

    def process_album(i, entry, lane=None):
        if lane is not None:
            PROGRESS.bind(lane)
        RUN_CONTEXT.tmp_dir = download_tmp_dir
        year, title, pinned = entry
        work = None
        try:
            PROGRESS.set_current(f'[{i}/{len(albums)}] {title} ({year})')
            if (not args.no_artist_index and
                    album_index_key(title, args.artist) not in
                    (artist_index or {}).get('albums', {})):
                log(f'[{i}/{len(albums)}] SKIP not in YTM index: {title}',
                    flush=True)
                return
            with STATE_LOCK:
                rec = copy.deepcopy(state['albums'].get(title, {}))
            rec['year'] = year
            idx_info = index_album(artist_index, args.ytdlp, args.artist, title, year)
            rtype = (idx_info or {}).get('type') or rec.get('type') or 'album'
            album_root = root / 'EP' if rtype == 'ep' else root
            album_root.mkdir(exist_ok=True)
            existing = find_existing_album(root, title, year, args.artist)
            if existing:
                fixed = fix_year(album_root, existing, year, title)
                rec.update(status='downloaded', type=rtype,
                           folder=fixed.relative_to(root).as_posix())
                save_album(title, rec)
                log(f'[{i}/{len(albums)}] SKIP {rtype.upper()}: {title}', flush=True)
                return
            if (rec.get('status') == 'downloaded' and rec.get('folder')
                    and (root / rec['folder']).exists()):
                fixed = fix_year(album_root, root / rec['folder'], year, title)
                rec['folder'] = fixed.relative_to(root).as_posix()
                save_album(title, rec)
                log(f'[{i}/{len(albums)}] SKIP {rtype.upper()}: {title}', flush=True)
                return

            log(f'[{i}/{len(albums)}] discovering {rtype}: {title} ({year})',
                flush=True)
            best, best_score = None, -999
            pinned_id = playlist_id_from(pinned)
            if pinned_id:
                info = validate(args.ytdlp, pinned_id, f'Album - {title}')
                cands = [{'id': pinned_id, **info}] if info else []
                if not cands:
                    log(f'    PINNED PLAYLIST INVALID: {pinned_id}', flush=True)
                rec['validated'] = cands
                save_album(title, rec)
            else:
                cands = []
                if idx_info:
                    cands = [{'id': idx_info['playlist_id'], **idx_info}]
                    log(f'    index: {idx_info["playlist_id"]} '
                        f'({idx_info.get("date") or year}, {rtype})', flush=True)
                if not cands:
                    cands = rec.get('validated') or []
                if not cands:
                    for browse in album_browses(args.ytdlp, args.artist, title, year):
                        got = playlist_id(args.ytdlp, browse)
                        if not got:
                            continue
                        pid, ptitle = got
                        info = validate(args.ytdlp, pid, ptitle)
                        if info:
                            cands.append({'id': pid, **info})
                    rec['validated'] = cands
                    save_album(title, rec)
            rec['validated'] = cands
            save_album(title, rec)
            for cand in cands:
                sc = score(title, year, args.artist, cand)
                tracks = set(norm(t) for t in cand.get('titles', []))
                if tracks:
                    for base in (root, root / 'EP'):
                        if not base.is_dir():
                            continue
                        for f in base.iterdir():
                            if not is_library_folder(f):
                                continue
                            if album_key(f.name, args.artist) == album_key(title, args.artist):
                                continue
                            have = set(norm(t) for t in folder_titles(f))
                            if len(tracks & have) / len(tracks) >= 0.7:
                                sc -= 50
                                break
                        else:
                            continue
                        break
                log(f'    {sc:3d} {cand["id"]} | {cand.get("playlist")} '
                    f'| date={cand.get("date")} album={cand.get("album")}', flush=True)
                if sc > best_score:
                    best, best_score = cand, sc
            if not best or best_score < 20:
                rec['status'] = 'unresolved'
                save_album(title, rec)
                log(f'    UNRESOLVED: {title}', flush=True)
                return
            file_sizes = best.get('file_bytes') or rec.get('file_bytes')
            if not file_sizes:
                file_sizes = playlist_sizes(args.ytdlp, best['id'])
            estimate = sum(file_sizes) if file_sizes else 0
            if estimate:
                rec['bytes'] = estimate
                rec['file_bytes'] = file_sizes
                PROGRESS.set_weight(title, estimate)
            release_key = (best.get('release_key') or
                           ytm_index.normkey(title, args.artist))
            members = (artist_index or {}).get('members', {}).get(release_key, [])
            exclude_ids = album_exclusions.get(release_key) or []
            exclude_set = set(exclude_ids)
            if exclude_ids:
                owner = (owners.get(
                    (excluded_members.get(release_key) or [{}])[0].get('ownerKey')
                ) or {})
                kept_in = owner.get('title') or 'an earlier release or library folder'
                log(f'    flat dedupe: skip {len(exclude_ids)} track(s) already '
                    f'kept in {kept_in}', flush=True)
            display_artist = (artist_index or {}).get('artistName') or args.artist
            fallback = (idx_info or {}).get('explicit')
            if fallback is None:
                fallback = rec.get('explicit')
            # Do not override `artist` here: sonic-core enriches each track from
            # the exact YTM entry and joins every credited performer with ", ".
            # The album artist is passed as a protected hint so filtering cannot
            # drop it from the artist tag.
            updates = album_updates(rtype, year, fallback)
            flags = tag_flags(updates)
            hints = performer_hint_flags([display_artist, args.artist])
            for member in members:
                if member.get('videoId') in exclude_set:
                    continue
                if member.get('explicit') is None:
                    continue
                key = member.get('videoId') or member.get('title')
                if key:
                    flags += track_tag_flags(key, explicit_updates(member['explicit']))
            work = album_root / f'.download-{uuid.uuid4().hex}'
            work.mkdir()
            register_temp_dir(work)
            tracks = int(best.get('tracks') or len(file_sizes) or 12)
            timeout = max(900, min(3600, 90 * tracks))
            override = os.environ.get('SONIC_BATCH_ALBUM_TIMEOUT')
            if override:
                timeout = max(1, int(override))
            try:
                p = download(args, ['--is-album', '--folder-pattern', 'year_album',
                                    '--naming', 'number_title',
                                    '--output-dir', str(work), '--skip-missing',
                                    *exclude_track_flags(exclude_ids),
                                    *flags, *hints,
                                    '--url', f'https://music.youtube.com/playlist?list={best["id"]}'],
                            timeout=timeout, total_bytes=estimate or 0,
                            file_totals=file_sizes, label=title,
                            concurrency=args.batch_track_concurrency)
            except subprocess.TimeoutExpired:
                shutil.rmtree(work, ignore_errors=True)
                rec.update(status='failed', playlist=best['id'],
                           error=f'timeout after {timeout}s')
                save_album(title, rec)
                log(f'    TIMEOUT: {title} after {timeout}s; partial album removed; '
                    'rerun to resume', flush=True)
                return
            except Exception as exc:
                shutil.rmtree(work, ignore_errors=True)
                rec.update(status='failed', playlist=best['id'],
                           error=f'{type(exc).__name__}: {exc}')
                save_album(title, rec)
                log(f'    ERROR: {title}: {exc}; rerun to resume', flush=True)
                return
            new = {f.name for f in work.iterdir() if f.is_dir()}
            if new:
                want = album_key(title, args.artist)
                created = [work / n for n in new]
                match = next((f for f in created if album_key(f.name, args.artist) == want), None)
                chosen = match or (created[0] if len(created) == 1 else sorted(created)[0])
                final = album_root / chosen.name
                chosen.rename(final)
                fixed = fix_year(album_root, final, year, title)
                rec.update(status='downloaded', type=rtype, playlist=best['id'],
                           folder=fixed.relative_to(root).as_posix())
                save_album(title, rec)
                log(f'    DOWNLOADED: {rec["folder"]}', flush=True)
            else:
                rec['status'] = 'failed'
                save_album(title, rec)
                err = '\n'.join((p.stdout + p.stderr).splitlines()[-8:])
                log(f'    FAILED: {err}', flush=True)
            shutil.rmtree(work, ignore_errors=True)
        finally:
            if work is not None:
                unregister_temp_dir(work)
                shutil.rmtree(work, ignore_errors=True)
            PROGRESS.complete(title, 'albums')
            if lane is not None:
                PROGRESS.unbind()

    pool_size = max(1, int(worker_slots(args)))
    if pool_size > 1 and len(albums) > 1:
        lane = PROGRESS.current_lane()
        with ThreadPoolExecutor(max_workers=pool_size) as pool:
            futures = [pool.submit(process_album, i, entry, lane)
                       for i, entry in enumerate(albums, 1)]
            for future in futures:
                try:
                    future.result()
                except (KeyboardInterrupt, InterruptedError):
                    raise
                except BaseException as exc:
                    log(f'album worker error: {exc}', flush=True)
    else:
        for i, entry in enumerate(albums, 1):
            process_album(i, entry)

    # singles: exact YTM membership is authoritative; on-disk titles remain a
    # fallback for --no-artist-index runs.
    album_titles = set()
    for base in (root, root / 'EP'):
        if not base.is_dir():
            continue
        for folder in base.iterdir():
            if is_library_folder(folder):
                for t in folder_titles(folder):
                    album_titles.add(norm(t))

    dest = root / 'SINGLES'
    dest.mkdir(exist_ok=True)

    def commit_single(title, rec):
        with STATE_LOCK:
            state['singles'][title] = rec
            save_state(state_path, state)

    def process_single(i, s, lane=None):
        # Pool threads do not inherit the artist thread's thread-local binding.
        if lane is not None:
            PROGRESS.bind(lane)
        RUN_CONTEXT.tmp_dir = download_tmp_dir
        title = s['title']
        try:
            PROGRESS.set_current(f'[{i}/{len(singles)}] {title}')
            if (not args.no_artist_index and
                    ytm_index.normkey(title, args.artist) not in
                    (artist_index or {}).get('tracks', {})):
                log(f'[{i}/{len(singles)}] SKIP not in YTM index: {title}',
                    flush=True)
                return
            rec = state['singles'].get(title, {})
            if not rec.get('url'):
                url, meta = index_single(artist_index, args.artist, title) or (None, {})
                if not url:
                    if artist_index and not args.no_artist_index:
                        rec['status'] = 'unresolved'
                        commit_single(title, rec)
                        log(f'[{i}/{len(singles)}] no playable YTM index entry: {title}',
                            flush=True)
                        return
                    log(f'[{i}/{len(singles)}] index miss, searching: {title}',
                        flush=True)
                    url, meta = resolve_single(args.ytdlp, args.artist, title)
                if not url:
                    rec['status'] = 'unresolved'
                    commit_single(title, rec)
                    log(f'[{i}/{len(singles)}] UNRESOLVED: {title}', flush=True)
                    return
                rec.update(url=url, meta=meta)
            url = rec['url']
            meta = rec.get('meta') or {}
            vid = meta.get('video_id') or video_id_from(url)
            if vid and meta.get('explicit') is None:
                meta['explicit'] = explicit_for_video(artist_index, vid)
            explicit = meta.get('explicit')
            if vid:
                rec['meta'] = meta

            # A Wikipedia single that is a member of an album or EP is never
            # downloaded into SINGLES, even when that release is not on disk yet.
            rel = video_release.get(vid) if vid else None
            if not rel:
                # YTM often gives a standalone single a different video ID from the
                # copy embedded in an album/EP, so exact IDs alone are not enough.
                rel = (artist_index or {}).get('memberTitles', {}).get(
                    ytm_index.normkey(title))
            if rel and not song_titles:
                status = 'skipped-ep-track' if rel['type'] == 'ep' else 'skipped-album-track'
                PROGRESS.set_weight(title, 0)
                rec.update(status=status, release=rel['title'], type=rel['type'],
                           explicit=explicit)
                commit_single(title, rec)
                log(f'[{i}/{len(singles)}] SKIP {rel["type"]} track: {title} '
                      f'(in {rel["title"]})', flush=True)
                return
            existing = find_existing_single(root, title)
            if existing or (rec.get('status') == 'downloaded'
                            and (dest / rec.get('file', '')).exists()):
                rec.update(status='downloaded', file=existing.name if existing else rec.get('file'))
                commit_single(title, rec)
                log(f'[{i}/{len(singles)}] SKIP single: {title}', flush=True)
                return
            if not song_titles and norm(title) in album_titles:
                PROGRESS.set_weight(title, 0)
                rec.update(status='skipped-album-track', explicit=explicit)
                commit_single(title, rec)
                log(f'[{i}/{len(singles)}] SKIP album track: {title}', flush=True)
                return

            single_size = int(rec.get('bytes') or DEFAULT_SINGLE_BYTES)
            PROGRESS.set_weight(title, single_size)
            artist = (meta.get('artist') or
                      (artist_index or {}).get('artistName') or args.artist)
            # Keep `artist` for the final filename only. The tag itself comes
            # from sonic-core's YTM enrichment, which preserves credited
            # performers only (e.g. "Drake, Central Cee") and drops writers.
            updates = single_updates(title, explicit)
            hints = performer_hint_flags([artist, args.artist])
            # Each parallel single downloads into its own hidden directory so
            # one thread can never pick up another thread's finished file.
            work = dest / f'.download-{uuid.uuid4().hex}'
            work.mkdir(exist_ok=True)
            register_temp_dir(work)
            try:
                p = download(args, ['--naming', 'artist_title',
                                    '--output-dir', str(work),
                                    '--skip-missing', *tag_flags(updates),
                                    *hints, '--url', url],
                             timeout=600, total_bytes=single_size,
                             file_totals=[single_size], label=title)
                new = audio_files(work)
                if not new:
                    rec['status'] = 'failed'
                    commit_single(title, rec)
                    err = '\n'.join((p.stdout + p.stderr).splitlines()[-8:])
                    log(f'[{i}/{len(singles)}] FAILED: {title}: {err}', flush=True)
                    return
                f = sorted(new)[0]
                variant = explicit_variant(artist_index, title, explicit)
                stem = f'{sanitize(artist)} - {sanitize(title)}'
                if variant:
                    stem += f' ({variant})'
                target = dest / f'{stem}{f.suffix}'
                if target.exists():
                    rec.update(status='downloaded', file=target.name,
                               type='single', explicit=explicit,
                               bytes=single_size)
                    commit_single(title, rec)
                    log(f'[{i}/{len(singles)}] SKIP single: {target.name}',
                        flush=True)
                    return
                f.rename(target)
                rec.update(status='downloaded', file=target.name, type='single',
                           explicit=explicit, bytes=single_size)
                commit_single(title, rec)
                log(f'[{i}/{len(singles)}] DOWNLOADED: {target.name}', flush=True)
            finally:
                unregister_temp_dir(work)
                shutil.rmtree(work, ignore_errors=True)

        finally:
            PROGRESS.complete(title, 'singles')
            if lane is not None:
                PROGRESS.unbind()

    # Each single is one track, so the singles pool uses the full track budget
    # (albums * tracks per album), not just the album count.
    pool_size = track_slots(args)
    if pool_size > 1 and len(singles) > 1:
        lane = PROGRESS.current_lane()
        with ThreadPoolExecutor(max_workers=pool_size) as pool:
            futures = [pool.submit(process_single, i, s, lane)
                       for i, s in enumerate(singles, 1)]
            for future in futures:
                try:
                    future.result()
                except (KeyboardInterrupt, InterruptedError):
                    raise
                except BaseException as exc:
                    log(f'single worker error: {exc}', flush=True)
    else:
        for i, s in enumerate(singles, 1):
            process_single(i, s)

    report()
    log(f'state: {state_path}', flush=True)


if __name__ == '__main__':
    try:
        main()
    except KeyboardInterrupt:
        if DISPLAY:
            DISPLAY.close()
        sys.stderr.write(
            '\nInterrupted. Finished items are saved; rerun to resume.\n')
        sys.exit(130)
    except Exception:
        # Close the pinned display before Python writes the traceback; otherwise
        # absolute cursor moves interleave with stderr and scramble the output.
        if DISPLAY:
            DISPLAY.close()
        raise
