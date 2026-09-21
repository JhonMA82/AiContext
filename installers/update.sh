#!/usr/bin/env sh
# Remote updater for aicontext (POSIX sh).
# Read-only with --check; mutating updates require --yes (mirrors
# `aicontext self update --yes`). Self-contained so it can run via
# `curl ... | sh -s -- --yes` without a prior local checkout.
set -eu

APP="aicontext"
REPO="JhonMA82/AiContext"
VERSION="latest"
PREFIX="${AICONTEXT_PREFIX:-$HOME/.local}"
BIN_DIR="${AICONTEXT_INSTALL_DIR:-$PREFIX/bin}"
CHECK_ONLY=0
YES=0
USE_CARGO_FALLBACK=1

log() { printf '%s\n' "$*"; }
err() { printf '%s\n' "$*" >&2; }

usage() {
  cat <<USAGE
Usage: update.sh [--check] [--version VERSION] [--prefix DIR] [--bin-dir DIR] [--yes] [--no-cargo-fallback] [--help]

Updates $APP remotely (idempotent).

  --check             Compare versions only; never change anything.
  --version VERSION   Target version (default: latest). Accepts "latest",
                      "0.4.0" or "v0.4.0".
  --prefix DIR        User prefix (default: \$HOME/.local).
  --bin-dir DIR       Binary directory (default: \$PREFIX/bin).
  --yes               Required to actually update (refused without it).
  --no-cargo-fallback Refuse the cargo fallback (fail instead).
  --help              Show this help.

Remote usage:
  curl -LsSf https://raw.githubusercontent.com/$REPO/v0.5.0/installers/update.sh | sh -s -- --check
  curl -LsSf https://raw.githubusercontent.com/$REPO/v0.5.0/installers/update.sh | sh -s -- --yes
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --check) CHECK_ONLY=1; shift ;;
    --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
    --version=*) VERSION="${1#--version=}"; shift ;;
    --prefix) PREFIX="${2:?--prefix needs a value}"; BIN_DIR="$PREFIX/bin"; shift 2 ;;
    --prefix=*) PREFIX="${1#--prefix=}"; BIN_DIR="$PREFIX/bin"; shift ;;
    --bin-dir) BIN_DIR="${2:?--bin-dir needs a value}"; shift 2 ;;
    --bin-dir=*) BIN_DIR="${1#--bin-dir=}"; shift ;;
    --yes) YES=1; shift ;;
    --no-cargo-fallback) USE_CARGO_FALLBACK=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) err "unknown argument: $1 (see --help)"; exit 2 ;;
  esac
done

if [ "$VERSION" != "latest" ]; then
  VERSION="$(printf '%s' "$VERSION" | sed 's/^v//')"
fi

installer_url() {
  if [ "$VERSION" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download/%s-installer.sh' "$REPO" "$APP"
  else
    printf 'https://github.com/%s/releases/download/v%s/%s-installer.sh' "$REPO" "$VERSION" "$APP"
  fi
}

# Read-only path: delegate to the installed binary when available so the
# version comparison stays single-sourced in self_update.rs.
do_check() {
  if command -v "$APP" >/dev/null 2>&1; then
    exec "$APP" self update --check
  fi
  if [ -x "$BIN_DIR/$APP" ]; then
    exec "$BIN_DIR/$APP" self update --check
  fi
  log "$APP is not installed; an update would install version: $VERSION"
  log "remote installer: $(installer_url)"
}

do_update() {
  if [ "$YES" -ne 1 ]; then
    err "refusing to update without --yes"
    err "remediation: update.sh --yes"
    exit 2
  fi
  # Prefer the in-place managed path when it can run without network
  # surprises; fall back to the remote installer below.
  if command -v "$APP" >/dev/null 2>&1; then
    if "$APP" self update --yes; then
      return 0
    fi
    log "managed update did not complete; continuing with the remote installer"
  elif [ -x "$BIN_DIR/$APP" ]; then
    if "$BIN_DIR/$APP" self update --yes; then
      return 0
    fi
    log "managed update did not complete; continuing with the remote installer"
  fi
  if ! command -v curl >/dev/null 2>&1; then
    err "missing required tool: curl"
    exit 3
  fi
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' EXIT INT TERM
  url="$(installer_url)"
  log "downloading official installer: $url"
  if curl --proto '=https' --tlsv1.2 -LsSf --max-time 60 "$url" -o "$tmp"; then
    mkdir -p "$BIN_DIR"
    # Same base-dir contract as install.sh: the official installer appends
    # `bin` itself, so it receives the parent of BIN_DIR.
    AICONTEXT_INSTALL_DIR="$(dirname "$BIN_DIR")" sh "$tmp"
    rm -f "$tmp"
    trap - EXIT INT TERM
    log "updated $APP into $BIN_DIR"
    return 0
  fi
  rm -f "$tmp"
  trap - EXIT INT TERM
  if [ "$USE_CARGO_FALLBACK" -eq 1 ] && command -v cargo >/dev/null 2>&1; then
    if [ "$VERSION" = "latest" ]; then
      log "falling back to: cargo install --git https://github.com/$REPO --locked"
      exec cargo install --git "https://github.com/$REPO" --locked
    else
      log "falling back to: cargo install --git https://github.com/$REPO --tag v$VERSION --locked"
      exec cargo install --git "https://github.com/$REPO" --tag "v$VERSION" --locked
    fi
  fi
  err "update failed: official installer unreachable and no cargo fallback available"
  exit 5
}

if [ "$CHECK_ONLY" -eq 1 ]; then
  do_check
else
  do_update
fi
