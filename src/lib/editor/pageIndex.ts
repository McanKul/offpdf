/** Clamp a view page index into [0, pageCount - 1] (or 0 when empty). */
export function clampPageIndex(index: number, pageCount: number): number {
  if (pageCount <= 0) return 0;
  return Math.min(Math.max(0, index), pageCount - 1);
}
