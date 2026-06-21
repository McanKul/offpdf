/** Renders & caches a single page-1 thumbnail for a file (for file chips). */
import { useEffect, useState } from "react";
import { renderThumbnails } from "@/lib/tauriCommands";

const cache = new Map<string, string>();

export function useFileThumb(path: string | undefined, size = 160): string | null {
  const [url, setUrl] = useState<string | null>(path ? cache.get(path) ?? null : null);

  useEffect(() => {
    if (!path) {
      setUrl(null);
      return;
    }
    const cached = cache.get(path);
    if (cached) {
      setUrl(cached);
      return;
    }
    let active = true;
    renderThumbnails(path, [1], size)
      .then((res) => {
        const u = res[0]?.dataUrl;
        if (u) {
          cache.set(path, u);
          if (active) setUrl(u);
        }
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [path, size]);

  return url;
}
