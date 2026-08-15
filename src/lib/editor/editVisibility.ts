/** Edit-card mount: canvas, explicit no-pages message, or hidden. */
export type EditCanvasVisibility = "edit" | "no-pages" | "hidden";

export function shouldShowEditCanvas(
  fileCount: number,
  refCount: number,
  inTauri: boolean,
): EditCanvasVisibility {
  if (!inTauri) return "hidden";
  if (fileCount === 0) return "hidden";
  if (refCount === 0) return "no-pages";
  return "edit";
}
