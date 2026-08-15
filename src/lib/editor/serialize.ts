import type { EditDocument, EditObject } from "./types";

/** Deep-clone an object so history snapshots do not share point arrays. */
export function cloneObject(o: EditObject): EditObject {
  const next = { ...o, rect: { ...o.rect } } as EditObject;
  if (next.kind === "ink") {
    next.points = next.points.map((p) => ({ x: p.x, y: p.y }));
  }
  return next;
}

/** Shift an object in PDF space (used by paste / nudge-all). */
export function offsetObject(o: EditObject, dx: number, dy: number): EditObject {
  const next = cloneObject(o);
  next.rect = { ...next.rect, x: next.rect.x + dx, y: next.rect.y + dy };
  if (next.kind === "line") {
    next.x1 += dx;
    next.y1 += dy;
    next.x2 += dx;
    next.y2 += dy;
  } else if (next.kind === "ink") {
    next.points = next.points.map((p) => ({ x: p.x + dx, y: p.y + dy }));
  }
  return next;
}

export function cloneDocument(doc: EditDocument): EditDocument {
  return {
    version: 1,
    objects: doc.objects.map(cloneObject),
    selectedIds: [...doc.selectedIds],
  };
}

/** Drop session-only fields (image preview URLs) before sending to Rust. */
export function toExportDocument(doc: EditDocument): EditDocument {
  return {
    version: 1,
    selectedIds: [],
    objects: doc.objects.map((o) => {
      if (o.kind === "image") {
        const { previewUrl: _drop, ...rest } = o;
        return rest;
      }
      return cloneObject(o);
    }),
  };
}

export function parseHexColor(color: string | undefined, fallback = { r: 0.067, g: 0.094, b: 0.153 }): {
  r: number;
  g: number;
  b: number;
} {
  if (!color) return fallback;
  const hex = toCssHex(color, "");
  if (!hex) return fallback;
  const n = parseInt(hex.slice(1), 16);
  return {
    r: ((n >> 16) & 255) / 255,
    g: ((n >> 8) & 255) / 255,
    b: (n & 255) / 255,
  };
}

/** True when a rectangle should not paint a fill (border only). */
export function isNoneFill(fill?: string): boolean {
  if (!fill) return true;
  const v = fill.trim().toLowerCase();
  return v === "none" || v === "transparent";
}

/** Normalize #rgb / #rrggbb for `<input type="color">`. Empty string if invalid. */
export function toCssHex(color: string | undefined, fallback = "#111827"): string {
  if (!color) return fallback;
  const t = color.trim();
  const m3 = /^#([0-9a-f]{3})$/i.exec(t);
  if (m3) {
    const [a, b, c] = m3[1];
    return `#${a}${a}${b}${b}${c}${c}`.toLowerCase();
  }
  const m6 = /^#([0-9a-f]{6})$/i.exec(t);
  if (m6) return `#${m6[1]}`.toLowerCase();
  return fallback;
}

export function rgbToHex(r: number, g: number, b: number): string {
  const h = (n: number) => Math.max(0, Math.min(255, Math.round(n))).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}`;
}
