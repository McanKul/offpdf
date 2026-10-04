import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { TextEditProblemCode, TextFace, TextStyleField, TextWarningCode } from "../types";
import * as copy from "./sourceTextCopy";
import {
  FILE_ERROR_COPY,
  GENERIC_REASON_COPY,
  PAGE_REASON_COPY,
  PROBLEM_COPY,
  REASON_COPY,
  UI,
  WARNING_COPY,
  editedRunLabel,
  fileBanner,
  fontDescription,
  jobLabel,
  listLabel,
  problemCopy,
  reasonCopy,
  warningCopy,
  widthSentence,
} from "./sourceTextCopy";

interface ReasonsJson {
  version: number;
  run: string[];
  page: string[];
  image: string[];
  file: string[];
  problem: string[];
  save: string[];
  warning: string[];
}

const reasons: ReasonsJson = JSON.parse(
  readFileSync(join(process.cwd(), "src/lib/editor/text-reasons.json"), "utf8"),
);

const FACES: TextFace[] = ["regular", "bold", "italic", "boldItalic"];
const FIELDS: TextStyleField[] = ["size", "face", "colour"];
const ORIGINAL_UNCHANGED = "The original file was not changed.";

const FORBIDDEN = [/\bcover(s|ed)?\b/i, /white-out/i, /flatten/i, /like Word/i, /we['’]ll/i, /\boverlay\b/i];

function sorted(list: string[]): string[] {
  return list.slice().sort();
}

/** Every string reachable from a value (objects and arrays, deeply). */
function strings(value: unknown, path: string, out: Array<[string, string]>): void {
  if (typeof value === "string") out.push([path, value]);
  else if (Array.isArray(value)) value.forEach((v, i) => strings(v, `${path}[${i}]`, out));
  else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value)) strings(v, `${path}.${k}`, out);
  }
}

/** Every exported string, plus what the exported functions produce. */
function everyString(): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  strings(copy, "copy", out);
  for (const code of reasons.problem as TextEditProblemCode[]) {
    for (const face of FACES) out.push([`problem ${code} ${face}`, problemCopy(code, ["ğ", " "], { face, subset: true, name: "a.pdf" })]);
    for (const field of FIELDS) out.push([`problem ${code} ${field}`, problemCopy(code, [], { field })]);
  }
  for (const code of reasons.warning as TextWarningCode[]) out.push([`warning ${code}`, warningCopy(code, "Next")]);
  out.push(["width", widthSentence(3.24, false)], ["width", widthSentence(-1, true)], ["job", jobLabel(2, 3)]);
  return out;
}

describe("text-reasons.json ↔ copy and TS unions", () => {
  it("has a reason row for every run code, and no other keys", () => {
    expect(sorted(Object.keys(REASON_COPY))).toEqual(sorted(reasons.run));
    for (const code of reasons.run) {
      const row = reasonCopy(code);
      expect(row, code).not.toBe(GENERIC_REASON_COPY);
      expect(row.short && row.title && row.body, code).toBeTruthy();
    }
  });

  it("has a page-reason row for every page code, and no other keys", () => {
    expect(sorted(Object.keys(PAGE_REASON_COPY))).toEqual(sorted(reasons.page));
    for (const code of reasons.page) expect(reasonCopy(code), code).toBe(PAGE_REASON_COPY[code as keyof typeof PAGE_REASON_COPY]);
  });

  it("gives every image code copy (the generic row for image-only codes)", () => {
    for (const code of reasons.image) expect(reasonCopy(code).body, code).toBeTruthy();
    for (const code of ["MASKED_IMAGE", "SHARED_XOBJECT", "TRANSFORMED_IMAGE"]) {
      expect(reasonCopy(code)).toBe(GENERIC_REASON_COPY);
    }
  });

  it("has problem and warning copy for exactly the JSON codes", () => {
    expect(sorted(Object.keys(PROBLEM_COPY))).toEqual(sorted(reasons.problem));
    expect(sorted(Object.keys(WARNING_COPY))).toEqual(sorted(reasons.warning));
    for (const code of reasons.problem) expect(problemCopy(code as TextEditProblemCode), code).toBeTruthy();
    for (const code of reasons.warning) expect(warningCopy(code as TextWarningCode), code).toBeTruthy();
  });

  it("has an error row for every file and save code", () => {
    for (const code of [...reasons.file, ...reasons.save, ...reasons.problem]) {
      const row = FILE_ERROR_COPY[code];
      expect(row, code).toBeDefined();
      expect(row.title && row.message, code).toBeTruthy();
    }
  });

  it("falls back to the generic row for unknown codes", () => {
    expect(reasonCopy("NOT_A_CODE")).toBe(GENERIC_REASON_COPY);
    expect(reasonCopy(null)).toBe(GENERIC_REASON_COPY);
    expect(reasonCopy(undefined)).toBe(GENERIC_REASON_COPY);
    expect(problemCopy("TEXT_EDIT_REFUSED", [], { reason: "NOT_A_CODE" })).toBe(GENERIC_REASON_COPY.body);
  });
});

