export type {
  PageBox,
  PageRotation,
  PageGeometry,
  PdfRect,
  Point,
  EditObjectKind,
  EditObjectBase,
  RectObject,
  EditObject,
  EditDocument,
} from "./types";
export {
  createEmptyDocument,
  normalizePdfRect,
  isPageRotation,
  normalizePageRotation,
} from "./types";

export type { CssRect, ViewportMapping } from "./coords";
export {
  displayedSize,
  unrotatedToDisplay,
  displayToUnrotated,
  pdfToViewport,
  viewportToPdf,
  pdfRectToViewport,
  viewportRectToPdf,
  makeMapping,
} from "./coords";

export type { HistoryState, EditAction } from "./editReducer";
export {
  MAX_HISTORY,
  createHistoryState,
  editReducer,
  canUndo,
  canRedo,
  makeRectObject,
} from "./editReducer";

export type { ResizeHandle } from "./resizeRect";
export { resizePdfRect } from "./resizeRect";
