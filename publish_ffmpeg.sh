#!/usr/bin/env bash
# Publish a managed-ffmpeg build set to R2 for Cameo's video modality.
#
# Cameo detects the user's own ffmpeg/ffprobe first; only if missing does it
# download a PINNED build from this manifest into ~/.cameo/bin, verifying the
# blake3 hash BEFORE the binary is ever made executable (see
# src-tauri/src/tools/ffmpeg.rs). This script computes those hashes + sizes and
# emits/uploads the manifest the app fetches at:
#
#   https://r.cameo.ink/tools/ffmpeg/manifest.json
#
# It mirrors publish_release.sh (rclone → R2, optional CDN purge, HEAD verify).
#
# Usage:
#   ./publish_ffmpeg.sh --version 8.1.1 --in ./ffmpeg-stage \
#       --source-file ./ffmpeg-8.1.1.tar.xz --accept-gpl [--dry-run] [--local-only]
#
# Input layout (--in DIR): one subdir per platform, each holding the two
# binaries. Only the platforms present are published (partial sets are fine).
#   <DIR>/mac-arm64/ffmpeg      <DIR>/mac-arm64/ffprobe
#   <DIR>/mac-x64/ffmpeg        <DIR>/mac-x64/ffprobe
#   <DIR>/win-x64/ffmpeg.exe    <DIR>/win-x64/ffprobe.exe
# Platform keys MUST match src-tauri/src/tools/ffmpeg.rs::platform_key().
#
# GPL note (PRD §8.3): the full ffmpeg you ship is GPL (x264/x265) and may carry
# H.264/HEVC patent obligations. Publishing here makes Cameo a distributor — so
# the corresponding source MUST be hosted and linked. Pass --source-file PATH to
# upload the source tarball alongside the binaries (the manifest's `source` URL
# is auto-derived from it, so it can't dangle), or --source-url if you host it
# elsewhere. This script refuses unless --accept-gpl is passed, so it can't be
# run unaware.
#
# R2 credentials come from .env — the SAME ones publish_release.sh uses
# (loaded the SAME way: `set -a; . ./.env`), so a working release setup needs
# zero extra config here:
#   R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY / R2_ENDPOINT  (S3 creds)
#   R2_BUCKET             R2 bucket the r.cameo.ink domain serves (default: cameo-gallery-images)
#   CAMEO_R2_PUBLIC_BASE  public base URL                          (default: https://r.cameo.ink)
#   CF_ZONE_ID/CF_API_TOKEN  optional Cloudflare cache purge
#
# Upload uses rclone's ad-hoc `:s3:` backend driven by RCLONE_S3_* env vars
# (NOT a named remote) — identical to publish_release.sh, so no `rclone config`
# is required. r.cameo.ink is the custom domain on the R2 bucket named in
# R2_BUCKET (releases + gallery images already live there); the ffmpeg manifest
# lands at the `tools/ffmpeg/` prefix in that same bucket.
#
# Prereqs: rclone (`brew install rclone`), b3sum (blake3 CLI — `brew install
# b3sum` or `cargo install b3sum`). The hash MUST be blake3: the app rejects a
# mismatch and never executes unverified bytes.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$ROOT/dist-ffmpeg" # manifest is always written here for inspection

# Load R2 creds from .env exactly as publish_release.sh does. Best-effort so
# --local-only works without it; the upload path validates the vars are present.
if [[ -f "$ROOT/.env" ]]; then set -a; . "$ROOT/.env"; set +a; fi

cyan() { printf '\033[36m%s\033[0m\n' "$1"; }
yellow() { printf '\033[33m%s\033[0m\n' "$1"; }
red() { printf '\033[31m%s\033[0m\n' "$1"; }
die() { red "✗ $1" >&2; exit 1; }

# ── args ─────────────────────────────────────────────────────────────────────
VERSION=""
IN_DIR=""
DRY_RUN=0
LOCAL_ONLY=0
ACCEPT_GPL=0
SOURCE_URL=""
SOURCE_FILE=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2;;
    --in) IN_DIR="$2"; shift 2;;
    --source-url) SOURCE_URL="$2"; shift 2;;
    --source-file) SOURCE_FILE="$2"; shift 2;;  # local GPL source tarball → uploaded + linked
    --dry-run) DRY_RUN=1; shift;;
    --local-only) LOCAL_ONLY=1; shift;;  # compute + emit manifest, no upload
    --accept-gpl) ACCEPT_GPL=1; shift;;
    *) die "unknown arg: $1";;
  esac
done

[[ -n "$VERSION" ]] || die "pass --version (e.g. --version 7.1)"
[[ -n "$IN_DIR" ]] || die "pass --in DIR (staging dir with per-platform subdirs)"
[[ -d "$IN_DIR" ]] || die "input dir not found: $IN_DIR"
if [[ "$ACCEPT_GPL" -ne 1 ]]; then
  die "refusing to publish a GPL ffmpeg without --accept-gpl (see PRD §8.3: GPL source-offer + H.264/HEVC patents). Upload corresponding source and pass --source-url + --accept-gpl."
