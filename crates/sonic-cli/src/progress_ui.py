#!/usr/bin/env python3
"""Inline pacman-style progress display for get-catalog.

The block (one row per in-flight download, plus a TOTAL row) is painted where
the cursor is and grows downward as downloads start, like pacman. It is erased
and repainted with relative cursor movement, so nothing is pinned to the bottom
of the terminal and there are no empty reserved rows.

Log lines are written straight to normal scrollback above the block, then the
block is repainted underneath them. Rows are truncated to the terminal width
so a line can never wrap and break the cursor-up arithmetic.

PROGRESS is a router: each worker thread binds its own Lane, so callers keep
using PROGRESS.task_begin(...) without passing a display around.
"""
import atexit
import shutil
import sys
import threading
import time
import uuid


def human(n):
    n = float(n or 0)
    for unit in ('B', 'K', 'M', 'G'):
        if n < 1024 or unit == 'G':
            return f'{int(n)}B' if unit == 'B' else f'{n:.1f}{unit}'
        n /= 1024
    return f'{n:.1f}G'


def bar(pct, width):
    filled = int(max(0.0, min(1.0, pct)) * width + 0.5)
    return '█' * filled + '░' * (width - filled)


def fit(text, width):
    if width <= 0:
        return ''
    return str(text)[:width].ljust(width)


def clock(since):
    elapsed = int(time.monotonic() - since)
    h, rem = divmod(elapsed, 3600)
    m, s = divmod(rem, 60)
    return f'{h:d}:{m:02d}:{s:02d}' if h else f'{m:02d}:{s:02d}'


