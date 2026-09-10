import { describe, expect, it } from "vitest";
import {
  documentOcrLangs,
  joinOcrLangs,
  ocrLangLabel,
  pickSearchOcrLang,
  searchOcrEmptyToast,
} from "./ocrLangs";

describe("ocrLangLabel", () => {
  it("ocr-lang-label-known: eng/tur/deu/fra → English/Turkish/German/French", () => {
    expect(ocrLangLabel("eng")).toBe("English");
    expect(ocrLangLabel("tur")).toBe("Turkish");
    expect(ocrLangLabel("deu")).toBe("German");
    expect(ocrLangLabel("fra")).toBe("French");
  });

  it("ocr-lang-label-unknown: snum/zzz stay the raw code", () => {
    expect(ocrLangLabel("snum")).toBe("snum");
    expect(ocrLangLabel("zzz")).toBe("zzz");
  });
});

describe("joinOcrLangs", () => {
  it("ocr-join-langs-sorted: sorts by code, strips duplicates, joins with +", () => {
    expect(joinOcrLangs(["tur", "eng"])).toBe("eng+tur");
    expect(joinOcrLangs(["tur", "eng", "tur"])).toBe("eng+tur");
    expect(joinOcrLangs(["fra", "deu"])).toBe("deu+fra");
  });
});

describe("pickSearchOcrLang", () => {
  it("R-SEARCH-LANG-BREW: brew-only installed → eng, not eng+tur", () => {
    expect(pickSearchOcrLang(["eng", "osd", "snum"])).toBe("eng");
  });

  it("R-SEARCH-LANG-BOTH: eng and tur installed → eng+tur", () => {
    expect(pickSearchOcrLang(["eng", "osd", "tur"])).toBe("eng+tur");
  });

  it("R-SEARCH-LANG-EMPTY: no preferred pack → empty string", () => {
    expect(pickSearchOcrLang(["osd", "snum"])).toBe("");
  });

  it("R-SEARCH-LANG-TUR: tur without eng → tur, do not invent eng", () => {
    expect(pickSearchOcrLang(["tur", "fra"])).toBe("tur");
  });
});

describe("searchOcrEmptyToast", () => {
  it("R-SEARCH-TOAST: title is No English or Turkish pack; description names tesseract-lang and OCR tool", () => {
    const toast = searchOcrEmptyToast();
    expect(toast.title).toBe("No English or Turkish pack");
    expect(toast.title).not.toBe("Select a language");
    expect(toast.description).toBe(
      "Install tesseract-lang, or run OCR from the OCR tool to pick an installed language.",
    );
    expect(toast.description).toContain("tesseract-lang");
    expect(toast.description).toContain("OCR tool");
  });
});

describe("documentOcrLangs", () => {
  it("R-DOC-LANGS: drops osd/equ/snum, keeps others in input order", () => {
    expect(documentOcrLangs(["eng", "osd", "snum"])).toEqual(["eng"]);
    expect(documentOcrLangs(["eng", "osd", "tur"])).toEqual(["eng", "tur"]);
    expect(documentOcrLangs(["osd", "equ", "snum"])).toEqual([]);
    expect(documentOcrLangs(["deu", "osd"])).toEqual(["deu"]);
  });
});
