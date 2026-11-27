#!/usr/bin/env python3
"""Small terminal checkbox picker used by get-catalog --interactive.

No third-party dependency: it uses termios for single-key input and plain ANSI
cursor movement for repainting. The state machine is kept separate from the
terminal so it can be tested without a TTY.
"""
import shutil
import sys

try:
    import termios
except ImportError:  # pragma: no cover - get-catalog is Linux/macOS only
    termios = None


class SelectionCancelled(Exception):
    """The user quit the picker; nothing should be downloaded."""


class Choice:
    """One selectable row.

    ``group`` enables group toggles (e.g. L for albums, S for singles);
    ``value`` is whatever the caller wants returned for a checked row.
    """

    def __init__(self, label, value=None, group='', selected=True):
        self.label = str(label)
        self.value = value
        self.group = str(group or '')
        self.selected = bool(selected)

    def __repr__(self):
        return f'Choice({self.label!r}, selected={self.selected})'


class SelectionState:
    """Pure state machine for the checkbox UI."""

    def __init__(self, choices, all_selected=True, title='Select items'):
        self.choices = list(choices)
        for choice in self.choices:
            choice.selected = bool(all_selected)
        self.index = 0 if self.choices else -1
        self.title = title

    @property
    def selected(self):
        return [c for c in self.choices if c.selected]

    @property
    def count(self):
        return len(self.selected)

    def clamp(self):
        if not self.choices:
            self.index = -1
        else:
            self.index = max(0, min(self.index, len(self.choices) - 1))

    def move(self, delta):
        if self.choices:
            self.index = (self.index + delta) % len(self.choices)

    def home(self):
        self.index = 0 if self.choices else -1

    def end(self):
        self.index = len(self.choices) - 1 if self.choices else -1

    def toggle(self):
        if self.index >= 0:
            choice = self.choices[self.index]
            choice.selected = not choice.selected

    def select_all(self, value=True):
        for choice in self.choices:
            choice.selected = bool(value)

    def clear_all(self):
        self.select_all(False)

    def toggle_group(self, group):
        group = (group or '').casefold()
        for choice in self.choices:
            if choice.group.casefold() == group:
                choice.selected = not choice.selected

    def handle(self, key):
        """Apply a normalized key; return 'confirm', 'cancel', or None."""
        if key in ('up', 'k'):
            self.move(-1)
        elif key in ('down', 'j'):
            self.move(1)
        elif key == 'home':
            self.home()
        elif key == 'end':
            self.end()
        elif key in ('space', 'toggle'):
            self.toggle()
        elif key in ('a', 'A'):
            self.select_all(True)
        elif key in ('n', 'N'):
            self.clear_all()
        elif key in ('l', 'L'):
            self.toggle_group('album')
        elif key in ('s', 'S'):
            self.toggle_group('single')
        elif key in ('enter', 'confirm'):
            return 'confirm'
        elif key in ('cancel', 'q', 'Q'):
            return 'cancel'
        return None


ARROW_KEYS = {
    'A': 'up', 'B': 'down', 'H': 'home', 'F': 'end',
}


def read_key(stream):
    """Read one keypress, decoding arrows and control keys."""
    char = stream.read(1)
    if not char:
        return 'cancel'
    if char == '\x1b':
        marker = stream.read(1)
        if marker == '[':
            code = stream.read(1)
            return ARROW_KEYS.get(code, '')
        return 'cancel'
    if char in ('\r', '\n'):
        return 'enter'
    if char == ' ':
        return 'space'
    if char in ('\x03', '\x04'):  # Ctrl-C, Ctrl-D
        return 'cancel'
    return char


def fit(text, width):
    if width <= 0:
        return ''
    return str(text)[:max(1, width - 1)].ljust(max(1, width - 1))