class Lane:
    """Progress state for one artist, including all in-flight downloads."""

    def __init__(self, display, label='', compact=False, max_tasks=1):
        self.display = display
        self.label = str(label or '')
        self.compact = compact
        self.max_tasks = max(1, int(max_tasks or 1))
        self.items = {}
        self.albums_done = self.singles_done = 0
        self.current = ''
        self.done_weight = 0.0
        self.tasks = {}
        self.started = time.monotonic()
        self._seq = 0

    # ---- queue accounting -------------------------------------------------
    def set_queue(self, album_titles, single_titles):
        self.items = {}
        for title in album_titles:
            self.items[title] = ('albums', 45 * 1024 * 1024, False)
        for title in single_titles:
            self.items[title] = ('singles', 5 * 1024 * 1024, False)
        self.display.draw()

    def set_weight(self, key, weight):
        rec = self.items.get(key)
        if rec:
            self.items[key] = (rec[0], max(0.0, float(weight or 0)), rec[2])
        self.display.draw()

    def set_current(self, text):
        self.current = str(text)
        self.display.draw()

    def complete(self, key, kind):
        rec = self.items.get(key)
        if rec and not rec[2]:
            self.items[key] = (rec[0], rec[1], True)
            self.done_weight += rec[1]
        if kind == 'albums':
            self.albums_done += 1
        else:
            self.singles_done += 1
        self.display.draw()

    # ---- in-flight downloads ----------------------------------------------
    def task_begin(self, total_bytes=0, file_totals=None, key=None, label=''):
        key = key or uuid.uuid4().hex
        self._seq += 1
        self.tasks[key] = {
            'seq': self._seq,
            'bytes': 0,
            'total': max(0, int(total_bytes or 0)),
            'file_totals': [int(x) for x in (file_totals or [])],
            'file_sizes': [],
            'file_index': 0,
            'label': str(label or ''),
            'speed': 0.0,
            'last_bytes': 0,
            'last_sample': time.monotonic(),
        }
        self.display.draw()
        return key

    def task_progress(self, value, files=None, key=None):
        task = self.tasks.get(key)
        if task is None:
            if len(self.tasks) == 1:
                task = next(iter(self.tasks.values()))
            else:
                return
        now = time.monotonic()
        value = max(0, int(value or 0))
        dt = now - task['last_sample']
        if dt >= 0.2:
            task['speed'] = max(0.0, (value - task['last_bytes']) / dt)
            task['last_bytes'], task['last_sample'] = value, now
        task['bytes'] = value
        task['file_sizes'] = [int(x) for x in (files or [])]
        if task['file_sizes']:
            task['file_index'] = max(
                range(len(task['file_sizes'])), key=task['file_sizes'].__getitem__)
        else:
            task['file_index'] = 0
        self.display.draw()

    def task_end(self, key=None):
        if key is None:
            self.tasks.clear()
        else:
            self.tasks.pop(key, None)
        self.display.draw()

    def message(self, text):
        self.display.message(text, self)

    # ---- accounting / rendering -------------------------------------------
    def totals(self):
        done = sum(1 for rec in self.items.values() if rec[2])
        total = len(self.items) or (self.albums_done + self.singles_done)
        weight = sum(rec[1] for rec in self.items.values())
        used = self.done_weight
        for task in self.tasks.values():
            used += (min(task['bytes'], task['total'])
                     if task['total'] else task['bytes'])
        speed = sum(t['speed'] for t in self.tasks.values())
        return done, total, used, weight, speed

    def _task_line(self, number, task, width):
        prefix = (f'[{self.label}] ' if self.display.show_labels else '')
        label = task['label'] or self.current or 'download'
        done, total = task['bytes'], task['total']
        if total:
            pct = min(1.0, done / total)
            size = f'{human(done)}/{human(total)}'
            pct_text = f'{pct:3.0%}'
        elif done:
            pct, size, pct_text = 0.0, human(done), '  ? '
        else:
            pct, size, pct_text = 0.0, '0B', '  0%'
        line = (f'{prefix}{number:2d}  {label}  {size} '
                f'[{bar(pct, 10)}] {pct_text}')
        return fit(line, width)

    def task_rows(self, max_rows):
        width = shutil.get_terminal_size((100, 24)).columns
        tasks = sorted(self.tasks.values(), key=lambda t: t['seq'])
        rows = []
        if len(tasks) > max_rows:
            hidden = len(tasks) - max_rows + 1
            rows.append(fit(f'     +{hidden} earlier', width))
            tasks = tasks[-(max_rows - 1):]
        for number, task in enumerate(tasks, 1):
            rows.append(self._task_line(number, task, width))
        return rows

    def summary_line(self):
        width = shutil.get_terminal_size((100, 24)).columns
        done, total, used, weight, speed = self.totals()
        pct = (used / weight) if weight else 0.0
        albums_total = (self.albums_done +
                        sum(1 for r in self.items.values()
                            if r[0] == 'albums' and not r[2]))
        singles_total = (self.singles_done +
                         sum(1 for r in self.items.values()
                             if r[0] == 'singles' and not r[2]))
        line = (f'[{self.label}] {done}/{total} '
                f'A {self.albums_done}/{albums_total} '
                f'S {self.singles_done}/{singles_total} '
                f'[{bar(pct, 8)}] {pct:3.0%}')
        return fit(line, width)


