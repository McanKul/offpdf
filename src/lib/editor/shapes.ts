type Box = { x: number; y: number; w: number; h: number };

/** Polygon points for triangle / star / hexagon / arrow inside a CSS (y-down) box. */
export function closedShapeCssPoints(
  kind: "triangle" | "star" | "hexagon" | "arrow",
  box: Box,
): string {
  const pts = polygonPoints(kind, box, true);
  return pts.map(([px, py]) => `${px},${py}`).join(" ");
}

export function polygonPoints(
  kind: "triangle" | "star" | "hexagon" | "arrow",
  box: Box,
  yDown: boolean,
): [number, number][] {
  const { x, y, w, h } = box;
  if (kind === "triangle") {
    return yDown
      ? [
          [x + w / 2, y],
          [x, y + h],
          [x + w, y + h],
        ]
      : [
          [x + w / 2, y + h],
          [x, y],
          [x + w, y],
        ];
  }
  if (kind === "star") {
    return starPoints(x + w / 2, y + h / 2, w / 2, h / 2, yDown);
  }
  if (kind === "hexagon") {
    const pts: [number, number][] = [];
    for (let i = 0; i < 6; i++) {
      const a = (Math.PI / 3) * i - (yDown ? Math.PI / 6 : -Math.PI / 6);
      pts.push([x + w / 2 + (w / 2) * Math.cos(a), y + h / 2 + (h / 2) * Math.sin(a)]);
    }
    return pts;
  }
  // block arrow pointing +x
  const neck = 0.55;
  const shaftTop = yDown ? y + h * 0.28 : y + h * 0.72;
  const shaftBot = yDown ? y + h * 0.72 : y + h * 0.28;
  const midY = y + h / 2;
  const neckX = x + w * neck;
  return [
    [x, shaftTop],
    [neckX, shaftTop],
    [neckX, yDown ? y : y + h],
    [x + w, midY],
    [neckX, yDown ? y + h : y],
    [neckX, shaftBot],
    [x, shaftBot],
  ];
}

/** Speech bubble path in SVG (y-down). */
export function bubbleSvgPath(box: Box): string {
  const { x, y, w, h } = box;
  const tail = Math.min(h * 0.22, 28);
  const bh = Math.max(h - tail, 8);
  const r = Math.min(w, bh) * 0.16;
  const t1x = x + w * 0.18;
  const t2x = x + w * 0.08;
  const t3x = x + w * 0.36;
  return [
    `M ${x + r} ${y}`,
    `H ${x + w - r}`,
    `A ${r} ${r} 0 0 1 ${x + w} ${y + r}`,
    `V ${y + bh - r}`,
    `A ${r} ${r} 0 0 1 ${x + w - r} ${y + bh}`,
    `H ${t3x}`,
    `L ${t2x} ${y + h}`,
    `L ${t1x} ${y + bh}`,
    `H ${x + r}`,
    `A ${r} ${r} 0 0 1 ${x} ${y + bh - r}`,
    `V ${y + r}`,
    `A ${r} ${r} 0 0 1 ${x + r} ${y}`,
    "Z",
  ].join(" ");
}

/** 5-point star. `yDown` true for SVG, false for PDF overlay (y-up). */
export function starPoints(
  cx: number,
  cy: number,
  rx: number,
  ry: number,
  yDown: boolean,
): [number, number][] {
  const inner = 0.382;
  const start = yDown ? -Math.PI / 2 : Math.PI / 2;
  const pts: [number, number][] = [];
  for (let i = 0; i < 10; i++) {
    const a = start + (i * Math.PI) / 5;
    const r = i % 2 === 0 ? 1 : inner;
    pts.push([cx + rx * r * Math.cos(a), cy + ry * r * Math.sin(a)]);
  }
  return pts;
}