describe("wording rules", () => {
  it("never uses a forbidden word in any exported or produced string", () => {
    const all = everyString();
    expect(all.length).toBeGreaterThan(250);
    for (const [path, text] of all) {
      for (const re of FORBIDDEN) expect(re.test(text), `${path}: ${text}`).toBe(false);
    }
  });

  it("ends a suggestion with the unchanged-file sentence at most once, and only at the end", () => {
    for (const [code, row] of Object.entries(FILE_ERROR_COPY)) {
      const count = row.suggestion.split(ORIGINAL_UNCHANGED).length - 1;
      expect(count, code).toBeLessThanOrEqual(1);
      if (count === 1) expect(row.suggestion.endsWith(ORIGINAL_UNCHANGED), code).toBe(true);
    }
  });

  it("has a sentence for every face and style control", () => {
    for (const face of FACES) {
      expect(problemCopy("FACE_UNAVAILABLE", [], { face })).toMatch(/^This page has no .+ version of this font\.$/);
      expect(problemCopy("FACE_UNAVAILABLE", ["ğ", " "], { face })).toMatch(/version of this font can't draw: ğ, space$/);
    }
    expect(problemCopy("FACE_UNAVAILABLE", [], { face: "boldItalic" })).toBe(
      "This page has no bold italic version of this font.",
    );
    for (const field of FIELDS) expect(problemCopy("STYLE_UNAVAILABLE", [], { field })).toBe(UI.style[field]);
    expect(Object.keys(UI.face)).toHaveLength(8);
    expect(Object.keys(UI.style).sort()).toEqual(["colour", "face", "size"]);
  });

  it("keeps the exact copy-deck strings the UI and tests rely on", () => {
    expect(UI.banner.addText).toBe("Adds a new text box. To change words already on the page, use Edit text (E).");
    expect(UI.banner.duplicatePage).toBe(
      "This page is in the list more than once, so its text can't be changed. Remove the extra copy first.",
    );
    expect(UI.style.face).toBe(
      "This line's font is set in a way OffPDF can't change, so bold and italic aren't available.",
    );
    expect(UI.bar.letters).toBe("Letters");
    expect(UI.bar.lettersTitle).toBe("This font only includes these letters:");
    expect(UI.face.noItalic).toBe("This page has no italic version of this font.");
    expect(UI.tool.editText.title).toBe("Edit text (E): change words already in this PDF, in its own font");
  });
});

describe("composed strings", () => {
  it("formats width sentences, prefixing estimates with About", () => {
    expect(widthSentence(3.2, true)).toBe("3.2 pt wider than before.");
    expect(widthSentence(3.24, false)).toBe("About 3.2 pt wider than before.");
    expect(widthSentence(-1.05, true)).toBe("1.05 pt narrower than before.");
    expect(widthSentence(-2, false)).toBe("About 2 pt narrower than before.");
    expect(widthSentence(0.001, true)).toBe("Same width as before.");
    expect(widthSentence(0.04, false)).toBe("Same width as before.");
    expect(widthSentence(Number.NaN, false)).toBe("Same width as before.");
  });

  it("builds job labels with the text part only when there are text changes", () => {
    expect(jobLabel(1, 0)).toBe("Edit PDF · 1 object");
    expect(jobLabel(3, 2)).toBe("Edit PDF · 3 objects · 2 text changes");
    expect(jobLabel(0, 1)).toBe("Edit PDF · 0 objects · 1 text change");
  });

  it("truncates list labels at 28 characters and neighbours at 24", () => {
    expect(listLabel("Invoice 2027", 3)).toBe("Edited text: “Invoice 2027” · p3");
    expect(listLabel("a".repeat(30), 1)).toBe(`Edited text: “${"a".repeat(28)}…” · p1`);
    expect(warningCopy("NEXT_TEXT_OVERLAP", "Total")).toBe("The new text runs into “Total”.");
    expect(warningCopy("NEXT_TEXT_OVERLAP", "b".repeat(30))).toBe(`The new text runs into “${"b".repeat(23)}…”.`);
    expect(warningCopy("NEXT_TEXT_OVERLAP")).toBe(UI.overlapUnnamed);
    expect(warningCopy("PREVIEW_UNAVAILABLE")).toBe(UI.status.previewUnavailable);
  });

  it("labels an edited line with one full stop after the new text, never two (live check: “days..”)", () => {
    expect(editedRunLabel("Invoice 2027", "Invoice 2026")).toBe("Edited text: Invoice 2027. Original: Invoice 2026");
    expect(editedRunLabel("Payment is due within 30 days.", "Payment is due within 14 days.")).toBe(
      "Edited text: Payment is due within 30 days. Original: Payment is due within 14 days.",
    );
    expect(editedRunLabel("Really?", "Yes")).toBe("Edited text: Really? Original: Yes");
    expect(editedRunLabel("Wait…", "Go")).toBe("Edited text: Wait… Original: Go");
    expect(editedRunLabel("(see p. 3.)", "x")).toBe("Edited text: (see p. 3.) Original: x");
    expect(editedRunLabel("Total: 3.50 EUR", "x")).toBe("Edited text: Total: 3.50 EUR. Original: x");
    for (const text of ["days.", "Done!", "“Quoted.”", "end. "]) expect(editedRunLabel(text, "old")).not.toMatch(/[.!?…]["”)]*\s*\./u);
  });

  it("builds the file banner and font rows", () => {
    expect(fileBanner("a.pdf", { message: "M.", suggestion: "S." })).toBe("Edit text is off for “a.pdf”: M. S.");
    expect(fileBanner("a.pdf", { message: "M.", suggestion: null })).toBe("Edit text is off for “a.pdf”: M.");
    expect(fontDescription({ displayName: "Calibri", embedded: true, subset: true })).toBe("Calibri · embedded subset");
    expect(fontDescription({ displayName: "Calibri", embedded: true, subset: false })).toBe("Calibri · embedded");
    expect(fontDescription({ displayName: "Calibri", embedded: false, subset: false })).toBe(
      "Calibri · not included in the PDF",
    );
  });

  it("fills problem copy from its context", () => {
    expect(problemCopy("GLYPH_MISSING", ["ğ"])).toBe("This document's font can't draw: ğ");
    expect(problemCopy("TEXT_EDIT_REFUSED", [], { reason: "TYPE3" })).toBe(REASON_COPY.TYPE3.body);
    expect(problemCopy("STALE", [])).toContain("“the PDF” changed on disk");
    expect(problemCopy("EDIT_CONFLICT")).toBe("This line is already part of another change on this page.");
  });
});
