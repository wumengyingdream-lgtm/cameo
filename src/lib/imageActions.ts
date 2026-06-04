import { open, save } from "@tauri-apps/plugin-dialog";
import { useBoardStore } from "../store/board";
import { useChatStore } from "../store/chat";
import { track } from "../services/cloud/telemetry";
import { ipc } from "./ipc";
import { buildOverlays, buildMarkNotes } from "./overlay";
import type { Asset, Placement, TextNode } from "../types";

// Preset prompts — ALWAYS English (agent-facing; UI language independent).
export const PRESET_REMOVE_BG =
  "Remove the background from this image — keep only the subject, and make the background transparent (PNG) or solid white.";
export const PRESET_UPSCALE =
  "Increase this image's sharpness and resolution so it looks crisper and more detailed, keeping the content and composition unchanged.";

/** A placement's filename (export default + display). */
export function imageName(pid: string): string {
  const { placements, assets } = useBoardStore.getState();
  const p = placements.get(pid);
  const a = p && assets.get(p.assetId);
  return a ? (a.path.split("/").pop() ?? "image.png") : "image.png";
}

function pngName(pid: string): string {
  return imageName(pid).replace(/\.[^.\\/]+$/, "") + ".png";
}

function joinPath(dir: string, name: string): string {
  const sep = dir.includes("\\") ? "\\" : "/";
  return dir.endsWith("\\") || dir.endsWith("/") ? `${dir}${name}` : `${dir}${sep}${name}`;
}

function uniqueNames(names: string[]): string[] {
  const seen = new Map<string, number>();
  return names.map((name) => {
    const dot = name.lastIndexOf(".");
    const stem = dot > 0 ? name.slice(0, dot) : name;
    const ext = dot > 0 ? name.slice(dot) : "";
    const key = name.toLowerCase();
    const count = seen.get(key) ?? 0;
    seen.set(key, count + 1);
    if (count === 0) return name;
    return `${stem} ${count + 1}${ext}`;
  });
}

function cssFont(style: TextNode["style"]): string {
  const italic = style.italic ? "italic " : "";
  const weight = style.bold ? "700" : "400";
  const family = style.fontFamily.replace(/"/g, '\\"');
  return `${italic}${weight} ${style.fontSize}px "${family}"`;
}

function worldToImage(p: Placement, asset: Asset, x: number, y: number): { x: number; y: number } {
  const dx = x - p.x;
  const dy = y - p.y;
  const cos = Math.cos(p.rotation);
  const sin = Math.sin(p.rotation);
  return {
    x: (cos * dx + sin * dy) / p.scale + asset.width / 2,
    y: (-sin * dx + cos * dy) / p.scale + asset.height / 2,
  };
}

function textIntersectsImage(p: Placement, asset: Asset, t: TextNode): boolean {
  if (t.z < p.z) return false;
  const cos = Math.cos(t.rotation);
  const sin = Math.sin(t.rotation);
  const hw = t.w / 2;
  const hh = t.h / 2;
  const corners = [
    [-hw, -hh],
    [hw, -hh],
    [hw, hh],
    [-hw, hh],
  ].map(([x, y]) =>
    worldToImage(
      p,
      asset,
      t.x + t.scale * (cos * x - sin * y),
      t.y + t.scale * (sin * x + cos * y),
    ),
  );
  const minX = Math.min(...corners.map((c) => c.x));
  const maxX = Math.max(...corners.map((c) => c.x));
  const minY = Math.min(...corners.map((c) => c.y));
  const maxY = Math.max(...corners.map((c) => c.y));
  return maxX >= 0 && minX <= asset.width && maxY >= 0 && minY <= asset.height;
}

function wrapText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
  const out: string[] = [];
  const paragraphs = text.split(/\r?\n/);
  for (const paragraph of paragraphs) {
    const tokens = paragraph.includes(" ") ? paragraph.split(/(\s+)/) : [...paragraph];
    let line = "";
    for (const token of tokens) {
      const next = line + token;
      if (line && ctx.measureText(next).width > maxWidth) {
        out.push(line.trimEnd());
        line = token.trimStart();
      } else {
        line = next;
      }
    }
    out.push(line);
  }
  return out;
}

function drawTextLine(
  ctx: CanvasRenderingContext2D,
  line: string,
  x: number,
  y: number,
  align: TextNode["style"]["align"],
  letterSpacing: number,
): void {
  if (!letterSpacing) {
    ctx.textAlign = align;
    ctx.fillText(line, x, y);
    return;
  }
  const chars = [...line];
  const width = chars.reduce((sum, ch, idx) => sum + ctx.measureText(ch).width + (idx ? letterSpacing : 0), 0);
  let cursor = align === "center" ? x - width / 2 : align === "right" ? x - width : x;
  ctx.textAlign = "left";
  for (const ch of chars) {
    ctx.fillText(ch, cursor, y);
    cursor += ctx.measureText(ch).width + letterSpacing;
  }
}

function drawTextNode(ctx: CanvasRenderingContext2D, p: Placement, asset: Asset, t: TextNode): void {
  const k = t.scale / p.scale;
  const angle = t.rotation - p.rotation;
  const cos = Math.cos(angle) * k;
  const sin = Math.sin(angle) * k;
  const center = worldToImage(p, asset, t.x, t.y);
  ctx.save();
  ctx.transform(cos, sin, -sin, cos, center.x, center.y);
  ctx.beginPath();
  ctx.rect(-t.w / 2, -t.h / 2, t.w, t.h);
  ctx.clip();
  ctx.font = cssFont(t.style);
  ctx.fillStyle = t.style.color;
  ctx.textBaseline = "top";
  const lineHeight = Math.max(0.6, t.style.lineHeight || 1.2) * t.style.fontSize;
  const lines = wrapText(ctx, t.text || " ", t.w);
  const totalH = lines.length * lineHeight;
  let y = -totalH / 2;
  const x = t.style.align === "center" ? 0 : t.style.align === "right" ? t.w / 2 : -t.w / 2;
  for (const line of lines) {
    drawTextLine(ctx, line, x, y, t.style.align, t.style.letterSpacing);
    y += lineHeight;
  }
  ctx.restore();
}

