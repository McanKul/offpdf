export type {
  PageBox,
  PageRotation,
  PageGeometry,
  PdfRect,
  Point,
  EditObjectKind,
  ClosedShapeKind,
  EllipseObject,
  TriangleObject,
  StarObject,
  RoundRectObject,
  HexagonObject,
  BubbleObject,
  ArrowObject,
  ShapeStyle,
  EditObjectBase,
  RectObject,
  TextObject,
  ImageObject,
  LineObject,
  InkObject,
  LinkAction,
  LinkObject,
  MarkupAnnotFields,
  NoteObject,
  HighlightObject,
  UnderlineObject,
  StrikeoutObject,
  MarkupInkObject,
  RedactObject,
  SourceTextObject,
  TextAlign,
  EditObject,
  EditDocument,
  FormFieldKind,
  FormField,
  FormValue,
} from "./types";
export {
  createEmptyDocument,
  normalizePdfRect,
  isPageRotation,
  normalizePageRotation,
  lineBounds,
  pointsBounds,
  mapPointsToRect,
  isClosedShape,
  isClosedShapeObject,
  isMarkupObject,
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

export type { PdfBoxQuad } from "./visibleBox";
export { visiblePageBox, alignPageBox, quadToBox, boxToQuad } from "./visibleBox";
export { imageCssRect, placeImagePdfRect } from "./placeImage";

export type {
  HistoryState,
  EditAction,
  LayerDir,
  SourceTextFields,
  SetSourceTextInput,
} from "./editReducer";
export {
  MAX_HISTORY,
  createHistoryState,
  editReducer,
  canUndo,
  canRedo,
  reorderOnPage,
  makeRectObject,
  makeRedactObject,
  makeClosedShape,
  makeTextObject,
  makeImageObject,
  makeLineObject,
  makeInkObject,
  makeLinkObject,
  makeNoteObject,
  makeHighlightObject,
  makeUnderlineObject,
  makeStrikeoutObject,
  makeMarkupInkObject,
  makeSourceTextObject,
  setSourceTextAction,
} from "./editReducer";

export type { ResizeHandle } from "./resizeRect";
export { resizePdfRect, resizeCssRect } from "./resizeRect";
export {
  cssBoxFromPoints,
  constrainCssBox1to1,
  resizeCssRectLocked,
  aspectLocked,
  sizeWithAspect,
  isNearlySquare,
} from "./aspect";
export { closedShapeCssPoints, starPoints, polygonPoints, bubbleSvgPath } from "./shapes";
export { rotateCss, cssCenter, pointerAngleDeg, normalizeDeg, snapDeg } from "./rotate";

export {
  cloneDocument,
  cloneObject,
  offsetObject,
  toExportDocument,
  parseHexColor,
  isNoneFill,
  toCssHex,
  rgbToHex,
  compactSourceTextStyle,
} from "./serialize";

export type { NeighbourKey } from "./sourceText";
export {
  STYLE_EPSILON,
  EDIT_TEXT_CHARS_MAX,
  SIZE_MIN_PT,
  SIZE_MAX_PT,
  SIZE_STEP_PT,
  LETTER_SPACING_MIN_PT,
  LETTER_SPACING_MAX_PT,
  LETTER_SPACING_STEP_PT,
  TEXT_INKS,
  normaliseTyped,
  fontsByKey,
  surfaceFor,
  missingChars,
  spaceProblem,
  estimateDeltaPt,
  estimateCaretOffsets,
  normaliseStyle,
  isNoOpEdit,
  blockingVerdict,
  blockingProblem,
  overlapsNext,
  caretIndexAt,
  editedGeometry,
  problemMessage,
  readingNeighbour,
  toTextEditIn,
  editsSignature,
  textStampGeometry,
} from "./sourceText";

export {
  remapEditDocument,
  planKeyRebind,
  rebindNeedsConfirm,
  resolveViewPageIndex,
  samePageKeys,
} from "./remapPages";
export type { KeyRebindPlan } from "./remapPages";

export { stageJustify } from "./stageLayout";
export { moveSelectedRects, selectedIdsOnPage } from "./moveSelection";
export { shouldShowEditCanvas } from "./editVisibility";
export type { EditCanvasVisibility } from "./editVisibility";
export { clampPageIndex } from "./pageIndex";
export {
  emptyObjectsBlockSave,
  incompleteSourceIds,
  incompleteSourcePaths,
  shouldRewriteSourceLinks,
} from "./linkSavePolicy";