fi

command -v b3sum >/dev/null 2>&1 || die "b3sum not found — install with 'brew install b3sum' or 'cargo install b3sum' (the app verifies blake3)"

# Bucket the r.cameo.ink custom domain serves (releases + images already live
# there). Override via R2_BUCKET in .env/shell; default matches the live setup.
R2_BUCKET="${R2_BUCKET:-cameo-gallery-images}"
R2_PUBLIC_BASE="${CAMEO_R2_PUBLIC_BASE:-https://r.cameo.ink}"

# --source-file: host the GPL corresponding-source tarball ourselves so the
# manifest's `source` link can't dangle (the GPL obligation is only met if the
# source is actually reachable). It's uploaded alongside the binaries and, unless
# --source-url was given explicitly, becomes the source URL. SOURCE_REL is added
# to UPLOADS after the array is initialized below.
SOURCE_REL=""
if [[ -n "$SOURCE_FILE" ]]; then
  [[ -f "$SOURCE_FILE" ]] || die "source file not found: $SOURCE_FILE"
  SOURCE_REL="tools/ffmpeg/$VERSION/source/$(basename "$SOURCE_FILE")"
  [[ -n "$SOURCE_URL" ]] || SOURCE_URL="$R2_PUBLIC_BASE/$SOURCE_REL"
fi

if [[ "$LOCAL_ONLY" -ne 1 ]]; then
  command -v rclone >/dev/null 2>&1 || die "rclone not found — brew install rclone (or use --local-only)"
  for v in R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_ENDPOINT R2_BUCKET; do
    [[ -n "${!v:-}" ]] || die "$v missing — set it in .env (same creds publish_release.sh uses), or run with --local-only"
  done
fi

cyan "Cameo ffmpeg publisher · v$VERSION"
mkdir -p "$OUT"

PLATFORMS=(mac-arm64 mac-x64 win-x64)
b3_of() { b3sum "$1" | awk '{print $1}'; }
size_of() { wc -c < "$1" | tr -d ' '; }

# Each platform contributes a JSON fragment "key": { ffmpeg{...}, ffprobe{...} }.
FRAGMENTS=()
UPLOADS=() # "src::dest-rel" pairs, uploaded after the manifest is built

# NOTE: this is called inside $(…) command substitution, which runs in a
# SUBSHELL — so it must NOT try to record uploads via `UPLOADS+=` (that write
# would be lost; bash arrays don't propagate out of a subshell). The caller
# tracks UPLOADS in the parent shell using the SAME rel path computed here.
emit_tool() { # platform bin_name local_path  →  prints  "name": { url, blake3, size }
  local platform="$1" name="$2" path="$3"
  local hash size url
  hash="$(b3_of "$path")"
  size="$(size_of "$path")"
  url="$R2_PUBLIC_BASE/$(rel_for "$platform" "$path")"
  printf '      "%s": { "url": "%s", "blake3": "%s", "size": %s }' "$name" "$url" "$hash" "$size"
}

# Single source of truth for an object's R2 key, used by BOTH emit_tool (URL in
# the manifest) and the parent-shell UPLOADS bookkeeping — they MUST agree.
rel_for() { # platform local_path  →  prints  tools/ffmpeg/<ver>/<platform>/<basename>
  printf 'tools/ffmpeg/%s/%s/%s' "$VERSION" "$1" "$(basename "$2")"
}

for platform in "${PLATFORMS[@]}"; do
  dir="$IN_DIR/$platform"
  [[ -d "$dir" ]] || { yellow "  skip $platform (no dir)"; continue; }
  if [[ "$platform" == win-x64 ]]; then
    ff="$dir/ffmpeg.exe"; fp="$dir/ffprobe.exe"
  else
    ff="$dir/ffmpeg"; fp="$dir/ffprobe"
  fi
  [[ -f "$ff" && -f "$fp" ]] || die "$platform: need both $(basename "$ff") and $(basename "$fp") in $dir"
  cyan "  hashing $platform"
  frag="$(printf '    "%s": {\n%s,\n%s\n    }' \
    "$platform" \
    "$(emit_tool "$platform" ffmpeg "$ff")" \
    "$(emit_tool "$platform" ffprobe "$fp")")"
  FRAGMENTS+=("$frag")
  # Record uploads in the PARENT shell — emit_tool's subshell can't (see above).
  UPLOADS+=("$ff::$(rel_for "$platform" "$ff")")
  UPLOADS+=("$fp::$(rel_for "$platform" "$fp")")
done

