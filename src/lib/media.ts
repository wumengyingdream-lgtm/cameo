// Media-kind helpers. `mediaKind` is DERIVED from an Asset's mime (never stored)
// so it can't drift from the file. One place owns the image/video split; every
// consumer (canvas, chat, selection bar) asks here.

import type { Asset, MediaKind } from "../types";

/** Video container extensions Cameo treats as time-based media. Mirrors
 *  `VIDEO_EXTS` in src-tauri/src/assets.rs — keep in sync. */
export const VIDEO_EXTS = ["mp4", "webm", "mov", "m4v"] as const;

/** Regex-alternation of the video extensions (e.g. "mp4|webm|mov|m4v"), so the
 *  chat-path detector (chatImageDetect.ts) derives its video list from this one
 *  source instead of re-listing them. */
export const VIDEO_EXT_ALT = VIDEO_EXTS.join("|");

export function mediaKindOfMime(mime: string | null | undefined): MediaKind {
  return mime?.startsWith("video/") ? "video" : "image";
}

export function mediaKindOf(asset: Asset | null | undefined): MediaKind {
  return mediaKindOfMime(asset?.mime);
}

export function isVideoAsset(asset: Asset | null | undefined): boolean {
  return mediaKindOf(asset) === "video";
}

/** The Board-relative path the canvas should render as the still texture: a
 *  video's poster frame when present, otherwise the asset's own file (images,
 *  or a poster-less video which falls back to its placeholder). */
export function stillPathOf(asset: Asset): string {
  if (isVideoAsset(asset) && asset.posterPath) return asset.posterPath;
  return asset.path;
}
