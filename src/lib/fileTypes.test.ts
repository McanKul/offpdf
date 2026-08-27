import { describe, expect, it } from "vitest";
import {
  isImagePath,
  isOfficePath,
  isPdfPath,
  isSupportedPath,
} from "./fileTypes";

const IMAGE_EXTENSIONS = [
  "png",
  "jpg",
  "jpeg",
  "gif",
  "bmp",
  "webp",
  "tif",
  "tiff",
  "heic",
  "heif",
];

const OFFICE_EXTENSIONS = [
  "doc",
  "docx",
  "xls",
  "xlsx",
  "ppt",
  "pptx",
  "odt",
  "ods",
  "odp",
  "rtf",
  "csv",
  "htm",
  "html",
];

describe("isImagePath", () => {
  it.each(IMAGE_EXTENSIONS)("accepts .%s", (ext) => {
    expect(isImagePath(`photo.${ext}`)).toBe(true);
  });

  it("accepts mixed-case extensions", () => {
    expect(isImagePath("photo.PNG")).toBe(true);
    expect(isImagePath("photo.JpEg")).toBe(true);
    expect(isImagePath("photo.HEIF")).toBe(true);
  });

  it("accepts POSIX and Windows-style paths", () => {
    expect(isImagePath("/home/user/pictures/photo.png")).toBe(true);
    expect(isImagePath("C:\\Users\\me\\Pictures\\photo.TIFF")).toBe(true);
  });

  it("rejects PDF and Office extensions", () => {
    expect(isImagePath("report.pdf")).toBe(false);
    expect(isImagePath("report.docx")).toBe(false);
  });

  it("rejects SVG", () => {
    expect(isImagePath("drawing.svg")).toBe(false);
  });
});

describe("isPdfPath", () => {
  it("accepts .pdf in any case", () => {
    expect(isPdfPath("report.pdf")).toBe(true);
    expect(isPdfPath("report.PDF")).toBe(true);
    expect(isPdfPath("report.PdF")).toBe(true);
  });

  it("accepts POSIX and Windows-style paths", () => {
    expect(isPdfPath("/tmp/report.pdf")).toBe(true);
    expect(isPdfPath("C:\\Temp\\report.pdf")).toBe(true);
  });

  it("rejects image and Office extensions", () => {
    expect(isPdfPath("photo.png")).toBe(false);
    expect(isPdfPath("report.docx")).toBe(false);
  });
});

describe("isOfficePath", () => {
  it.each(OFFICE_EXTENSIONS)("accepts .%s", (ext) => {
    expect(isOfficePath(`document.${ext}`)).toBe(true);
  });

  it("accepts mixed-case extensions", () => {
    expect(isOfficePath("document.DOCX")).toBe(true);
    expect(isOfficePath("sheet.XlSx")).toBe(true);
  });

  it("accepts POSIX and Windows-style paths", () => {
    expect(isOfficePath("/home/user/docs/report.docx")).toBe(true);
    expect(isOfficePath("C:\\Users\\me\\Documents\\report.ODT")).toBe(true);
  });

  it("rejects the macro-enabled DOCM extension", () => {
    expect(isOfficePath("report.docm")).toBe(false);
  });

  it("rejects PDF and image extensions", () => {
    expect(isOfficePath("report.pdf")).toBe(false);
    expect(isOfficePath("photo.png")).toBe(false);
  });
});

describe("isSupportedPath", () => {
  it.each([...IMAGE_EXTENSIONS, "pdf", ...OFFICE_EXTENSIONS])(
    "is the union of the supported groups for .%s",
    (ext) => {
      const path = `file.${ext}`;
      expect(isSupportedPath(path)).toBe(
        isImagePath(path) || isPdfPath(path) || isOfficePath(path),
      );
    },
  );

  it("rejects a path with no extension", () => {
    expect(isSupportedPath("/home/user/README")).toBe(false);
  });

  it("rejects unsupported extensions", () => {
    expect(isSupportedPath("archive.zip")).toBe(false);
    expect(isSupportedPath("archive.tar.gz")).toBe(false);
    expect(isSupportedPath("drawing.svg")).toBe(false);
    expect(isSupportedPath("report.docm")).toBe(false);
  });

  it("rejects near misses where a supported extension is not the last one", () => {
    expect(isSupportedPath("report.pdf.exe")).toBe(false);
    expect(isSupportedPath("photo.heic.tmp")).toBe(false);
  });
});
