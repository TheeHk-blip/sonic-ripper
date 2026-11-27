# YouTube Music batch helpers

All discovery is done on `music.youtube.com` only, so URLs are exact YouTube
Music track/album IDs. Downloads use `sonic-cli --ytdlp-clients web_music` and
pass `--binaries-dir $SONIC_TOOLS_DIR` so they always use the bundled sidecars
in the tools directory.

## Configuration (environment variables)

No path is hardcoded. Every script reads its defaults from `ytm_config.py`,
and each default can be overridden with an environment variable (a leading `~`
is expanded). Command-line flags such as `--cli`, `--ytdlp`, `--cookies` and
`--root` still win over the environment.

| Variable               | Default                                  | Purpose                                                                                |
| ---------------------- | ---------------------------------------- | -------------------------------------------------------------------------------------- |
| `SONIC_TOOLS_DIR`      | `/path/to/tools`                         | Folder holding `sonic-cli` and `sonic-yt-dlp`                                          |
| `SONIC_CLI`            | `$SONIC_TOOLS_DIR/sonic-cli`             | The `sonic-cli` binary                                                                 |
| `SONIC_YTDLP`          | `$SONIC_TOOLS_DIR/sonic-yt-dlp`          | yt-dlp used for discovery/validation                                                   |
| `YTM_COOKIE_FILE`      | `/path/to/music.youtube.com_cookies.txt` | Netscape cookies for YouTube Music                                                     |
| `SONIC_MUSIC_ROOT`     | `/path/to/music/library`                 | Library folder that holds the artist folders (set this explicitly)                     |
| `SONIC_DATA_DIR`       | `$XDG_DATA_HOME/sonic-ripper-cli`        | Base folder for state and logs                                                         |
| `SONIC_STATE_DIR`      | `$SONIC_DATA_DIR/ytm-batch-state`        | Per-artist state, index and CLI configs                                                |
| `SONIC_LOG_DIR`        | `$SONIC_DATA_DIR/ytm-batch-logs`         | Logs for `--detach` runs                                                               |
| `SONIC_INFO_CACHE_DIR` | OS cache dir `/yt-info`                  | Shared yt-dlp metadata cache between CLI processes (7-day TTL)                         |
| `YTM_CLIENT_VERSION`   | `v*`                                     | WEB_REMIX client build sent to InnerTube; bump it if requests start returning HTTP 400 |
| `YTM_HL` / `YTM_GL`    | `en-GB` / `KE`                           | InnerTube language and region                                                          |

Full yt-dlp metadata lookups are shared between CLI processes through a
7-day disk cache keyed by video id plus a stable cookie/client fingerprint.
Volatile session cookies are ignored so normal cookie refresh does not invalidate
it. Delete the directory (or point `SONIC_INFO_CACHE_DIR` at a fresh one) to
force new lookups.

Example (put these in your shell profile):

```bash
export SONIC_MUSIC_ROOT=/path/to/music/library
export YTM_COOKIE_FILE=/path/to/music.youtube.com_cookies.txt
export SONIC_TOOLS_DIR=/path/to/tools
```

Run `python3 ytm_config.py` to print the paths currently in effect.

## Incremental all-in-one runner (recommended)

Only the artist name is required. Wikipedia supplies a reference list of
albums, mixtapes, EPs and singles; the YTM artist index decides what actually
exists. A Wikipedia title is downloaded only if the same release is in the
index, and wiki-only titles are skipped without a search. Wikipedia must match
the exact requested artist name; a similar name such as `Rah Digga` is never
accepted for `Digga D`. If Wikipedia has no list, is unreachable, or has zero
exact YTM matches, the runner falls back to every release in the YTM index so
`--interactive` can still show a list to select from. It finds
the artist folder in the library (or creates one), saves state after every
item, and skips anything already downloaded on the next run.

```bash
get-catalog --artist 'Marvin Gaye' --detach
```

If an artist's YouTube Music Topic channel lists singles that the artist page
does not, pass the channel URL once. Its release tiles and Top songs are merged
into the same exact-ID index, and the channel id is cached for later refreshes:

