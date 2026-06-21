/** Renders one page with pdf.js to a canvas — true vector, so it stays crisp at
 * any zoom (re-rasterized at the displayed scale, like Acrobat). Falls back via
 * onFail() if pdf.js can't handle the page. */
import { useEffect, useRef } from "react";
import { pdfjsLib, PDF_OPTS } from "@/lib/pdfjs";

const MAX_CANVAS_SIDE = 8000;

export function PdfVectorView({
  bytes,
  zoom,
  highlight,
  onFail,
}: {
  bytes: Uint8Array;
  zoom: number;
  highlight?: string;
  onFail: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const docRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const taskRef = useRef<any>(null);
  const zoomRef = useRef(zoom);
  zoomRef.current = zoom;
  const hlRef = useRef(highlight);
  hlRef.current = highlight;

  // Load the (single-page) document once per page.
  useEffect(() => {
    let cancelled = false;
    const loading = pdfjsLib.getDocument({ data: bytes, ...PDF_OPTS });
    loading.promise
      .then((doc: unknown) => {
        if (cancelled) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          (doc as any)?.destroy?.();
          return;
        }
        docRef.current = doc;
        void renderPage();
      })
      .catch(() => onFail());
    return () => {
      cancelled = true;
      try {
        taskRef.current?.cancel?.();
      } catch {
        /* ignore */
      }
      try {
        docRef.current?.destroy?.();
      } catch {
        /* ignore */
      }
      docRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bytes]);

  // Re-render at the current zoom / when the search term changes.
  useEffect(() => {
    if (docRef.current) void renderPage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom, highlight]);

  const renderPage = async () => {
    const doc = docRef.current;
    const canvas = canvasRef.current;
    if (!doc || !canvas) return;
    try {
      const z = zoomRef.current;
      const page = await doc.getPage(1);
      const dpr = window.devicePixelRatio || 1;
      const vh = window.innerHeight || 900;
      const base = page.getViewport({ scale: 1 });
      let scale = (vh * 0.8 * z * dpr) / base.height;
      const longest = Math.max(base.width, base.height) * scale;
      if (longest > MAX_CANVAS_SIDE) scale *= MAX_CANVAS_SIDE / longest;

      const viewport = page.getViewport({ scale });
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      try {
        taskRef.current?.cancel?.();
      } catch {
        /* ignore */
      }
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      // CSS box matches the raster path (height scales with zoom; width auto).
      canvas.style.height = `${78 * z}vh`;
      canvas.style.width = "auto";

      const task = page.render({ canvasContext: ctx, viewport });
      taskRef.current = task;
      await task.promise;

      // Draw search highlights (whole text items containing the query).
      const q = (hlRef.current ?? "").trim().toLowerCase();
      if (q.length >= 2) {
        try {
          const tc = await page.getTextContent();
          ctx.fillStyle = "rgba(255, 214, 0, 0.42)";
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          for (const item of tc.items as any[]) {
            if (!item.str || !item.str.toLowerCase().includes(q)) continue;
            const m = pdfjsLib.Util.transform(viewport.transform, item.transform);
            const h = Math.hypot(m[2], m[3]) || item.height * scale;
            const w = (item.width || 0) * scale;
            ctx.fillRect(m[4], m[5] - h, w, h);
          }
        } catch {
          /* highlight is best-effort */
        }
      }
    } catch {
      // RenderingCancelledException is expected on rapid zoom; ignore.
    }
  };

  return <canvas ref={canvasRef} style={{ borderRadius: 8, display: "block", maxWidth: zoom <= 1 ? "100%" : "none" }} />;
}
