#!/usr/bin/env bash
# publish_release.sh — sign + upload the macOS auto-update release to R2.
#
# macOS half of the release. Windows publishes independently from a Windows box
# via publish_release.ps1 — the two write DIFFERENT manifest files on R2
# (darwin-*.json here, windows-x86_64.json there), so independent, async uploads
# never clobber each other.
#
# Prerequisite:  ./build_release.sh            (default = arm + Intel, with updater)
#
# Usage:
#   ./publish_release.sh                       # sign + upload mac artifacts
#   ./publish_release.sh --dry-run             # print what would happen, upload nothing
#
# What it does:
#   1. Reads version from src-tauri/tauri.conf.json
#   2. For each arch (mac-arm, mac-intel):
#      - Locates the auto-update payload (.app.tar.gz)
#      - Reads/creates its .sig (signed at build time via TAURI_SIGNING_PRIVATE_KEY)
#      - Writes a manifest JSON ({version, notes, pub_date, signature, url})
#   3. rclone uploads to R2:
#      - .app.tar.gz + .sig + .dmg → release/v<version>/
#      - manifests → update/{darwin-aarch64,darwin-x86_64}.json
#
# R2 credentials come from .env (R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY /
# R2_ENDPOINT / R2_BUCKET). The same .env that hosts TAURI_SIGNING_PRIVATE_KEY
# also hosts these — re-uses the gallery upload creds.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"

info() { printf '→ %s\n' "$*"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*"; }
die()  { printf '  \033[31m✗\033[0m %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "macOS only — on Windows run publish_release.ps1"

DRY_RUN=0
for arg in "$@"; do
  case "$arg" in
    --dry-run)   DRY_RUN=1 ;;
    -h|--help)   sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown flag: $arg" ;;
  esac
done

# ── load .env ────────────────────────────────────────────────────────────────
[[ -f .env ]] || die ".env not found — needed for R2 creds and signing key"
info "loading .env"
set -a; . ./.env; set +a

for v in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ENDPOINT R2_BUCKET; do
  [[ -n "${!v:-}" ]] || die "$v missing from .env"
done

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  warn "TAURI_SIGNING_PRIVATE_KEY not set — manifests will reference unsigned payloads (clients will refuse them)"
fi

command -v rclone >/dev/null || die "rclone not on PATH — brew install rclone"

# ── version ─────────────────────────────────────────────────────────────────
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
[[ -n "$VERSION" ]] || die "could not read version from tauri.conf.json"
ok "version: $VERSION"
PUB_DATE=$(date -u +%Y-%m-%dT%H:%M:%SZ)
NOTES="Cameo v${VERSION}"
DOWNLOAD_BASE="https://r.cameo.ink/release/v${VERSION}"

# ── locate artifacts ────────────────────────────────────────────────────────
TARGET_DIR="${ROOT}/src-tauri/target"

# Manifests are written into a temp dir then uploaded.
MANIFEST_DIR=$(mktemp -d)
trap 'rm -rf "$MANIFEST_DIR"' EXIT

# Track files we'll upload. Each entry: <local_path>|<r2_dest_path>
declare -a UPLOADS=()

queue() {
  UPLOADS+=("$1|$2")
}

# Sign a payload if .sig is missing. Best-effort — failure leaves SIG="".
ensure_sig() {
  local payload="$1"
  local sig="${payload}.sig"
  if [[ -f "$sig" ]]; then
    echo "$sig"
    return
  fi
  if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    return
  fi
  info "  signing $(basename "$payload")"
  local keyfile
  keyfile=$(mktemp)
  chmod 600 "$keyfile"
  echo "$TAURI_SIGNING_PRIVATE_KEY" > "$keyfile"
  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
    pnpm tauri signer sign -k "$keyfile" -p "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" "$payload" >/dev/null
  else
    pnpm tauri signer sign -k "$keyfile" "$payload" >/dev/null
  fi
  rm -f "$keyfile"
  if [[ -f "$sig" ]]; then echo "$sig"; fi
}

write_manifest() {
  local manifest_name="$1"  # e.g. darwin-aarch64
  local payload_filename="$2"  # filename on R2, e.g. Cameo_aarch64.app.tar.gz
  local sig_file="$3"  # local path to .sig (or empty)
  local signature=""
  if [[ -n "$sig_file" && -f "$sig_file" ]]; then
    signature=$(cat "$sig_file")
  fi
  local out="${MANIFEST_DIR}/${manifest_name}.json"
  cat > "$out" <<EOF
{
  "version": "${VERSION}",
  "notes": "${NOTES}",
  "pub_date": "${PUB_DATE}",
  "signature": "${signature}",
  "url": "${DOWNLOAD_BASE}/${payload_filename}"
}
EOF
  ok "manifest: ${manifest_name}.json"
  if [[ -z "$signature" ]]; then
    warn "  signature empty — clients will REJECT this update"
  fi
  queue "$out" "update/${manifest_name}.json"
}