```bash
get-catalog --artist 'digga' --refresh-index --select \
  --ytm-channel 'https://music.youtube.com/channel/UC...'
```

Handle URLs and bare handles work too. They are resolved to a `UC...` channel
id once with yt-dlp, then the canonical id is cached:

```bash
get-catalog --artist 'dave' --refresh-index --select \
  --ytm-channel 'https://music.youtube.com/@Artist'
# equivalent: --ytm-channel '@Artist'
```

`get-catalog` is a launcher for `ytm-artist-batch.py` on your PATH
(for example `/path/to/bin/get-catalog`), so all the same arguments work. An optional
`--` separator is accepted, e.g. `get-catalog --artist 'Sade' -- --status-only`.

- Without `--detach`, progress is drawn inline on a terminal (not a pipe), like
  pacman: the block starts at the cursor and grows downward as downloads start,
  using relative cursor movement.
  - One row per in-flight download:
    `N  label  downloaded/total [bar] %`. Each download has its own size and
    bracket bar. A row appears when its download starts and disappears when it
    finishes, so the block is only as tall as the active downloads.
  - The `TOTAL` row is the last line of the block and moves down as rows are
    added: `TOTAL n/m files  used/estimated [bar] %  clock speed`. Percent, `█/░`
    bar, elapsed clock and transfer speed are weighted by estimated byte size,
    so the bar tracks real music bytes rather than just queue entries.
  - With `--artists`, every artist runs at the same time in the foreground and
    gets its own download rows plus an `[artist]`-prefixed summary:
    `A x/y S x/y [bar] %`. Log lines carry the same prefix and are printed to scrollback
    above the block, which is then repainted, so wrapped text cannot corrupt
    the bars.

  Downloaded bytes come from the actual bytes `sonic-cli` streams into its
  `sonic-download-*/audio.webm.part` files under the run's temporary directory
  (the system temp dir, or the library's hidden `.sonic-tmp` during catalog
  runs). The byte monitor follows the `TMPDIR` of the marked
  child processes, so those external-drive runs update their rows too.
  Per-download and global totals come from the playlist's flat durations at
  about 300 kbit/s (no extra per-track metadata calls); skipped album/EP tracks
  carry zero weight. Log lines go to scrollback above the block and a 0.25 s
  ticker keeps the bars, clock and speed moving even when no new log line
  appears. Ctrl-C stops every artist's child download tree, removes its partial
  stream files, restores the cursor, and exits quietly with code 130; rerunning
  the same command resumes from saved state.
  An album download timeout is also non-fatal: its partial folder is removed,
  the item is saved as `failed`, and the next album continues; rerun to retry.
  Suppressed for `--detach`, `--status-only`, and redirected output.

- `--detach` runs it in the background; log:
  `$SONIC_LOG_DIR/marvin-gaye.log`
- `--cancel marvin-gaye` stops a detached session by artist name or slug and
  exits. It signals the whole worker process group, so `sonic-cli`, `yt-dlp`
  and `ffmpeg` children stop too. See "Cancelling detached sessions" below.
- `--status-only` prints each item's status plus `ALBUMS x/y done`,
  `SINGLES x/y done`, and `TOTAL x/y done` summaries with a breakdown. It uses
  the same Wikipedia-filtered catalog as a download run; the first call may
  discover and cache the Wikipedia list.
- `--interactive` (alias `--select`) opens a checkbox picker before any
  download starts. It lists every exact YTM release from the index, including
  singles that are also album tracks, so the user can decide what to keep;
  Wikipedia filtering is not applied before the picker. After confirmation,
  flat duplicate tracks are removed from the selected set before the download
  queue is built, so duplicate singles do not run in either foreground or
  detached mode. `Space` toggles the highlighted row, `A` selects all, `N` clears all,
  `L`/`S` toggle all albums/singles, `Enter` confirms, and `q` cancels. After
  confirming, press `F`/Enter for a foreground run or `D` for a detached
  background run. It needs a real terminal and one artist; it cannot be combined
  with `--detach`, `--status-only`, `--retag`, or `--artists`.