class Display:
    """Owns the terminal: all lanes are painted and logged under one lock."""

    def __init__(self, labels, enabled=False, compact=None, slots=None):
        self.enabled = bool(enabled) and bool(labels)
        labels = list(labels or [])
        if slots is None:
            slots = [1] * len(labels)
        elif isinstance(slots, int):
            slots = [slots] * len(labels)
        else:
            slots = list(slots) + [1] * (len(labels) - len(slots))
        self.lanes = [Lane(self, label, compact, slots[i])
                      for i, label in enumerate(labels)]
        self.show_labels = len(self.lanes) > 1
        self.lock = threading.RLock()
        self._lines = 0
        self._rendered = []
        self._closed = False
        self._paused = False
        self.started = time.monotonic()
        self._stop = threading.Event()
        if self.enabled:
            sys.stdout.write('\x1b[?25l')
            sys.stdout.flush()
            atexit.register(self.close)
            threading.Thread(target=self._ticker, daemon=True).start()
            self.draw()

    def lane(self, label):
        for lane in self.lanes:
            if lane.label == label:
                return lane
        return None

    def _ticker(self):
        while not self._stop.wait(0.25):
            self.draw()

    def _erase_seq(self):
        if not self._lines:
            return ''
        return '\x1b[1A' * (self._lines - 1) + '\r\x1b[J'

    def _paint_seq(self):
        width = shutil.get_terminal_size((100, 24)).columns
        self._rendered = [line[:max(1, width - 1)] for line in self._layout()]
        self._lines = len(self._rendered)
        return '\n'.join(self._rendered)

    def _layout(self):
        lines = []
        for lane in self.lanes:
            lines.extend(lane.task_rows(lane.max_tasks))
            if self.show_labels:
                lines.append(lane.summary_line())
        lines.append(self._total_line(
            shutil.get_terminal_size((100, 24)).columns))
        return lines

    def _total_line(self, width):
        done = total = used = weight = 0.0
        speed = 0.0
        for lane in self.lanes:
            d, t, u, w, s = lane.totals()
            done += d
            total += t
            used += u
            weight += w
            speed += s
        pct = (used / weight) if weight else 0.0
        speed_text = f' {human(speed)}/s' if speed >= 1024 else ''
        line = (f'TOTAL {done:.0f}/{total:.0f} files  '
                f'{human(used)}/{human(weight)}  '
                f'[{bar(pct, 18)}] {pct:5.1%}  '
                f'{clock(self.started)}{speed_text}')
        return fit(line, width)

    def suspend(self):
        """Temporarily yield the terminal to another full-screen prompt."""
        if not self.enabled:
            return
        with self.lock:
            self._paused = True
            sys.stdout.write(self._erase_seq() + '\x1b[?25h')
            sys.stdout.flush()
            self._lines = 0
            self._rendered = []

    def resume(self):
        """Take the terminal back after a prompt has finished."""
        if not self.enabled:
            return
        with self.lock:
            self._paused = False
            sys.stdout.write('\x1b[?25l')
            sys.stdout.flush()
        self.draw()

    def draw(self, move=True):
        if not self.enabled or not self.lanes or self._paused:
            return
        with self.lock:
            width = shutil.get_terminal_size((100, 24)).columns
            lines = [line[:max(1, width - 1)] for line in self._layout()]
            if len(lines) != self._lines:
                output = self._erase_seq() + '\n'.join(lines)
                self._rendered = lines
                self._lines = len(lines)
            else:
                output = ''
                bottom = self._lines - 1
                for index, (old, new) in enumerate(zip(self._rendered, lines)):
                    if old == new:
                        continue
                    offset = bottom - index
                    if offset:
                        output += f'\x1b[{offset}A'
                    output += '\r\x1b[2K' + new
                    if offset:
                        output += f'\x1b[{offset}B'
                self._rendered = lines
            if not output:
                return
            sys.stdout.write(output)
            sys.stdout.flush()

    def message(self, text, lane=None):
        prefix = (f'[{lane.label}] ' if lane and self.show_labels else '')
        lines = str(text).splitlines() or ['']
        if prefix:
            lines = [prefix + line for line in lines]
        if not self.enabled:
            for line in lines:
                print(line, flush=True)
            return
        with self.lock:
            # Log lines become permanent scrollback; the block is repainted below.
            sys.stdout.write(self._erase_seq() + '\n'.join(lines) + '\n'
                             + self._paint_seq())
            sys.stdout.flush()

    def close(self):
        if self._closed:
            return
        self._closed = True
        self._stop.set()
        if not self.enabled:
            return
        with self.lock:
            width = shutil.get_terminal_size((100, 24)).columns
            # Wipe the live block, then leave the final TOTAL line in scrollback.
            sys.stdout.write(self._erase_seq()
                             + self._total_line(width).rstrip() + '\n'
                             + '\x1b[?25h')
            sys.stdout.flush()
            self.enabled = False


class _ProgressRouter:
    """Dispatch progress calls to the calling thread's bound Lane."""

    def __init__(self):
        self.local = threading.local()

    def bind(self, lane):
        self.local.lane = lane

    def unbind(self):
        if hasattr(self.local, 'lane'):
            del self.local.lane

    def current_lane(self):
        return getattr(self.local, 'lane', None)

    def __getattr__(self, name):
        lane = getattr(self.local, 'lane', None)
        if lane is not None:
            return getattr(lane, name)
        if name == 'message':
            return lambda text='', **_kw: print(text, flush=True)
        return lambda *_a, **_kw: None


PROGRESS = _ProgressRouter()
