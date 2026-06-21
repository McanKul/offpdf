/** Which files the app accepts. PDFs load directly; images and Office documents
 * are converted to PDF on import (Office needs LibreOffice). */

export const IMAGE_RE = /\.(png|jpe?g|gif|bmp|webp|tiff?)$/i;
export const PDF_RE = /\.pdf$/i;
export const OFFICE_RE = /\.(docx?|xlsx?|pptx?|odt|ods|odp|rtf|csv|html?)$/i;
export const SUPPORTED_RE =
  /\.(pdf|png|jpe?g|gif|bmp|webp|tiff?|docx?|xlsx?|pptx?|odt|ods|odp|rtf|csv|html?)$/i;

export function isImagePath(path: string): boolean {
  return IMAGE_RE.test(path);
}
export function isPdfPath(path: string): boolean {
  return PDF_RE.test(path);
}
export function isOfficePath(path: string): boolean {
  return OFFICE_RE.test(path);
}
export function isSupportedPath(path: string): boolean {
  return SUPPORTED_RE.test(path);
}