- `--root /path/to/artist/folder` overrides the auto-detected artist folder.
- Temporary download and yt-dlp extraction files go in the hidden
  `.sonic-tmp/` directory inside the artist root (including a custom `--root`),
  rather than the system temporary filesystem. Each child process gets an
  isolated subdirectory that is removed on exit; abandoned run directories are
  reaped the next time the artist is processed.
- `--genre SOUL` is used only when a brand-new artist folder must be created.
- `--albums albums.tsv --singles singles.json` overrides the Wikipedia lists.
  TSV format: `YEAR<TAB>ALBUM`, with an optional third column for an exact YTM
  playlist URL/ID: `YEAR<TAB>ALBUM<TAB>https://music.youtube.com/playlist?list=OLAK5uy_...`.
  A pinned URL always wins over search, which is useful when YouTube Music's
  search returns a fake or tribute album.
- `--artists` runs several artists with one command. It accepts a JSON array, a
  comma-separated list, repeated flags, or the bracket form. Each artist is
  isolated by its slug id (state, log, CLI config and yt-dlp config):

  ```bash
  get-catalog --artists 'cleo sol, sade' --bandwidth 500k --detach
  get-catalog --artists '["cleo sol", "sade"]' --bandwidth 500k --detach
  get-catalog --artists['cleo sol', 'sade'] --bandwidth 500k --detach
  ```

  To choose albums per artist, add a parallel `--albums[...]` list. Each inner
  list belongs to the artist at the same position. Titles or 1-based album
  numbers work; `all`/`*` means the full catalog for that artist:

  ```bash
  get-catalog --artists['cleo sol', 'sade'] --albums[[1,2,3], [1,2]] --detach
  get-catalog --artists['cleo sol', 'sade'] --albums[['Rose in the Dark'], ['Diamond Life']] --detach
  ```

  With `--detach`, one background worker starts per artist and writes its own
  log. Without it, all artists run concurrently in one foreground process, each
  with its own lane. Each artist processes up to its assigned `--concurrency`
  share of albums (and, later, singles) at once; multi-artist shares sum to the
  global limit. `--status-only` also performs the checks in parallel
  and prefixes every line with `[artist]`. If you pass `--bandwidth`, that rate
  is the group's combined cap, not a per-artist cap. `--root`, `--state`,
  `--config-dir`, `--albums` and `--singles` cannot be combined with multiple
  artists.

## Cancelling detached sessions

Every detached worker is started in its own session and records a small pidfile
at `$SONIC_STATE_DIR/<artist-slug>.catalog.pid`:

```text
get-catalog --artist 'Strandz' --detach
get-catalog --cancel strandz
```

`--cancel` accepts the displayed name or the slug, so `Strandz`, `strandz` and
`STRANDZ` all match. It checks that the recorded PID is still a live catalog
worker for the same run before signalling the process group with `SIGTERM`,
then escalates to `SIGKILL` only if the group does not stop. A stale pidfile is
ignored and removed. The process title is also set to `cat-<slug>` where the
platform supports it (Linux `prctl`), e.g. `cat-strandz`; this is only a `ps`
convenience, not the control mechanism.

The command also scans live session leaders whose command line contains this
script and the requested artist. That fallback can cancel detached runs started
before pidfiles existed, such as a current run using older code. Foreground
runs are deliberately not matched; use Ctrl-C for those.

A detached worker handles `SIGTERM` like foreground Ctrl-C: it stops its child
tree and removes its active `.sonic-tmp/run-*` and `.download-*` directories
before exiting. `--cancel` also reaps stale `run-*` directories for that
artist and empty `.download-*` folders. A further safety sweep still runs at the
start of the next catalog run.

## Fast artist index (default)

Before downloading, the script mimics a normal YouTube Music session: it
searches the artist, clicks the artist card, reads the artist page, opens the
complete "Singles and EPs" tab, then
the "Albums" tab via its `continuation` token, plus the "Show all" songs list
for older tracks. This replaces ~12–32 s of per-track searching with one
**7–10 s** index build, then resolves almost everything locally.

