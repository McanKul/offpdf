import type { EditObject } from "./types";
import { offsetObject } from "./serialize";

/** Keep selection-driven actions scoped to the page currently being edited. */
export function selectedIdsOnPage(
  objects: EditObject[],
  selectedIds: string[],
  activePageIndex: number,
): string[] {
  const ids = new Set(selectedIds);
  return objects
    .filter((object) => object.pageIndex === activePageIndex && ids.has(object.id))
    .map((object) => object.id);
}

/** Translate selected objects by PDF-space dx/dy. Other-page ids must stay put. */
export function moveSelectedRects(
  objects: EditObject[],
  selectedIds: string[],
  activePageIndex: number,
  dx: number,
  dy: number,
): EditObject[] {
  const selected = new Set(selectedIds);
  return objects.map((o) =>
    selected.has(o.id) && o.pageIndex === activePageIndex ? offsetObject(o, dx, dy) : o,
  );
}
