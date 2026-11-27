"""Shared paths and defaults for the ytm-* scripts and get-catalog.

Every value can be overridden with an environment variable, so nothing here is
tied to one user or machine:

  SONIC_TOOLS_DIR    folder holding sonic-cli and sonic-yt-dlp   (/path/to/tools)
  SONIC_CLI          path to the sonic-cli binary                ($SONIC_TOOLS_DIR/sonic-cli)
  SONIC_YTDLP        path to the sonic-yt-dlp binary             ($SONIC_TOOLS_DIR/sonic-yt-dlp)
  YTM_COOKIE_FILE    Netscape cookies.txt for music.youtube.com  (/path/to/cookies.txt)
  SONIC_DATA_DIR     base folder for state and logs              ($XDG_DATA_HOME/sonic-ripper-cli)
  SONIC_STATE_DIR    per-artist state, index and CLI configs     ($SONIC_DATA_DIR/ytm-batch-state)
  SONIC_LOG_DIR      logs for --detach runs                      ($SONIC_DATA_DIR/ytm-batch-logs)
  SONIC_INFO_CACHE_DIR shared yt-dlp metadata cache               (OS cache dir/yt-info)
  SONIC_MUSIC_ROOT   library folder that holds the artist folders (see music_root())
  YTM_CLIENT_VERSION WEB_REMIX client version sent to InnerTube  (see below)
  YTM_HL / YTM_GL    InnerTube language / region                 (en-GB / KE)

"~" is expanded in every path variable.
"""
import os
from pathlib import Path


def _path(env, default):
    value = os.environ.get(env, '').strip()
    return Path(value).expanduser() if value else Path(default)


HOME = Path.home()
_XDG_DATA = _path('XDG_DATA_HOME', HOME / '.local' / 'share')

TOOLS_DIR = _path('SONIC_TOOLS_DIR', HOME / '.sonic-cli')
CLI = _path('SONIC_CLI', TOOLS_DIR / 'sonic-cli')
YTDLP = _path('SONIC_YTDLP', TOOLS_DIR / 'sonic-yt-dlp')
COOKIES = _path('YTM_COOKIE_FILE', HOME / 'music.youtube.com_cookies.txt')

DATA_DIR = _path('SONIC_DATA_DIR', _XDG_DATA / 'sonic-ripper-cli')
STATE_DIR = _path('SONIC_STATE_DIR', DATA_DIR / 'ytm-batch-state')
LOG_DIR = _path('SONIC_LOG_DIR', DATA_DIR / 'ytm-batch-logs')

CLIENT_VERSION = os.environ.get('YTM_CLIENT_VERSION', '1.20261122.17.00')
HL = os.environ.get('YTM_HL', 'en-GB')
GL = os.environ.get('YTM_GL', 'KE')

def music_root():
    """Library folder. Looked up on every call because the drive may mount late.

    $SONIC_MUSIC_ROOT wins; otherwise the platform's default Music folder is
    used. Set the variable explicitly for a library on another drive.
    """
    value = os.environ.get('SONIC_MUSIC_ROOT', '').strip()
    if value:
        return Path(value).expanduser()
    return HOME / 'Music'


if __name__ == '__main__':
    for name in ('TOOLS_DIR', 'CLI', 'YTDLP', 'COOKIES', 'DATA_DIR',
                 'STATE_DIR', 'LOG_DIR'):
        print(f'{name:<11} {globals()[name]}')
    print(f'{"MUSIC_ROOT":<11} {music_root()}')
    print(f'{"CLIENT":<11} {CLIENT_VERSION}  hl={HL} gl={GL}')