- Every release tile already contains the exact `videoId`, the `OLAK5uy_`
  `playlistId` and the year in its play button/subtitle. Those IDs are stored directly; no
  extra browse or yt-dlp metadata call is needed. sonic-cli's own enrichment
  fills in the full metadata when the track is downloaded.
- Cached per artist as `<state-dir>/<artist>-ytm-index.json` for 7 days.
  `--refresh-index` rebuilds it; `--index-age N` changes the lifetime in days;
  `--no-artist-index` restores the old per-track search behavior.
- Only the CLI cookie file is used. SAPISID hashes are derived per request;
  no browser-session secrets are stored.
- A track is never accepted unless the credited artist matches. If neither
  the index nor search can verify it, it is reported as `unresolved` instead
  of downloading someone else's cover.
- Duplicate tracks are owned flatly across album/EP releases; standalone
  singles never own an album track. Every member row is matched by exact video
  id and normalized title; the earliest album/EP by year keeps it, and
  duplicate rows are removed from later releases in the cached index file.
  Selected release member lists are refreshed from YTM before dedupe, so a
  playlist edit after the index was cached is seen. When a later album is
  downloaded, those exact video ids are passed to `sonic-cli --exclude-track=...`
  and filtered out before metadata enrichment only when the owner is also
  selected or already on disk. Tracks already present in another album/EP
  folder are excluded too. If only a later album was selected and the owner is
  not downloaded, that album keeps the song for this run. A single is skipped
  only when the song is in a selected or downloaded album/EP or already present
  in a library album folder.

## Curation, routing and metadata

Wikipedia is a reference list, not a source of downloads: an indexed YTM
release is queued as an album/EP or single only when Wikipedia lists it. The
non-single list includes studio albums, mixtapes and EPs, because YTM labels
those differently. Titles that exist only on Wikipedia are skipped, and the
per-item search resolver is never used for them. Dub/riddim/version releases
are instrumental beats, and
anniversary, remastered or re-recorded editions are repeats; both are skipped.
Deluxe editions are kept, since they often contain the only indexed copy of
the original album. If Wikipedia has no album or single list, cannot be matched
to the artist, or matches no indexed release, that category falls back to the
YTM index list. Live albums are not auto-excluded;
use `--interactive` to untick them. YTM supplies exact IDs, the
`Single`/`EP` subtitle label, the explicit `E` badge, and album/EP membership.
Explicit `--albums` / `--singles` files and `--select-albums` override the
Wikipedia reference but are still filtered against the index.
`--no-artist-index` opts out and restores Wikipedia discovery plus the
per-item search resolver.

- Release type routes the download:

  ```text
  ARTIST/YYYY - Title/          album
  ARTIST/EP/YYYY - Title/       EP
  ARTIST/SINGLES/               single
  ```

- For every Wikipedia-approved album plus every YTM-labelled EP, the exact
  member track list is fetched once and cached in the index. The index is
  scanned **before the download queue is built**, so a single that is really an
  album/EP track (or already sits in an album folder on disk) is removed before
  any worker starts. The download-time check is kept as a second line of
  defence and records `skipped-album-track` / `skipped-ep-track`. YTM sometimes
  gives the standalone single a different video ID from the copy inside the
  album/EP, so title matching is used as well as exact IDs.
- The `artist` tag is left to sonic-core's YTM enrichment, which keeps only
  performers as one comma-separated value (for example `Drake, Central Cee`).
  Writer/producer/composer/DJ credits and legal-name-style credits such as
  `A. Graham` are dropped. The requested artist is passed as a protected hint,
  and album downloads also protect the album artist, so neither can be filtered
  out. No `ALBUMARTIST` tag is added; folder and single filenames still use the
  requested artist.
- Every downloaded file gets `RELEASE_TYPE`, `MUSICBRAINZ_ALBUMTYPE`,
  `EXPLICIT` (`1`/`0`) and `ITUNESADVISORY` (`1`/`2`) tags. The explicit flag is
  keyed by exact video ID, so a clean and an explicit version of the same title
  remain distinguishable; when both exist the filename gains `(Clean)` or
  `(Explicit)`. Tags are passed to `sonic-cli` as `--tag` / `--track-tag` and
  written by sonic-core at download time, so Opus cover art is preserved.
  `--retag` uses sonic-core's `--tag-file` mode for the same reason.
