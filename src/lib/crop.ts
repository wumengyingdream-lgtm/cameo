import type { Asset, Rect } from "../types";
import { cameoUrl } from "./ipc";

function loadImage(url: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`load ${url}`));
    img.src = url;
  });
}

/** Render a normalized crop rect [0,1] of an asset to PNG bytes for
 *  replace_placement_image. Non-destructive — the original file is untouched. */
export async function bakeCrop(boardId: string, asset: Asset, rect: Rect): Promise<number[]> {
  const img = await loadImage(cameoUrl(boardId, asset.path));
  const W = asset.width;
  const H = asset.height;
  const sx = Math.max(0, rect.x * W);
  const sy = Math.max(0, rect.y * H);
  const sw = Math.max(1, Math.min(W - sx, rect.w * W));
  const sh = Math.max(1, Math.min(H - sy, rect.h * H));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(sw);
  canvas.height = Math.round(sh);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.drawImage(img, sx, sy, sw, sh, 0, 0, canvas.width, canvas.height);
  const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, "image/png"));
  if (!blob) throw new Error("toBlob failed");
  return Array.from(new Uint8Array(await blob.arrayBuffer()));
}
