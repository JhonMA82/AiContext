#!/usr/bin/env sh
# Remote uninstaller for aicontext (POSIX sh).
# Removes only AIContext-owned state; foreign files, project manifests and
# .engineering/ dirs are reported and left untouched. Requires --yes
# (mirrors `aicontext self uninstall --managed --yes`).
set -eu

APP="aicontext"
PREFIX="${AICONTEXT_PREFIX:-$HOME/.local}"
BIN_DIR="${AICONTEXT_INSTALL_DIR:-$PREFIX/bin}"
MANAGED_PREFIX="$HOME/.local/share/aicontext"
MANAGED=0
YES=0

log() { printf '%s\n' "$*"; }
err() { printf '%s\n' "$*" >&2; }

usage() {
  cat <<USAGE
Usage: uninstall.sh [--managed] [--prefix DIR] [--bin-dir DIR] [--yes] [--help]

Removes only $APP-owned state (never foreign files).

  --managed     Also remove managed-tools owned by AIContext
                (\$HOME/.local/share/aicontext entries inside the prefix).
  --prefix DIR  User prefix used at install time (default: \$HOME/.local).
  --bin-dir DIR Binary directory used at install time (default: \$PREFIX/bin).
  --yes         Required to actually remove anything.
  --help        Show this help.

Remote usage:
  curl -LsSf https://raw.githubusercontent.com/JhonMA82/AiContext/v0.7.0/installers/uninstall.sh | sh -s -- --managed --yes

Never touched: foreign files, project manifests, .engineering/ dirs.
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --managed) MANAGED=1; shift ;;
    --prefix) PREFIX="${2:?--prefix needs a value}"; BIN_DIR="$PREFIX/bin"; shift 2 ;;
    --prefix=*) PREFIX="${1#--prefix=}"; BIN_DIR="$PREFIX/bin"; shift ;;
    --bin-dir) BIN_DIR="${2:?--bin-dir needs a value}"; shift 2 ;;
    --bin-dir=*) BIN_DIR="${1#--bin-dir=}"; shift ;;
    --yes) YES=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) err "unknown argument: $1 (see --help)"; exit 2 ;;
  esac
done

if [ "$YES" -ne 1 ]; then
  err "refusing to uninstall without --yes"
  err "remediation: uninstall.sh --managed --yes"
  exit 2
fi

# A path is removable only when it lives inside one of the owned prefixes.
inside_owned() {
  path="$1"
  case "$path" in
    "$HOME/.local/"*|"$HOME/.cargo/"*|"$PREFIX/"*) return 0 ;;
    *) return 1 ;;
  esac
}

remove_file_owned() {
  path="$1"
  label="$2"
  if [ ! -e "$path" ]; then
    log "absent: $label"
    return 0
  fi
  if inside_owned "$path"; then
    rm -f "$path"
    log "removed $label"
  else
    log "left alone: $label (outside managed prefix; left untouched)"
  fi
}

remove_binary() {
  # Candidates in priority order; deduplicated by exact path. The trailing
  # legacy entries cover installs made by early 0.5.0 wrappers, which passed
  # the bin dir itself as the install base and nested `<dir>/bin/bin`.
  seen=""
  for candidate in "$BIN_DIR/$APP" "$HOME/.cargo/bin/$APP" "$HOME/.local/bin/$APP" "$MANAGED_PREFIX/bin/$APP" "$BIN_DIR/bin/$APP" "$PREFIX/bin/bin/$APP" "$HOME/.local/bin/bin/$APP"; do
    case ":$seen:" in
      *":$candidate:"*) continue ;;
    esac
    seen="$seen:$candidate"
    remove_file_owned "$candidate" "binary at $candidate"
  done
  # Anything else currently on PATH is foreign by definition: report only.
  if command -v "$APP" >/dev/null 2>&1; then
    found="$(command -v "$APP")"
    case ":$seen:" in
      *":$found:"*) ;;
      *) log "left alone: binary at $found (outside managed prefix; left untouched)" ;;
    esac
  fi
}

registry_paths() {
  registry="$1"
  if [ ! -f "$registry" ]; then
    return 0
  fi
  if command -v jq >/dev/null 2>&1; then
    jq -r '.tools[]?.path // empty' "$registry" 2>/dev/null || true
  else
    sed -n 's/.*"path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$registry"
  fi
}

remove_managed_tools() {
  registry="$MANAGED_PREFIX/managed-tools.json"
  if [ "$MANAGED" -ne 1 ]; then
    log "left alone: managed tools prefix (pass --managed to remove owned tools)"
    return 0
  fi
  if [ ! -f "$registry" ]; then
    log "left alone: no ownership registry found"
    return 0
  fi
  registry_paths "$registry" | while IFS= read -r tool_path; do
    [ -n "$tool_path" ] || continue
    remove_file_owned "$tool_path" "managed tool at $tool_path"
  done
  # The registry itself lives inside the managed prefix by construction.
  rm -f "$registry"
  log "removed ownership registry at $registry"
}

remove_owned_skill() {
  # Same agent list as `agent::SUPPORTED_AGENTS`: pi keeps its historical
  # tree, opencode follows its documented global skills directory.
  skill_found=0
  while IFS= read -r skill_dir; do
    [ -n "$skill_dir" ] || continue
    manifest="$skill_dir/.aicontext-managed.json"
    if [ -f "$manifest" ]; then
      skill_found=1
      rm -f "$skill_dir/SKILL.md" "$manifest"
      log "removed owned skill file $skill_dir/SKILL.md"
      log "removed ownership manifest $manifest"
      if rmdir "$skill_dir" 2>/dev/null; then
        log "removed skill dir $skill_dir"
      else
        log "left alone: skill dir $skill_dir (foreign files remain; left untouched)"
      fi
    elif [ -f "$skill_dir/SKILL.md" ]; then
      skill_found=1
      log "left alone: skill dir exists without an AIContext ownership manifest; left untouched"
    fi
  done <<EOF
$HOME/.pi/agent/skills/aicontext-adopt
${XDG_CONFIG_HOME:-$HOME/.config}/opencode/skills/aicontext-adopt
EOF
  if [ "$skill_found" -eq 0 ]; then
    log "left alone: no owned agent skills found"
  fi
}

remove_binary
remove_managed_tools
remove_owned_skill
log "only AIContext-owned state was touched; foreign files, project manifests and .engineering/ dirs were left alone."