# ── macOS scan (arm + intel) ────────────────────────────────────────────────
for arch_pair in "aarch64-apple-darwin:aarch64:darwin-aarch64" "x86_64-apple-darwin:x86_64:darwin-x86_64"; do
  IFS=":" read -r RUST_TARGET ARCH MANIFEST_NAME <<< "$arch_pair"
  BUNDLE="$TARGET_DIR/$RUST_TARGET/release/bundle"
  if [[ ! -d "$BUNDLE" ]]; then
    warn "skipping $RUST_TARGET — no bundle (run ./build_release.sh first)"
    continue
  fi
  info "$RUST_TARGET"

  # .app.tar.gz for the updater, .dmg for manual download (DMG is optional — the
  # updater doesn't need it).
  TAR=$(find "$BUNDLE/macos" -maxdepth 1 -name "*.app.tar.gz" ! -name "*.sig" 2>/dev/null | head -1)
  DMG=$(find "$BUNDLE/dmg" -maxdepth 1 -name "*.dmg" 2>/dev/null | head -1)

  if [[ -z "$TAR" ]]; then
    warn "  no .app.tar.gz for $RUST_TARGET — skipping updater manifest (DMG-only release for this arch)"
  else
    SIG=$(ensure_sig "$TAR" || true)
    # Add arch suffix to the uploaded filename so ARM and Intel don't clash on
    # R2 (Tauri names both "Cameo.app.tar.gz"; we rename on upload).
    base=$(basename "$TAR" .app.tar.gz)
    UPLOAD_NAME="${base}_${ARCH}.app.tar.gz"
    queue "$TAR" "release/v${VERSION}/${UPLOAD_NAME}"
    [[ -n "$SIG" ]] && queue "$SIG" "release/v${VERSION}/${UPLOAD_NAME}.sig"
    write_manifest "$MANIFEST_NAME" "$UPLOAD_NAME" "$SIG"
  fi

  if [[ -n "$DMG" ]]; then
    queue "$DMG" "release/v${VERSION}/$(basename "$DMG")"
    ok "  dmg: $(basename "$DMG")"
  fi
done

# ── upload ──────────────────────────────────────────────────────────────────
if [[ ${#UPLOADS[@]} -eq 0 ]]; then
  die "no artifacts to upload — did you run ./build_release.sh first?"
fi

echo ""
info "preparing to upload ${#UPLOADS[@]} file(s) to R2 bucket '$R2_BUCKET':"
for entry in "${UPLOADS[@]}"; do
  IFS="|" read -r SRC DST <<< "$entry"
  printf '    %s\n         → r2:%s/%s\n' "$(basename "$SRC")" "$R2_BUCKET" "$DST"
done
echo ""

if [[ "$DRY_RUN" -eq 1 ]]; then
  ok "DRY RUN — no upload performed."
  exit 0
fi

read -r -p "Proceed with upload? [y/N] " yn
[[ "$yn" =~ ^[Yy]$ ]] || { warn "aborted by user"; exit 0; }

# Configure rclone's ad-hoc `:s3:` backend via env vars. Do NOT use an inline
# connection string (`:s3,endpoint=…:`) — R2_ENDPOINT contains "://" and rclone
# truncates an unquoted value at the first colon, yielding endpoint="https".
export RCLONE_S3_PROVIDER="Cloudflare"
export RCLONE_S3_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export RCLONE_S3_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export RCLONE_S3_ENDPOINT="$R2_ENDPOINT"
export RCLONE_S3_REGION="auto"
# R2 tokens can't create buckets; skip rclone's pre-upload bucket check/create
# (the bucket already exists) — otherwise the CreateBucket probe 403s.
export RCLONE_S3_NO_CHECK_BUCKET="true"

for entry in "${UPLOADS[@]}"; do
  IFS="|" read -r SRC DST <<< "$entry"
  info "rclone copyto $(basename "$SRC") → r2:$R2_BUCKET/$DST"
  rclone --config /dev/null copyto "$SRC" ":s3:${R2_BUCKET}/${DST}"
done

echo ""
ok "macOS release v${VERSION} published"
echo ""
echo "  ┌─ live manifest URLs ──────────────────────────────────────"
for entry in "${UPLOADS[@]}"; do
  IFS="|" read -r SRC DST <<< "$entry"
  if [[ "$DST" == update/* ]]; then
    printf '     https://r.cameo.ink/%s\n' "$DST"
  fi
done
echo "  └───────────────────────────────────────────────────────────"
echo ""
warn "REMINDER: bump src-tauri/tauri.conf.json's version BEFORE running this"
warn "script for the next release, or you'll re-publish v${VERSION}."
warn "Windows publishes separately — run publish_release.ps1 on the Windows box."