[[ ${#FRAGMENTS[@]} -gt 0 ]] || die "no platforms found under $IN_DIR (expected mac-arm64/ mac-x64/ win-x64/)"

# Queue the GPL source tarball for upload too (if --source-file was given).
[[ -n "$SOURCE_REL" ]] && UPLOADS+=("$SOURCE_FILE::$SOURCE_REL")

# ── build manifest (exact shape src-tauri/src/tools/ffmpeg.rs parses) ─────────
# Join fragments with ",\n" BETWEEN them — never a trailing comma (invalid JSON;
# the partial-platform-set case is common, so this must be exact).
builds_json=""
for i in "${!FRAGMENTS[@]}"; do
  [[ "$i" -gt 0 ]] && builds_json+=$',\n'
  builds_json+="${FRAGMENTS[$i]}"
done
MANIFEST="$OUT/manifest.json"
{
  printf '{\n'
  printf '  "tool": "ffmpeg",\n'
  printf '  "version": "%s",\n' "$VERSION"
  printf '  "builds": {\n%s\n  }' "$builds_json"
  if [[ -n "$SOURCE_URL" ]]; then
    printf ',\n  "source": "%s"\n' "$SOURCE_URL"
  else
    printf '\n'
  fi
  printf '}\n'
} > "$MANIFEST"
cyan "  manifest → $MANIFEST"
cat "$MANIFEST"
# Fail fast if the host has python/node to sanity-check JSON validity.
if command -v python3 >/dev/null 2>&1; then
  python3 -c "import json,sys; json.load(open('$MANIFEST'))" || die "generated manifest is not valid JSON"
fi
[[ -n "$SOURCE_URL" ]] || yellow "  ⚠ no --source-url: GPL corresponding-source link omitted from manifest (PRD §8.3)"

# ── upload ────────────────────────────────────────────────────────────────────
if [[ "$LOCAL_ONLY" -eq 1 ]]; then
  cyan "✓ local-only: manifest emitted, nothing uploaded"
  exit 0
fi

# Configure rclone's ad-hoc `:s3:` backend via env vars — identical to
# publish_release.sh (no named remote / `rclone config` needed). R2_ENDPOINT
# contains "://", so it MUST go through the env var, never an inline
# `:s3,endpoint=…:` connection string (rclone truncates that at the first colon).
export RCLONE_S3_PROVIDER="Cloudflare"
export RCLONE_S3_ACCESS_KEY_ID="$R2_ACCESS_KEY_ID"
export RCLONE_S3_SECRET_ACCESS_KEY="$R2_SECRET_ACCESS_KEY"
export RCLONE_S3_ENDPOINT="$R2_ENDPOINT"
export RCLONE_S3_REGION="auto"
# R2 tokens can't create buckets; skip rclone's pre-upload CreateBucket probe
# (it'd 403) — the bucket already exists.
export RCLONE_S3_NO_CHECK_BUCKET="true"

RCLONE_FLAGS=()
[[ "$DRY_RUN" -eq 1 ]] && RCLONE_FLAGS+=(--dry-run)

upload() { # local_path  dest_rel
  local src="$1" rel="$2"
  cyan "  ↑ $rel"
  # `${arr[@]+"${arr[@]}"}` guard: on macOS's bash 3.2, expanding an EMPTY array
  # under `set -u` aborts with "unbound variable" — this expands to nothing when
  # RCLONE_FLAGS is empty (the normal, non-dry-run case) instead of erroring.
  rclone --config /dev/null ${RCLONE_FLAGS[@]+"${RCLONE_FLAGS[@]}"} copyto "$src" ":s3:${R2_BUCKET}/${rel}" || die "upload failed: $src"
}

for pair in ${UPLOADS[@]+"${UPLOADS[@]}"}; do
  upload "${pair%%::*}" "${pair##*::}"
done
upload "$MANIFEST" "tools/ffmpeg/manifest.json"

# ── optional CDN purge + verify (best-effort, mirrors publish_release.sh) ──────
if [[ "$DRY_RUN" -ne 1 ]]; then
  if [[ -n "${CF_ZONE_ID:-}" && -n "${CF_API_TOKEN:-}" ]]; then
    cyan "  purging CDN cache for manifest.json"
    curl -fsS -X POST "https://api.cloudflare.com/client/v4/zones/$CF_ZONE_ID/purge_cache" \
      -H "Authorization: Bearer $CF_API_TOKEN" -H "Content-Type: application/json" \
      --data "{\"files\":[\"$R2_PUBLIC_BASE/tools/ffmpeg/manifest.json\"]}" >/dev/null || yellow "  ⚠ CDN purge failed (non-fatal)"
  fi
  cyan "  verifying $R2_PUBLIC_BASE/tools/ffmpeg/manifest.json"
  curl -fsS -I "$R2_PUBLIC_BASE/tools/ffmpeg/manifest.json" >/dev/null || yellow "  ⚠ HEAD verify failed (CDN may lag)"
fi

cyan "✓ ffmpeg v$VERSION published"
