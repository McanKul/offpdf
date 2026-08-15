/** SVG/CSS rotation (y-down): positive degrees = clockwise, matches `rotate(deg cx cy)`. */
export function rotateCss(
  p: { x: number; y: number },
  c: { x: number; y: number },
  degCw: number,
): { x: number; y: number } {
  const rad = (degCw * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const dx = p.x - c.x;
  const dy = p.y - c.y;
  return { x: c.x + dx * cos - dy * sin, y: c.y + dx * sin + dy * cos };
}

export function cssCenter(box: { x: number; y: number; w: number; h: number }): { x: number; y: number } {
  return { x: box.x + box.w / 2, y: box.y + box.h / 2 };
}

export function pointerAngleDeg(
  p: { x: number; y: number },
  c: { x: number; y: number },
): number {
  return (Math.atan2(p.y - c.y, p.x - c.x) * 180) / Math.PI;
}

/** Normalize to (-180, 180]. */
export function normalizeDeg(deg: number): number {
  let d = ((((deg + 180) % 360) + 360) % 360) - 180;
  if (d === -180) d = 180;
  return Math.round(d * 10) / 10;
}

export function snapDeg(deg: number, step = 15): number {
  return normalizeDeg(Math.round(deg / step) * step);
}
