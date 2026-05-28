import { useEffect, useState } from "react";
import { cameoUrl, ipc } from "./ipc";

const objectUrlCache = new Map<string, Promise<string>>();

function cacheKey(boardId: string, relPath: string): string {
  return `${boardId}\0${relPath}`;
}

export function loadAssetObjectUrl(boardId: string, relPath: string, mime = "image/png"): Promise<string> {
  const key = cacheKey(boardId, relPath);
  const cached = objectUrlCache.get(key);
  if (cached) return cached;

  const promise = ipc
    .readAssetBytes(boardId, relPath)
    .then((bytes) => {
      const blob = new Blob([new Uint8Array(bytes)], { type: mime });
      return URL.createObjectURL(blob);
    })
    .catch((err) => {
      objectUrlCache.delete(key);
      throw err;
    });

  objectUrlCache.set(key, promise);
  return promise;
}

export function useAssetObjectUrl(boardId: string | null, relPath: string | null, mime?: string | null): string | null {
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    if (!boardId || !relPath) {
      setUrl(null);
      return;
    }

    let live = true;
    setUrl(null);
    loadAssetObjectUrl(boardId, relPath, mime ?? "image/png")
      .then((next) => {
        if (live) setUrl(next);
      })
      .catch(() => {
        if (live) setUrl(cameoUrl(boardId, relPath));
      });

    return () => {
      live = false;
    };
  }, [boardId, relPath, mime]);

  return url;
}
