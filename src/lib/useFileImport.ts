import { useEffect, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useBoardStore } from "../store/board";

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

  // Clipboard paste of an image.
  useEffect(() => {
    const onPaste = async (e: ClipboardEvent) => {
      const items = e.clipboardData?.items;
      if (!items) return;
      for (const it of items) {
        if (it.type.startsWith("image/")) {
          const file = it.getAsFile();
          if (!file) continue;
          const buf = new Uint8Array(await file.arrayBuffer());
          const ext = it.type.split("/")[1] || "png";
          void useBoardStore.getState().importBytes(buf, ext, "image");
        }
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);
}
