# Sonic Ripper

**Sonic Ripper** is a desktop application for extracting audio and video from Spotify and YouTube links. Built with Tauri + React, it lets you discover tracks from playlists or individual URLs, configure output settings, and download them as high-quality audio files or ZIP archives.

## Table of Contents

- [Key Features](#key-features)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Development](#development)
- [Build](#build)
- [License](#license)

## Key Features

| Feature                    | Description                                                                                                                                 |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| **Source Analysis**        | Paste Spotify or YouTube URLs to parse tracks/playlists. Detects playlist vs. single tracks automatically.                                  |
| **Format & Quality**       | Choose from FLAC, MP3, M4A, WAV, OPUS, or MP4. Select bitrate (128k–320k or lossless). For video, choose resolution up to 1080p.            |
| **ID3 Tagging**            | Embed metadata (title, artist, album, year, track number) and high-res album artwork directly into audio files.                             |
| **Naming Patterns**        | Three patterns: `Artist - Title`, `01 - Artist - Title`, or `Title Only`.                                                                   |
| **Download Folder**        | Save tracks directly to a chosen directory instead of ZIP. Supports batch downloading with concurrency.                                     |
| **Bot Detection Bypass**   | Extract cookies from your browser profile (Chrome, Firefox, Safari, Edge, Brave, Opera, Vivaldi) or manually paste Netscape-format cookies. |
| **Virtualized Track List** | Smooth scrolling with per-row height measurement for thousands of tracks.                                                                   |
| **Built-in Media Player**  | Inline YouTube preview within the app.                                                                                                      |
| **Cross-Platform**         | Packaged as AppImage (Linux), with Windows and macOS builds supported.                                                                      |

## Installation

### Prerequisites

- [Node.js](https://nodejs.org/) (>= 20 recommended)
- [pnpm](https://pnpm.io/) (package manager)
- [Rust](https://rust-lang.org/) (for Tauri backend, via `rustup`)
- Tauri CLI: `pnpm tauri`

### Clone & Setup

```bash
git clone https://github.com/your-username/sonic-ripper.git
cd sonic-ripper
pnpm install
```

The project embeds `yt-dlp` and `ffmpeg` binaries for the target platform (see `src-tauri/binaries/`).

### Development Mode

```bash
pnpm dev
```

Runs Vite in development mode with Tauri. App runs at `http://localhost:1420` by default.

## Usage

### 1. Add a Source

- Paste a Spotify track/playlist URL or YouTube URL into the input field.
- Click **Search** or press Enter.
- If a playlist is detected, you'll see the track count and can configure settings.
- If a single track, you'll go straight to the configure step.

### 2. Configure

- **Output Format** – Select FLAC (lossless) or a compressed format (MP3, M4A, OPUS, WAV, MP4).
- **Bitrate/Quality** – For lossy formats: 128k–320k. For lossless: no bitrate setting.
- **Sample Rate** – 44.1 kHz (CD Quality) or 48 kHz (Studio Quality), only for lossless formats.
- **Video Quality** – If selecting MP4: choose from 360p to Max Resolution (best).
- **Naming Pattern** – How filenames are structured.
- **ID3 Tags** – Toggle embedding metadata and artwork.
- **Download Folder** – Choose a directory for direct folder saves (required for folder mode).
- **Bot Bypass** – Select your browser profile or manually paste cookies.

### 3. Download

- **Save as ZIP** – Downloads all tracks as a single `.zip` archive with proper folder structure and metadata.
- **Save to Folder** – Downloads tracks directly to your chosen download folder, one file per track.

### 4. Media Player

- Click the play icon on any track row to open the inline YouTube preview player at the bottom of the window.
- Expand/collapse the video component using the TV icon (YouTube tracks only).

## Configuration

All settings are stored in the Tauri backend and persist between sessions. The default configuration:

| Setting              | Default               | Description                          |
| -------------------- | --------------------- | ------------------------------------ |
| `format`             | `flac`                | Output audio format                  |
| `bitrate`            | `lossless`            | Audio bitrate (ignored for FLAC/WAV) |
| `saveInFolder`       | `true`                | Save to folder vs. ZIP               |
| `skipMissingTracks`  | `true`                | Skip failed tracks in batch          |
| `namingPattern`      | `number_artist_title` | File naming scheme                   |
| `embedId3Tags`       | `true`                | Embed ID3 metadata                   |
| `youtubeCookies`     | _(empty)_             | Manual Netscape cookie text          |
| `cookiesFromBrowser` | _(empty)_             | Browser profile extraction mode      |
| `sampleRate`         | `44100`               | Lossless sample rate                 |
| `videoQuality`       | _(undefined)_         | Video resolution (MP4 only)          |
| `downloadFolder`     | `null`                | Path to selected download folder     |

### YouTube Bot Detection Bypass

Sonic Ripper uses `yt-dlp` which may encounter bot checks on YouTube. Two options are provided:

1. **Browser Profile** – Select your browser from the dropdown. The app will extract session cookies automatically.
2. **Manual Paste** – Paste the contents of your `cookies.txt` (Netscape format) into the textarea. The file should contain lines like:

```
# Netscape HTTP Cookie File
.youtube.com	TRUE	/	TRUE	1791234567	__Secure-3PSID	AIzaSy...
```

**Tip:** If using browser extraction, close the browser first — it may lock its cookie database while running, blocking extraction.

## Development

### Project Structure

```
src/
├── App.tsx           # Main React component with step wizard (source → configure → results)
├── main.tsx          # Entry point
├── App.css           # Tailwind base + custom CSS variables (theme colors, fonts)
├── types.ts          # TypeScript interfaces (Track, DownloadSettings, AudioFormat, etc.)
├── components/       # UI components
│   ├── MediaPlayer.tsx      # Inline YouTube player
│   ├── SettingsPanel.tsx    # Configuration panel
│   ├── StepProgress.tsx     # Step wizard progress indicator
│   ├── TrackListHeader.tsx  # Track count display
│   └── TrackRow.tsx         # Individual track row with progress
├── lib/
│   ├── api.ts           # Tauri RPC wrappers (analyze, download, settings)
│   └── useVirtualizer.tsx   # Virtualized list with dynamic row heights
└── lib/                 (other lib files)
src-tauri/
├── tauri.conf.json    # Tauri config (window size, CSP, binaries, targets)
├── Cargo.toml         # Rust dependencies
├── build.rs           # Tauri build script
├── gen/               # Generated code
├── capabilities/      # Feature capabilities
└── src/               # Rust source code
```

### Key Development Commands

| Command        | Description                              |
| -------------- | ---------------------------------------- |
| `pnpm dev`     | Start development server + Tauri         |
| `pnpm build`   | Build the frontend (Vite + TSC)          |
| `pnpm tauri`   | Run Tauri commands (build, dev, preview) |
| `pnpm preview` | Preview the built Vite app               |

### Adding New Features

- **New UI component** – Add under `src/components/` and import in `App.tsx`.
- **New Tauri command** – Add function in `src-tauri/src/` (Rust), declare in `src-tauri/api.ts` (TypeScript), then invoke from the React app using `invoke`.
- **New binary** – Place in `src-tauri/binaries/` and reference in `tauri.conf.json` under `bundle.externalBin`.

## Build

### Build Frontend Only

```bash
pnpm build
```

Output goes to `dist/`.

### Build Tauri Application

```bash
pnpm tauri build
```

This compiles the Rust backend and packages the frontend. The output binary will be in `src-tauri/target/release/bundle/`.

### Target Platforms

The project is configured to build **AppImage** for Linux (as specified in `tauri.conf.json`). To build for other platforms, modify the `bundle.targets` field in `tauri.conf.json`:

```json
"bundle": {
  "active": true,
  "targets": ["appimage", "deb", "winget", "msi"]
}
```

## License

MIT

## Acknowledgments

- [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) for video/audio extraction
- [`ffmpeg`](https://ffmpeg.org/) for format conversion and tagging
- [Tauri](https://tauri.app/) for the native bridge
- [React](https://react.dev/) and [Vite](https://vitejs.dev/) for the frontend
- [TailwindCSS](https://tailwindcss.com/) for styling
- [lucide-react](https://lucide.dev/) for icons
