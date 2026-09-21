#!/usr/bin/env sh
# Remote installer for aicontext (POSIX sh).
# Primary source: official cargo-dist installer published on GitHub Releases.
# Fallback: cargo install from the git repository (the crate is not
# published on crates.io; requires a Rust toolchain).
# Only writes inside the user prefix; no root needed; never touches
# project manifests or .engineering/ dirs.
set -eu

APP="aicontext"
REPO="JhonMA82/AiContext"
VERSION="latest"
PREFIX="${AICONTEXT_PREFIX:-$HOME/.local}"
BIN_DIR="${AICONTEXT_INSTALL_DIR:-$PREFIX/bin}"
USE_CARGO_FALLBACK=1

log() { printf '%s\n' "$*"; }
err() { printf '%s\n' "$*" >&2; }

usage() {
  cat <<USAGE
Usage: install.sh [--version VERSION] [--prefix DIR] [--bin-dir DIR] [--yes] [--no-cargo-fallback] [--help]

Installs $APP remotely (idempotent, safe to re-run).

  --version VERSION   Version to install (default: latest).
                      Accepts "latest", "0.4.0" or "v0.4.0".
  --prefix DIR        User prefix (default: \$HOME/.local).
  --bin-dir DIR       Binary directory (default: \$PREFIX/bin).
  --yes               Accepted for automation compat; install never prompts.
  --no-cargo-fallback Refuse the cargo fallback (fail instead).
  --help              Show this help.

Remote usage:
  curl -LsSf https://raw.githubusercontent.com/$REPO/main/installers/install.sh | sh
  curl -LsSf https://raw.githubusercontent.com/$REPO/main/installers/install.sh | sh -s -- --version 0.4.0

Env overrides: AICONTEXT_PREFIX, AICONTEXT_INSTALL_DIR.
USAGE
}

# Parse args (works when piped via `sh -s -- <args>`).
while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
    --version=*) VERSION="${1#--version=}"; shift ;;
    --prefix) PREFIX="${2:?--prefix needs a value}"; BIN_DIR="$PREFIX/bin"; shift 2 ;;
    --prefix=*) PREFIX="${1#--prefix=}"; BIN_DIR="$PREFIX/bin"; shift ;;
    --bin-dir) BIN_DIR="${2:?--bin-dir needs a value}"; shift 2 ;;
    --bin-dir=*) BIN_DIR="${1#--bin-dir=}"; shift ;;
    --yes) shift ;;
    --no-cargo-fallback) USE_CARGO_FALLBACK=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) err "unknown argument: $1 (see --help)"; exit 2 ;;
  esac
done

# Normalize version: strip a leading 'v', keep 'latest' as-is.
if [ "$VERSION" != "latest" ]; then
  VERSION="$(printf '%s' "$VERSION" | sed 's/^v//')"
fi

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    err "missing required tool: $1"
    return 1
  fi
}

installer_url() {
  if [ "$VERSION" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download/%s-installer.sh' "$REPO" "$APP"
  else
    printf 'https://github.com/%s/releases/download/v%s/%s-installer.sh' "$REPO" "$VERSION" "$APP"
  fi
}

run_official_installer() {
  need_cmd curl || return 1
  tmp="$(mktemp)"
  trap 'rm -f "$tmp"' EXIT INT TERM
  url="$(installer_url)"
  log "downloading official installer: $url"
  if curl --proto '=https' --tlsv1.2 -LsSf --max-time 60 "$url" -o "$tmp"; then
    log "running official installer into: $BIN_DIR"
    mkdir -p "$BIN_DIR"
    # cargo-dist installers honor <APP>_INSTALL_DIR (here AICONTEXT_INSTALL_DIR).
    AICONTEXT_INSTALL_DIR="$BIN_DIR" sh "$tmp"
    rm -f "$tmp"
    trap - EXIT INT TERM
    return 0
  fi
  rm -f "$tmp"
  trap - EXIT INT TERM
  return 1
}

run_cargo_fallback() {
  if [ "$USE_CARGO_FALLBACK" -ne 1 ]; then
    return 1
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    return 1
  fi
  if [ "$VERSION" = "latest" ]; then
    log "installer download failed; falling back to: cargo install --git https://github.com/$REPO --locked"
    cargo install --git "https://github.com/$REPO" --locked
  else
    log "installer download failed; falling back to: cargo install --git https://github.com/$REPO --tag v$VERSION --locked"
    cargo install --git "https://github.com/$REPO" --tag "v$VERSION" --locked
  fi
}

verify_install() {
  if [ -x "$BIN_DIR/$APP" ]; then
    "$BIN_DIR/$APP" --version 2>/dev/null || true
    return 0
  fi
  if command -v "$APP" >/dev/null 2>&1; then
    "$APP" --version 2>/dev/null || true
    return 0
  fi
  return 1
}

main() {
  if run_official_installer; then
    log "installed $APP into $BIN_DIR"
  elif run_cargo_fallback; then
    log "installed $APP via cargo"
  else
    err "install failed: could not download the official installer"
    err "try again later, or (with a Rust toolchain) run: cargo install --git https://github.com/$REPO --locked"
    exit 5
  fi
  if verify_install; then
    case ":$PATH:" in
      *":$BIN_DIR:"*) ;;
      *) log "hint: add $BIN_DIR to PATH to use $APP everywhere" ;;
    esac
    log "done. Next: $APP status"
  else
    err "warning: installer ran but no $APP binary was found on PATH or in $BIN_DIR"
    exit 5
  fi
}

main
