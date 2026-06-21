// Generates a 1024x1024 PNG app-icon source with no external deps (Node zlib).
// Re-run after changing the source art: npm run tauri icon scripts/icon-src.png
import { deflateSync } from "node:zlib";
import { writeFileSync } from "node:fs";

const SIZE = 1024;

// --- simple RGBA canvas ---
const px = Buffer.alloc(SIZE * SIZE * 4);
function set(x, y, r, g, b, a = 255) {
  if (x < 0 || y < 0 || x >= SIZE || y >= SIZE) return;
  const i = (y * SIZE + x) * 4;
  px[i] = r; px[i + 1] = g; px[i + 2] = b; px[i + 3] = a;
}

// brand gradient #2563eb -> #1e40af, rounded square, transparent outside
const radius = 180;
function inRounded(x, y) {
  const m = 0; // full-bleed tile
  const minX = m, minY = m, maxX = SIZE - m, maxY = SIZE - m;
  if (x < minX || y < minY || x >= maxX || y >= maxY) return false;
  const cx = Math.min(Math.max(x, minX + radius), maxX - radius);
  const cy = Math.min(Math.max(y, minY + radius), maxY - radius);
  const dx = x - cx, dy = y - cy;
  return dx * dx + dy * dy <= radius * radius || (x >= minX + radius && x < maxX - radius) || (y >= minY + radius && y < maxY - radius);
}

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    if (!inRounded(x, y)) { set(x, y, 0, 0, 0, 0); continue; }
    const t = y / SIZE;
    const r = Math.round(0x25 + (0x1e - 0x25) * t);
    const g = Math.round(0x63 + (0x40 - 0x63) * t);
    const b = Math.round(0xeb + (0xaf - 0xeb) * t);
    set(x, y, r, g, b, 255);
  }
}

// white document glyph in the center
const docX = 360, docY = 300, docW = 304, docH = 424, fold = 96;
function inDoc(x, y) {
  if (x < docX || x >= docX + docW || y < docY || y >= docY + docH) return false;
  // cut the top-right fold
  const fx = docX + docW - fold, fy = docY + fold;
  if (x >= fx && y < fy && (x - fx) + (fy - y) > fold) return false;
  return true;
}
for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    if (inDoc(x, y)) set(x, y, 255, 255, 255, 255);
  }
}
// three text lines on the document
function bar(yy, x0, x1) {
  for (let y = yy; y < yy + 26; y++) for (let x = x0; x < x1; x++) set(x, y, 0x25, 0x63, 0xeb, 255);
}
bar(470, 410, 614);
bar(540, 410, 614);
bar(610, 410, 560);

// --- encode PNG ---
function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return ~c >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length, 0);
  const t = Buffer.from(type, "ascii");
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(Buffer.concat([t, data])), 0);
  return Buffer.concat([len, t, data, crc]);
}

const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0); ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8; ihdr[9] = 6; ihdr[10] = 0; ihdr[11] = 0; ihdr[12] = 0;

// add filter byte (0) per scanline
const raw = Buffer.alloc(SIZE * (SIZE * 4 + 1));
for (let y = 0; y < SIZE; y++) {
  raw[y * (SIZE * 4 + 1)] = 0;
  px.copy(raw, y * (SIZE * 4 + 1) + 1, y * SIZE * 4, (y + 1) * SIZE * 4);
}
const idat = deflateSync(raw, { level: 9 });
const png = Buffer.concat([sig, chunk("IHDR", ihdr), chunk("IDAT", idat), chunk("IEND", Buffer.alloc(0))]);

const out = new URL("./icon-src.png", import.meta.url);
writeFileSync(out, png);
console.log("Wrote", out.pathname, png.length, "bytes");
