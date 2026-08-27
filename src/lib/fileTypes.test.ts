import { describe, expect, it } from "vitest";
import {
  isImagePath,
  isOfficePath,
  isPdfPath,
  isSupportedPath,
} from "./fileTypes";

const imagePaths = [
  "photo.PNG",
  "photos/photo.jpg",
  "C:\\Users\\Alex\\photo.JPEG",
  "photo.gif",
  "photo.bmp",
  "photo.webp",
  "photo.tif",
  "photo.TIFF",
  "photo.heic",
  "photo.HEIF",
];

const officePaths = [
  "report.DOC",
  "report.docx",
  "C:\\Users\\Alex\\sheet.XLS",
  "sheet.xlsx",
  "slides.PPT",
  "slides.pptx",
  "notes.odt",
  "sheet.ODS",
  "slides.odp",
  "notes.rtf",
  "data.csv",
  "page.HTM",
  "page.html",
];

describe("file type helpers", () => {
  it.each(imagePaths)("accepts image path %s", (path) => {
    expect(isImagePath(path)).toBe(true);
    expect(isSupportedPath(path)).toBe(true);
  });

  it.each(officePaths)("accepts office path %s", (path) => {
    expect(isOfficePath(path)).toBe(true);
    expect(isSupportedPath(path)).toBe(true);
  });

  it.each(["report.PDF", "docs/report.pdf", "C:\\Docs\\report.PdF"])(
    "accepts PDF path %s",
    (path) => {
      expect(isPdfPath(path)).toBe(true);
      expect(isSupportedPath(path)).toBe(true);
    },
  );

  it.each([
    "report.pdf.exe",
    "photo.heic.tmp",
    "vector.svg",
    "macro.docm",
    "README",
    "archive.tar.gz",
  ])("rejects unsupported or near-miss path %s", (path) => {
    expect(isImagePath(path)).toBe(false);
    expect(isPdfPath(path)).toBe(false);
    expect(isOfficePath(path)).toBe(false);
    expect(isSupportedPath(path)).toBe(false);
  });
});