def render(state, width=None, height=None):
    """Build the visible lines, with a viewport around the cursor."""
    size = shutil.get_terminal_size((100, 24))
    width = width or size.columns
    height = height or size.lines
    header = [
        fit(state.title, width),
        fit(f'{state.count}/{len(state.choices)} selected', width),
        fit('up/down move  space toggle  A all  N none  L albums  S singles',
            width),
        fit('Enter start download  q cancel', width),
    ]
    rows = max(3, height - len(header) - 1)
    total = len(state.choices)
    if total:
        start = max(0, min(state.index - rows // 2, total - rows))
        end = min(total, start + rows)
    else:
        start = end = 0
    lines = list(header)
    for pos in range(start, end):
        choice = state.choices[pos]
        mark = '✓' if choice.selected else ' '
        pointer = '❯' if pos == state.index else ' '
        lines.append(fit(f'{pointer} [{mark}] {choice.label}', width))
    if start:
        lines.append(fit(f'... {start} above', width))
    if end < total:
        lines.append(fit(f'... {total - end} below', width))
    return lines


def paint(output, lines, previous):
    """Repaint in place; ``previous`` is the line count last painted."""
    if previous:
        output.write(f'\x1b[{previous - 1}A')
    output.write('\r\x1b[J' + '\n'.join(lines))
    output.flush()


def choose_run_mode(input_stream=None, output_stream=None, prompt=None):
    """Ask where the confirmed selection should run.

    Enter/F means foreground, D means detached, q cancels. This uses normal
    line input because select_items() has already restored the terminal.
    """
    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    prompt = prompt or '\nRun mode: [F]oreground (default), [D]etached, [q] cancel: '
    while True:
        output_stream.write(prompt)
        output_stream.flush()
        line = input_stream.readline()
        if not line:
            raise SelectionCancelled()
        answer = line.strip().casefold()
        if answer in ('', 'f', 'fg', 'foreground', 'run'):
            return 'foreground'
        if answer in ('d', 'bg', 'detach', 'detached', 'background'):
            return 'detached'
        if answer in ('q', 'quit', 'cancel'):
            raise SelectionCancelled()
        output_stream.write('Please enter F, D, or q.\n')
        output_stream.flush()


def select_items(choices, title='Select items', input_stream=None,
                 output_stream=None):
    """Show a checkbox picker and return the checked choices.

    Raises SelectionCancelled when the user quits. Requires a real terminal;
    batch/detached runs should not call this.
    """
    if not choices:
        return []
    if termios is None:
        raise RuntimeError('interactive selection needs termios')
    input_stream = input_stream or sys.stdin
    output_stream = output_stream or sys.stdout
    if not input_stream.isatty() or not output_stream.isatty():
        raise RuntimeError('interactive selection requires a terminal')

    state = SelectionState(choices, title=title)
    fd = input_stream.fileno()
    saved = termios.tcgetattr(fd)
    settings = termios.tcgetattr(fd)
    settings[3] &= ~(termios.ECHO | termios.ICANON)
    settings[6][termios.VMIN] = 1
    settings[6][termios.VTIME] = 0
    termios.tcsetattr(fd, termios.TCSADRAIN, settings)

    previous = 0
    result = None
    cancelled = False
    output_stream.write('\x1b[?25l')
    try:
        while result is None:
            lines = render(state)
            paint(output_stream, lines, previous)
            previous = len(lines)
            action = state.handle(read_key(input_stream))
            if action == 'confirm':
                result = state.selected
            elif action == 'cancel':
                cancelled = True
                break
    finally:
        if previous:
            output_stream.write(f'\x1b[{previous - 1}A')
        output_stream.write('\r\x1b[J\x1b[?25h')
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)
        if cancelled:
            output_stream.write('Selection cancelled; nothing downloaded.\n')
        else:
            output_stream.write(
                f'Selected {len(result or [])}/{len(choices)} item(s).\n')
        output_stream.flush()
    if cancelled:
        raise SelectionCancelled()
    return result
