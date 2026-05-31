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
#   ./publish_ffmpeg.sh --version 7.1 --in ./ffmpeg-stage [--dry-run] [--local-only]
#
# Input layout (--in DIR): one subdir per platform, each holding the two
# binaries. Only the platforms present are published (partial sets are fine).
#   <DIR>/mac-arm64/ffmpeg      <DIR>/mac-arm64/ffprobe
#   <DIR>/mac-x64/ffmpeg        <DIR>/mac-x64/ffprobe
#   <DIR>/win-x64/ffmpeg.exe    <DIR>/win-x64/ffprobe.exe
# Platform keys MUST match src-tauri/src/tools/ffmpeg.rs::platform_key().
#
# GPL note (PRD §8.3): the full ffmpeg you ship is GPL (x264/x265) and may carry
# H.264/HEVC patent obligations. Publishing here makes Cameo a distributor —
# also upload the corresponding source and point `source` at it. This script
# will refuse unless --accept-gpl is passed, so it can't be run unaware.
#
# Env (.env or shell), reused from publish_release.sh:
#   CAMEO_R2_REMOTE       rclone remote name              (default: r2)
#   CAMEO_R2_BUCKET       R2 bucket                       (default: cameo-dist)
#   CAMEO_R2_PUBLIC_BASE  public base URL                 (default: https://r.cameo.ink)
#   CF_ZONE_ID/CF_API_TOKEN  optional Cloudflare cache purge
#
# Prereqs: rclone (configured R2 remote), b3sum (blake3 CLI — `brew install
# b3sum` or `cargo install b3sum`). The hash MUST be blake3: the app rejects a
# mismatch and never executes unverified bytes.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$ROOT/dist-ffmpeg" # manifest is always written here for inspection

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
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2;;
    --in) IN_DIR="$2"; shift 2;;
    --source-url) SOURCE_URL="$2"; shift 2;;
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

RCLONE_REMOTE="${CAMEO_R2_REMOTE:-r2}"
R2_BUCKET="${CAMEO_R2_BUCKET:-cameo-dist}"
R2_PUBLIC_BASE="${CAMEO_R2_PUBLIC_BASE:-https://r.cameo.ink}"

if [[ "$LOCAL_ONLY" -ne 1 ]]; then
  command -v rclone >/dev/null 2>&1 || die "rclone not found — install + configure an R2 remote (or use --local-only)"
fi

cyan "Cameo ffmpeg publisher · v$VERSION"
mkdir -p "$OUT"

PLATFORMS=(mac-arm64 mac-x64 win-x64)
b3_of() { b3sum "$1" | awk '{print $1}'; }
size_of() { wc -c < "$1" | tr -d ' '; }

# Each platform contributes a JSON fragment "key": { ffmpeg{...}, ffprobe{...} }.
FRAGMENTS=()
UPLOADS=() # "src::dest-rel" pairs, uploaded after the manifest is built

emit_tool() { # platform bin_name local_path  →  prints  "name": { url, blake3, size }
  local platform="$1" name="$2" path="$3"
  local hash size rel url
  hash="$(b3_of "$path")"
  size="$(size_of "$path")"
  rel="tools/ffmpeg/$VERSION/$platform/$(basename "$path")"
  url="$R2_PUBLIC_BASE/$rel"
  UPLOADS+=("$path::$rel")
  printf '      "%s": { "url": "%s", "blake3": "%s", "size": %s }' "$name" "$url" "$hash" "$size"
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
done

[[ ${#FRAGMENTS[@]} -gt 0 ]] || die "no platforms found under $IN_DIR (expected mac-arm64/ mac-x64/ win-x64/)"

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

RCLONE_FLAGS=(--s3-no-check-bucket)
[[ "$DRY_RUN" -eq 1 ]] && RCLONE_FLAGS+=(--dry-run)
dest_base="$RCLONE_REMOTE:$R2_BUCKET"

upload() { # local_path  dest_rel
  local src="$1" rel="$2"
  cyan "  ↑ $rel"
  rclone "${RCLONE_FLAGS[@]}" copyto "$src" "$dest_base/$rel" || die "upload failed: $src"
}

for pair in "${UPLOADS[@]}"; do
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
