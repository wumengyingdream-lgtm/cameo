#!/usr/bin/env bash
# build_dev.sh — kill running instances, build a debug .app, print the path.
#
# Usage:
#   ./build_dev.sh          # kill running + build, then print the path to click
#   ./build_dev.sh -o       # ...and launch it
#   ./build_dev.sh --clean  # cargo clean first (full rebuild)
#
# Output: src-tauri/target/debug/bundle/macos/Cameo.app
# Windows: use build_dev.ps1 instead.

set -euo pipefail

[[ "$(uname -s)" == "Darwin" ]] || { echo "this script is for macOS — on Windows run build_dev.ps1" >&2; exit 1; }

OPEN=0
CLEAN=0
for arg in "$@"; do
  case "$arg" in
    -o|--open) OPEN=1 ;;
    --clean)   CLEAN=1 ;;
    -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown flag: $arg" >&2; exit 1 ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

command -v pnpm  >/dev/null || { echo "✗ pnpm not on PATH"; exit 1; }
command -v cargo >/dev/null || { echo "✗ cargo not on PATH"; exit 1; }

# Load .env so VITE_CAMEO_API_KEY (cloud features) and TAURI_SIGNING_PRIVATE_KEY
# (if someone runs an updater-flagged build) are available. Empty vars are
# unset so codesign/Tauri don't try to use them.
if [[ -f .env ]]; then
  set -a; . ./.env; set +a
  for v in APPLE_SIGNING_IDENTITY APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID TAURI_SIGNING_PRIVATE_KEY; do
    if [[ -z "${!v:-}" ]]; then unset "$v"; fi
  done
  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD-}"
  fi
fi

# ---- kill anything running ------------------------------------------------
echo "→ stopping running Cameo / dev server / codex sidecar"
pkill -f "tauri dev"                 2>/dev/null || true
pkill -f "target/debug/cameo"        2>/dev/null || true
pkill -f "Cameo.app/Contents/MacOS"  2>/dev/null || true
pkill -f "codex app-server"          2>/dev/null || true
lsof -ti:1420 2>/dev/null | xargs kill -9 2>/dev/null || true
sleep 0.5

[[ -d node_modules ]] || { echo "→ pnpm install (first run)"; pnpm install; }

if [[ "$CLEAN" -eq 1 ]]; then
  echo "→ cargo clean + rm dist"
  (cd src-tauri && cargo clean)
  rm -rf dist
fi

start_ts=$(date +%s)

# ---- build debug .app bundle ----------------------------------------------
echo "→ pnpm tauri build --debug --bundles app"
pnpm tauri build --debug --bundles app

bundle="$ROOT/src-tauri/target/debug/bundle/macos/Cameo.app"
[[ -d "$bundle" ]] || { echo "✗ bundle not produced at $bundle" >&2; exit 1; }

echo ""
echo "✓ built in $(( $(date +%s) - start_ts ))s"
echo ""
echo "  ┌─ click to launch ────────────────────────────────────────"
echo "     $bundle"
echo "  └───────────────────────────────────────────────────────────"
echo ""
echo "  or run:  open \"$bundle\""
echo ""

if [[ "$OPEN" -eq 1 ]]; then
  echo "→ launching Cameo.app"
  open "$bundle"
fi