- `--retag` is a one-pass backfill that writes those tags to files already in
  the library without downloading anything:

  ```bash
  get-catalog --artist 'Zach Bryan' --retag
  ```

- `--songs` bypasses the album/EP membership skip, which is useful when you
  explicitly want both versions of a track.
- `--songs` downloads only the named tracks and skips the album catalog. Accepts
  a JSON array, a comma-separated list, or repeated flags:

  ```bash
  get-catalog --artist 'Cleo Sol' --songs '["Butterfly", "One"]' --detach
  get-catalog --artist 'Cleo Sol' --songs 'Butterfly, One' --detach
  get-catalog --artist 'Cleo Sol' --songs Butterfly --songs One --detach
  ```

- `--select-albums` (`--albums[]` is also accepted) downloads only the named
  albums and skips singles. It accepts a JSON array, comma-separated titles, or
  repeated flags, and matches titles, `YEAR - TITLE`, or pinned `OLAK5uy_` IDs:

  ```bash
  get-catalog --artist 'Marvin Gaye' --select-albums '["Moods of Marvin Gaye", "M.P.G."]' --detach
  get-catalog --artist 'Marvin Gaye' --select-albums "1971 - What's Going On" --detach
  ```

- `--format`, `--bitrate`, `--sample-rate`, and `--video-quality` are passed to
  `sonic-cli`:

  ```bash
  get-catalog --artist 'Sade' --format mp3 --bitrate 320k --detach
  get-catalog --artist 'Sade' --format mp4 --video-quality 1080p --detach
  ```

- `--cli-args '...'` forwards any other `sonic-cli` flags, e.g.
  `--cli-args '--concurrency 2 --no-id3'`.
- Three forms control parallelism; all are capped at 16 in-flight downloads so
  raising the track count can never multiply albums into a process explosion:

  ```bash
  # 1 album at a time, 6 tracks downloading inside it
  get-catalog --artist 'Sade' --concurrency 6

  # 6 albums at a time, 1 track each (the default)
  get-catalog --artist 'Sade' --album-concurrency 6

  # 2 albums at once, 3 tracks each = 6 downloads
  get-catalog --artist 'Sade' --album-concurrency 2 --track-concurrency 3
  ```

  `--concurrency N` (alias `--global-concurrency N`) and `--track-concurrency N`
  alone are track-only requests: one album with `N` tracks. `--album-c N` alone
  is album-only: `N` albums with one track each. Pairing `--album-concurrency A`
  with `--track-concurrency T` runs `A * T` downloads. With `--artists`, album
  counts are split evenly (remainder to earlier artists); for track-only
  requests the track budget is split instead, with one album per artist:

  ```bash
  get-catalog --artists['cleo sol', 'sade'] --album-concurrency 6 --detach
  # -> 3 albums per artist, 1 track each = 6 downloads

  get-catalog --artists['cleo sol', 'sade'] --concurrency 6 --detach
  # -> 1 album per artist, 3 tracks each = 6 downloads

  get-catalog --album-concurrency 4 --track-concurrency 2 --bandwidth 500K
  # -> 8 downloads sharing the 500K cap
  ```

  Limits: `1 <= A <= 16`, `1 <= T <= 16`, `A * T <= 16`, and the chosen album
  count (or track count for a track-only request) must be at least the number
  of artists so nobody gets zero slots. An explicit `--concurrency` inside
  `--cli-args` is a track-only request (1 album * N tracks) and logs a note, so
  it can no longer multiply silently. Rebalancing happens between items, not in
  the middle of a file, because yt-dlp reads its settings when each process
  starts.

  The full track budget (`A * T`) also sizes the parallel singles pool. A
  `watch?v=` URL is one `sonic-cli` process, so the worker runs several of them
  at once; the album batch flag does not apply to that single-track path. Each
  temporary download uses its own hidden `.download-<uuid>` folder, then the
  finished file is moved into `SINGLES`.

