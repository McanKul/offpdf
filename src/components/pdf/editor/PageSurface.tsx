/**
 * Renders one PDF page with pdf.js and reports layout + PageGeometry for the
 * editor overlay.
 *
 * Zoom is applied as: displayScale = (fitWidth * zoom) / pageWidth.
 * `fitWidth` must be the *stage* content width (unzoomed fit), NOT the page
 * element’s current CSS width — otherwise zoom compounds and/or paint races
 * leave a blank canvas.
 */
import { useEffect, useRef, useState, type RefObject } from "react";
import { pdfjsLib, PDF_OPTS } from "@/lib/pdfjs";
import {
  normalizePageRotation,
  visiblePageBox,
  type PageGeometry,
  type PageRotation,
} from "@/lib/editor";

const MAX_CANVAS_SIDE = 8000;

export interface PageLayout {
  cssWidth: number;
  cssHeight: number;
  geometry: PageGeometry;
}

export function PageSurface({
  bytes,
  zoom,
  fitWidth,
  pageIndex,
  canvasRef: canvasRefProp,
  onLayout,
  onFail,
}: {
  bytes: Uint8Array;
  zoom: number;
  /** Unzoomed “fit to stage” width in CSS px (from the scroll stage, not the page). */
  fitWidth: number;
  pageIndex: number;
  /** Optional external ref so the editor can sample pixels (eyedropper). */
  canvasRef?: RefObject<HTMLCanvasElement | null>;
  onLayout: (layout: PageLayout) => void;
  onFail: (reason?: string) => void;
}) {
  const localCanvas = useRef<HTMLCanvasElement | null>(null);
  const setCanvas = (el: HTMLCanvasElement | null) => {
    localCanvas.current = el;
    if (canvasRefProp) {
      (canvasRefProp as { current: HTMLCanvasElement | null }).current = el;
    }
  };
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const docRef = useRef<any>(null);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const taskRef = useRef<any>(null);
  const geometryRef = useRef<PageGeometry | null>(null);
  const onLayoutRef = useRef(onLayout);
  onLayoutRef.current = onLayout;
  const onFailRef = useRef(onFail);
  onFailRef.current = onFail;
  const paintGen = useRef(0);
  const [ready, setReady] = useState(false);

  // Load single-page document once per bytes change.
  useEffect(() => {
    let cancelled = false;
    setReady(false);
    geometryRef.current = null;

    // Copy: pdf.js transfers/detaches the ArrayBuffer to the worker.
    const data = bytes.byteLength ? bytes.slice() : bytes;
    if (!data.byteLength) {
      onFailRef.current("Page data was empty.");
      return;
    }

    const loading = pdfjsLib.getDocument({ data, ...PDF_OPTS });
    loading.promise
      .then(async (doc: unknown) => {
        if (cancelled) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          (doc as any)?.destroy?.();
          return;
        }
        docRef.current = doc;
        try {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const page = await (doc as any).getPage(1);
          const rotate = normalizePageRotation(page.rotate ?? 0) as PageRotation;
          const media = (page.mediaBox ?? page.view) as number[];
          const crop = (page.cropBox ?? null) as number[] | null;
          const mediaQ: [number, number, number, number] = [
            media[0] ?? 0,
            media[1] ?? 0,
            media[2] ?? 612,
            media[3] ?? 792,
          ];
          const cropQ =
            crop && crop.length === 4
              ? ([crop[0], crop[1], crop[2], crop[3]] as [number, number, number, number])
              : null;
          const box = visiblePageBox(mediaQ, cropQ);
          if (!(box.w > 0 && box.h > 0)) {
            onFailRef.current("Page has an invalid size.");
            return;
          }
          geometryRef.current = { box, rotate, pageIndex };
          setReady(true);
        } catch (e) {
          const msg = e instanceof Error ? e.message : "Could not open the page.";
          onFailRef.current(msg);
        }
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        const msg = e instanceof Error ? e.message : "pdf.js could not open this page.";
        onFailRef.current(msg);
      });

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
    // pageIndex applied on geometry when load completes; reload only on new bytes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bytes]);

  useEffect(() => {
    if (geometryRef.current) {
      geometryRef.current = { ...geometryRef.current, pageIndex };
    }
  }, [pageIndex]);

  // Paint whenever zoom / fitWidth / ready changes. Uses a generation counter so
  // a cancelled render never leaves the canvas blank without a follow-up paint.
  useEffect(() => {
    if (!ready) return;
    const doc = docRef.current;
    const canvas = localCanvas.current;
    const geometry = geometryRef.current;
    if (!doc || !canvas || !geometry) return;

    const gen = ++paintGen.current;
    let cancelled = false;

    const paint = async () => {
      try {
        const page = await doc.getPage(1);
        if (cancelled || gen !== paintGen.current) return;

        const dpr = window.devicePixelRatio || 1;
        const baseW = Math.max(fitWidth, 280);
        const z = Math.max(zoom, 0.1);

        // pdf.js applies page /Rotate by default — do not pass rotation again.
        const baseViewport = page.getViewport({ scale: 1 });
        let scale = (baseW * z) / baseViewport.width;
        if (!Number.isFinite(scale) || scale <= 0) scale = 1;

        const cssWidth = baseViewport.width * scale;
        const cssHeight = baseViewport.height * scale;
        let renderScale = scale * dpr;
        const longest = Math.max(baseViewport.width, baseViewport.height) * renderScale;
        if (longest > MAX_CANVAS_SIDE) {
          renderScale *= MAX_CANVAS_SIDE / longest;
        }

        const viewport = page.getViewport({ scale: renderScale });
        const ctx = canvas.getContext("2d");
        if (!ctx) return;

        try {
          taskRef.current?.cancel?.();
        } catch {
          /* ignore */
        }

        // Clear then size — avoids flashing previous zoom level.
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        canvas.style.width = `${cssWidth}px`;
        canvas.style.height = `${cssHeight}px`;

        const task = page.render({ canvasContext: ctx, viewport });
        taskRef.current = task;
        await task.promise;

        if (cancelled || gen !== paintGen.current) return;

        onLayoutRef.current({
          cssWidth,
          cssHeight,
          geometry: { ...geometry, pageIndex },
        });
      } catch (e) {
        const name =
          e && typeof e === "object" && "name" in e
            ? String((e as { name: string }).name)
            : "";
        if (name === "RenderingCancelledException" || cancelled) return;
        const msg = e instanceof Error ? e.message : "Could not render this page.";
        onFailRef.current(msg);
      }
    };

    void paint();

    return () => {
      cancelled = true;
      try {
        taskRef.current?.cancel?.();
      } catch {
        /* ignore */
      }
    };
  }, [ready, zoom, fitWidth, pageIndex]);

  return <canvas ref={setCanvas} className="pdf-editor__canvas" />;
}