async function imageFromAsset(boardId: string, asset: Asset): Promise<ImageBitmap> {
  const bytes = await ipc.readAssetBytes(boardId, asset.path);
  const blob = new Blob([new Uint8Array(bytes)], { type: asset.mime || "image/png" });
  return createImageBitmap(blob);
}

function overlayTextsFor(p: Placement, asset: Asset): TextNode[] {
  const { textNodes } = useBoardStore.getState();
  return [...textNodes.values()]
    .filter((t) => textIntersectsImage(p, asset, t))
    .sort((a, b) => a.z - b.z);
}

async function renderPlacementWithText(boardId: string, p: Placement, asset: Asset, texts: TextNode[]): Promise<number[]> {
  const bitmap = await imageFromAsset(boardId, asset);
  const canvas = document.createElement("canvas");
  canvas.width = asset.width || bitmap.width;
  canvas.height = asset.height || bitmap.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("Could not create export canvas");
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();
  for (const text of texts) drawTextNode(ctx, p, asset, text);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) throw new Error("Could not encode PNG");
  return [...new Uint8Array(await blob.arrayBuffer())];
}

async function exportCompositeIfNeeded(boardId: string, pid: string, dest: string): Promise<boolean> {
  const { placements, assets } = useBoardStore.getState();
  const p = placements.get(pid);
  const asset = p && assets.get(p.assetId);
  if (!p || !asset || !asset.mime.startsWith("image/")) return false;
  const texts = overlayTextsFor(p, asset);
  if (!texts.length) return false;
  const bytes = await renderPlacementWithText(boardId, p, asset, texts);
  await ipc.exportRenderedImage(dest, bytes);
  return true;
}

/** Copy one image to the system clipboard, flattening text overlays when present. */
export async function copyImageToClipboard(boardId: string, pid: string): Promise<void> {
  const { placements, assets } = useBoardStore.getState();
  const p = placements.get(pid);
  const asset = p && assets.get(p.assetId);
  if (p && asset && asset.mime.startsWith("image/")) {
    const texts = overlayTextsFor(p, asset);
    if (texts.length) {
      const bytes = await renderPlacementWithText(boardId, p, asset, texts);
      await ipc.copyRenderedImage(bytes);
      return;
    }
  }
  await ipc.copyImage(boardId, pid);
}

/** Run a preset prompt against one image (去背景 / 变高清). Any marks on the image
 *  are composed into the message and consumed after the turn is sent. */
export function runImagePreset(boardId: string, pid: string, presetPrompt: string): void {
  const notes = buildMarkNotes([pid]);
  const instruction = [notes, presetPrompt].filter((s) => s.trim()).join("\n\n");
  const turn = useChatStore.getState().startTurn(instruction, [pid]);
  void (async () => {
    try {
      const ov = await buildOverlays(boardId, [pid]);
      await ipc.sendMessage(boardId, instruction, [pid], ov);
      useBoardStore.getState().consumeMarks([pid]);
    } catch (e) {
      const message = `Could not send this preset: ${e instanceof Error ? e.message : String(e)}`;
      useChatStore.getState().failTurn(message, turn);
      void ipc.frontLog("error", message).catch(() => {});
    }
  })();
}

/** Export one image via a native save dialog. */
export async function exportImage(boardId: string, pid: string): Promise<void> {
  const { placements, assets } = useBoardStore.getState();
  const p = placements.get(pid);
  const asset = p && assets.get(p.assetId);
  const hasText = !!p && !!asset && asset.mime.startsWith("image/") && overlayTextsFor(p, asset).length > 0;
  const dest = await save({ defaultPath: hasText ? pngName(pid) : imageName(pid), title: "Export image" });
  if (typeof dest === "string") {
    const rendered = await exportCompositeIfNeeded(boardId, pid, dest);
    if (!rendered) await ipc.exportAsset(boardId, pid, dest);
    void track("image_exported", { count: 1 });
  }
}

/** Export the current selection. Single image keeps Save As; multi-select picks
 *  a folder and preserves each source filename inside it. */
export async function exportImages(boardId: string, ids: string[]): Promise<void> {
  const placements = useBoardStore.getState().placements;
  const uniqueIds = [...new Set(ids)].filter((id) => placements.has(id));
  if (uniqueIds.length === 0) return;
  if (uniqueIds.length === 1) {
    await exportImage(boardId, uniqueIds[0]);
    return;
  }

  const dest = await open({ directory: true, multiple: false, title: "Export images" });
  if (typeof dest === "string") {
    const names = uniqueNames(uniqueIds.map((id) => pngName(id)));
    const plainIds: string[] = [];
    for (const [i, id] of uniqueIds.entries()) {
      const rendered = await exportCompositeIfNeeded(boardId, id, joinPath(dest, names[i]));
      if (!rendered) plainIds.push(id);
    }
    if (plainIds.length) await ipc.exportAssets(boardId, plainIds, dest);
    void track("image_exported", { count: uniqueIds.length });
  }
}
