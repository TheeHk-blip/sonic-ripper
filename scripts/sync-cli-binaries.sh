#!/usr/bin/env bash
# Copy the yt-dlp / ffmpeg / deno sidecars next to the CLI binary so the CLI
# can use the exact same tools as the desktop app.
set -euo pipefail

profile="${1:-debug}"
root="$(cd "$(dirname "$0")/.." && pwd)"
triple="$(rustc -vV | sed -n 's/^host: //p')"
exe=""
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) exe=".exe" ;;
esac

dest="$root/target/$profile"
mkdir -p "$dest"

for tool in sonic-yt-dlp sonic-ffmpeg sonic-deno; do
  src=""
  for candidate in \
    "$root/src-tauri/target/$profile/$tool$exe" \
    "$root/src-tauri/binaries/$tool-$triple$exe"; do
    if [[ -f "$candidate" ]]; then
      src="$candidate"
      break
    fi
  done

  if [[ -z "$src" ]]; then
    echo "Could not find $tool$exe (looked in src-tauri/target/$profile and src-tauri/binaries)" >&2
    exit 1
  fi

  cp "$src" "$dest/$tool$exe"
  chmod +x "$dest/$tool$exe" 2>/dev/null || true
  echo "Installed $tool$exe -> $dest"
done
