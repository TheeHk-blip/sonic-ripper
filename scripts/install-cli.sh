#!/usr/bin/env bash
# Install sonic-cli, its sidecars, and the ytm-* Python helpers for the current user.
set -euo pipefail

REPO="${SONIC_REPO:-TheeHk-blip/sonic-ripper}"
VERSION="${SONIC_VERSION:-v1.0.7}"
SIDECAR_TAG="${SONIC_SIDECAR_TAG:-sidecar-binaries-v1}"
TOOLS_DIR="${SONIC_TOOLS_DIR:-$HOME/.sonic-cli}"
BIN_DIR="${SONIC_BIN_DIR:-$HOME/.local/bin}"

PY_SCRIPTS=(
  ytm-artist-batch.py
  ytm_config.py
  ytm_index.py
  ytm-find-albums.py
  ytm-validate-albums.py
  ytm-download-albums.py
  ytm-resolve-singles.py
  ytm-download-singles.py
  wiki_discovery.py
  progress_ui.py
  checkbox_select.py
  opus_settags.py
  opus_retag.py
  normalize_ampersand_tags.py
)

log() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die 'curl is required (install curl and retry)'

if command -v python3 >/dev/null 2>&1; then
  PYTHON=python3
elif command -v python >/dev/null 2>&1; then
  PYTHON=python
else
  die 'python3 is required for get-catalog'
fi

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Linux)
      case "$arch" in
        x86_64 | amd64) printf 'x86_64-unknown-linux-gnu' ;;
        aarch64 | arm64) printf 'aarch64-unknown-linux-gnu' ;;
        *) die "unsupported Linux architecture: $arch" ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) printf 'x86_64-apple-darwin' ;;
        arm64) printf 'aarch64-apple-darwin' ;;
        *) die "unsupported macOS architecture: $arch" ;;
      esac
      ;;
    *) die "unsupported OS: $os (use install-cli.ps1 on Windows)" ;;
  esac
}

TARGET="$(detect_target)"

download() {
  local url="$1" dest="$2" tmp
  tmp="$(mktemp "${dest}.XXXXXX")"
  if ! curl -fsSL --retry 3 --proto '=https' --tlsv1.2 -o "$tmp" "$url"; then
    rm -f "$tmp"
    return 1
  fi
  mv "$tmp" "$dest"
}

raw_url() {
  printf 'https://raw.githubusercontent.com/%s/%s/crates/sonic-cli/src/%s' \
    "$REPO" "$VERSION" "$1"
}

release_url() {
  printf 'https://github.com/%s/releases/download/%s/%s' "$REPO" "$1" "$2"
}

ensure_path() {
  local line shell_name profile
  shell_name="$(basename "${SHELL:-bash}")"
  case "$shell_name" in
    fish)
      profile="$HOME/.config/fish/config.fish"
      line="fish_add_path $BIN_DIR"
      mkdir -p "$(dirname "$profile")"
      if ! grep -Fq "$line" "$profile" 2>/dev/null; then
        printf '\n%s\n' "$line" >>"$profile"
      fi
      ;;
    zsh)
      profile="$HOME/.zshrc"
      line="export PATH=\"$BIN_DIR:\$PATH\""
      touch "$profile"
      if ! grep -Fq "$BIN_DIR" "$profile"; then
        printf '\n%s\n' "$line" >>"$profile"
      fi
      ;;
    *)
      profile="$HOME/.bashrc"
      line="export PATH=\"$BIN_DIR:\$PATH\""
      touch "$profile"
      if ! grep -Fq "$BIN_DIR" "$profile"; then
        printf '\n%s\n' "$line" >>"$profile"
      fi
      ;;
  esac
  printf '%s' "$profile"
}

log "Installing Sonic CLI tools to $TOOLS_DIR"
mkdir -p "$TOOLS_DIR" "$BIN_DIR"

log "Downloading Python helpers from $VERSION"
for script in "${PY_SCRIPTS[@]}"; do
  download "$(raw_url "$script")" "$TOOLS_DIR/$script"
done

log "Downloading sonic-cli for $TARGET"
if ! download "$(release_url "$VERSION" "sonic-cli-$TARGET")" "$TOOLS_DIR/sonic-cli"; then
  die "no prebuilt sonic-cli asset for $TARGET in release $VERSION"
fi

for tool in sonic-yt-dlp sonic-ffmpeg sonic-deno; do
  log "Downloading $tool for $TARGET"
  download "$(release_url "$SIDECAR_TAG" "$tool-$TARGET")" "$TOOLS_DIR/$tool"
done

chmod +x "$TOOLS_DIR/sonic-cli" "$TOOLS_DIR/sonic-yt-dlp" \
  "$TOOLS_DIR/sonic-ffmpeg" "$TOOLS_DIR/sonic-deno"

cat >"$BIN_DIR/get-catalog" <<EOF
#!/usr/bin/env sh
set -e
: "\${SONIC_TOOLS_DIR:=\$HOME/.sonic-cli}"
: "\${SONIC_BINARIES_DIR:=\$SONIC_TOOLS_DIR}"
export SONIC_TOOLS_DIR SONIC_BINARIES_DIR

if command -v python3 >/dev/null 2>&1; then
  PYTHON=python3
elif command -v python >/dev/null 2>&1; then
  PYTHON=python
else
  echo "get-catalog: python3 is required" >&2
  exit 1
fi

exec "\$PYTHON" "\$SONIC_TOOLS_DIR/ytm-artist-batch.py" "\$@"
EOF

cat >"$BIN_DIR/sonic-cli" <<EOF
#!/usr/bin/env sh
set -e
: "\${SONIC_TOOLS_DIR:=\$HOME/.sonic-cli}"
export SONIC_TOOLS_DIR
exec "\$SONIC_TOOLS_DIR/sonic-cli" "\$@"
EOF

chmod +x "$BIN_DIR/get-catalog" "$BIN_DIR/sonic-cli"

PROFILE="$(ensure_path)"
export PATH="$BIN_DIR:$PATH"

log "Installed get-catalog and sonic-cli into $BIN_DIR"
printf '  shell profile updated: %s\n' "$PROFILE"
printf '  tools directory:       %s\n' "$TOOLS_DIR"
printf '\nOpen a new terminal, then set your library/cookies if you have not already:\n'
printf '  export SONIC_MUSIC_ROOT="$HOME/Music"\n'
printf '  export YTM_COOKIE_FILE="$HOME/music.youtube.com_cookies.txt"\n'
printf '\nTry: get-catalog --artist "Sade" --status-only\n'