- `--bandwidth 500K` (alias `--limit-rate 500K`) is a **total** cap, like
  qBittorrent's global speed:

  ```bash
  get-catalog --artist 'Sade' --bandwidth 500K --detach
  get-catalog --artists 'cleo sol, sade' --bandwidth 500K --detach
  ```

  With a single artist and no explicit concurrency, that artist gets the whole
  500K with one download at a time. With `--artists`, the parent writes a shared
  state file under
  `<state-dir>/global-<RATE>-c<CONCURRENCY>-<ids>/bandwidth.json` and each worker
  writes its current share to its own `<artist>-ytdlp-config/yt-dlp/config`.
  The cap is divided by the number of _active slots_ (workers × their
  concurrency), so 6 slots get ~83K each; if a worker finishes, the remaining
  slots get a bigger share on their next item. In-flight singles use a depth
  counter, so one single finishing does not mark the worker inactive while its
  siblings are still downloading. Each child gets its own `XDG_CONFIG_HOME`,
  keeping the parallel workers' yt-dlp configs isolated.

  An explicit `--concurrency` in `--cli-args` is still respected as a per-worker
  value, in which case the cap applies per download as before:

  ```bash
  get-catalog --artist 'Sade' --bandwidth 500K --cli-args '--concurrency 2' --detach
  # 500K per download, ~1M aggregate
  ```

- `get-catalog --help` opens the full grouped reference with all options and examples.

State defaults to
`$SONIC_STATE_DIR/<artist>.json` (see Configuration above).

Each artist gets its own `sonic-cli` config directory
(`$SONIC_STATE_DIR/<artist>-cli-config`). Catalog downloads pass every setting
to `sonic-cli` in memory, so parallel workers cannot overwrite each other's
`settings.json` or redirect a download to another folder. Concurrent runs for
different artists are therefore safe.

## 1. Find album playlists

`albums.tsv` format: `YEAR<TAB>ALBUM TITLE` per line, optionally followed by
`<TAB>PLAYLIST_URL` to pin an exact YouTube Music album.

```bash
/path/to/tools/ytm-find-albums.py \
  --artist 'Marvin Gaye' \
  --albums albums.tsv \
  --output candidates.json
```

It searches both `ARTIST ALBUM YEAR` and `ARTIST ALBUM`, then prints
`OLAK5uy_...` playlist IDs.

## 2. Validate candidates

```bash
/path/to/tools/ytm-validate-albums.py \
  --candidates candidates.json \
  --output validated.json
```

Prints track count and first-track `album` / `date` / `artist`. Pick the
candidate whose metadata matches the intended original album, then write a
map file: `{"Album Title": "OLAK5uy_..."}`.

## 3. Download albums

```bash
/path/to/tools/ytm-download-albums.py \
  --map album-map.json \
  --root "$SONIC_MUSIC_ROOT/GENRE/ARTIST" \
  --tracks-output album-tracks.json
```

Creates `<year> - <album>` folders and exports all track titles for dedup.

## 4. Resolve singles

`singles.json`: `[{"title": "...", "year": "..."}, ...]`

```bash
/path/to/tools/ytm-resolve-singles.py \
  --artist 'Marvin Gaye' \
  --singles singles.json \
  --output resolved-singles.json
```

Emits exact `https://music.youtube.com/watch?v=...` URLs with verified artist
metadata. Pass `--album-tracks album-tracks.json` to skip album tracks here.

## 5. Download singles

```bash
/path/to/tools/ytm-download-singles.py \
  --artist 'Marvin Gaye' \
  --resolved resolved-singles.json \
  --root "$SONIC_MUSIC_ROOT/SOUL/ARTIST" \
  --album-tracks album-tracks.json
```

Downloads to `SINGLES/`, patches wrong tags in-place (cover art preserved), and
skips singles already present on downloaded albums.

## Tag patcher

```bash
/path/to/tools/opus_settags.py input.opus output.opus 'artist=ARTIST' 'title=TITLE'
```
