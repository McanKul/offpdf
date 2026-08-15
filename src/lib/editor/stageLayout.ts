/** Stage flex justify: start when the page is wider than the stage, else center. */
export function stageJustify(
  pageWidth: number,
  stageInnerWidth: number,
): "start" | "center" {
  return pageWidth > stageInnerWidth ? "start" : "center";
}
