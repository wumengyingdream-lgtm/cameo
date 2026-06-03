import { useEffect, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useBoardStore } from "../store/board";
import { useClipboardStore, CAMEO_CLIP_MARKER } from "../store/clipboard";

interface FileImportOptions {
  onDrop?: (paths: string[], position?: { x: number; y: number }) => void;
}

/** Wires OS file-drop and clipboard-image paste into the open Board. */
export function useFileImport(options: FileImportOptions = {}) {
  const onDropRef = useRef(options.onDrop);

  useEffect(() => {
    onDropRef.current = options.onDrop;
  }, [options.onDrop]);

  // Native drag-and-drop of files onto the window.
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") {
          const paths = event.payload.paths ?? [];
          if (paths.length) {
            const dpr = window.devicePixelRatio || 1;
            const position = event.payload.position
              ? { x: event.payload.position.x / dpr, y: event.payload.position.y / dpr }
              : undefined;
            const onDrop = onDropRef.current;
            if (onDrop) onDrop(paths, position);
            else void useBoardStore.getState().importFiles(paths);
          }
        }
      })
      .then((u) => {
        if (cancelled) u();
        else unlisten = u;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  // Clipboard paste onto the canvas. Two sources, in priority order:
  //  1. The in-app canvas clipboard (Cmd+C of placements) — multi-select,
  //     cross-board, and videos. Re-imports every copied item by source path.
  //  2. OS clipboard image/video bytes — a single picture/clip pasted from
  //     another app, imported as one new placement.
  // Pastes targeting an editable (the chat composer / inputs) are left alone —
  // the composer has its own paste handler that turns content into a reference.
  useEffect(() => {
    const onPaste = async (e: ClipboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }
      // In-app canvas clipboard wins when it's still the most recent copy — the
      // OS clipboard carries our marker (trim: some managers strip whitespace).
      // If the marker couldn't be written at all (`marked` false), trust the
      // in-app store as a fallback rather than silently dropping the copy.
      // Otherwise something was copied elsewhere after us → fall through to bytes.
      const clip = useClipboardStore.getState();
      const markerMatch = e.clipboardData?.getData("text/plain")?.trim() === CAMEO_CLIP_MARKER;
      if (clip.items.length > 0 && (markerMatch || !clip.marked)) {
        e.preventDefault();
        void useBoardStore.getState().pasteClipboard();
        return;
      }
      const items = e.clipboardData?.items;
      if (!items) return;
      for (const it of items) {
        if (it.type.startsWith("image/") || it.type.startsWith("video/")) {
          const file = it.getAsFile();
          if (!file) continue;
          const buf = new Uint8Array(await file.arrayBuffer());
          const ext = it.type.split("/")[1] || "png";
          void useBoardStore.getState().importBytes(buf, ext, "image");
          break;
        }
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);
}
