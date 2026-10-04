/**
 * DTO contract (T7 side of DTO-01…03): the key sets and value kinds (string, number, boolean,
 * object, arrays of them, nullability) of the TS mirrors in `src/lib/types.ts` equal the
 * samples Rust serialises into
 * `__fixtures__/text-edit-dto-contract.json` (T5), including the verdict's
 * reason/face/field/detail/caretOffsets; reason and problem strings are
 * SCREAMING_SNAKE codes from `text-reasons.json`; the TS `sourceText` export
 * object and `TextEditIn` carry exactly the fields Rust reads.
 *
 * Samples are found by their distinguishing keys anywhere in the file, so the
 * test does not depend on how T5 groups them; each DTO must appear at least once.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type {
  SourceTextStyle,
  TextEditIn,
  TextEditVerdict,
  TextFaceOption,
  TextFont,
  TextPreview,
  TextRect,
  TextRun,
  TextRunMetrics,
  TextRunStyle,
  TextSourceInfo,
  TextVec,
  TextWarning,
  PageText,
} from "../types";
import { makeSourceTextObject } from "./editReducer";
import { toExportDocument } from "./serialize";

declare module "node:fs" {
  export function existsSync(path: string): boolean;
}

const FIXTURE = join(process.cwd(), "src/lib/editor/__fixtures__/text-edit-dto-contract.json");
const REASONS = JSON.parse(readFileSync(join(process.cwd(), "src/lib/editor/text-reasons.json"), "utf8")) as Record<
  string,
  string[] | number
>;

/** Every key of `T`, checked by the compiler: a missing or extra key fails `npm run typecheck`. */
function keys<T>(shape: Record<keyof Required<T>, 0>): string[] {
  return Object.keys(shape).sort();
}

/**
 * The JSON kind of a TS field type (review-T5 M5: value types, not only key names): "string",
 * "number", "boolean" or "object"; "K[]" for an array of K; a trailing "?" when null is allowed.
 * `Kinds<T>` derives the kind of every field of `T`, so a schema below that disagrees with the
 * TS type fails `npm run typecheck`, and a fixture value that disagrees with the schema fails
 * the test.
 */
type Scalar<V> = V extends string
  ? "string"
  : V extends number
    ? "number"
    : V extends boolean
      ? "boolean"
      : V extends object
        ? "object"
        : never;
type Base<V> = V extends readonly (infer E)[] ? `${Scalar<E>}[]` : Scalar<V>;
type Kind<V> = null extends V ? `${Base<NonNullable<V>>}?` : Base<V>;
type Kinds<T> = { [K in keyof Required<T>]: Kind<Required<T>[K]> };

function kinds<T>(k: Kinds<T>): Record<string, string> {
  return k as Record<string, string>;
}

function kindOf(v: unknown): string {
  if (v === null) return "null";
  if (Array.isArray(v)) return "array";
  return typeof v;
}

/** Whether a serialised value has the kind the TS type gives its field. */
function hasKind(kind: string, v: unknown): boolean {
  const nullable = kind.endsWith("?");
  const k = nullable ? kind.slice(0, -1) : kind;
  if (v === null) return nullable;
  if (!k.endsWith("[]")) return kindOf(v) === k;
  return Array.isArray(v) && v.every((x) => kindOf(x) === k.slice(0, -2));
}

type Obj = Record<string, unknown>;
const has = (o: Obj, ...k: string[]) => k.every((key) => Object.prototype.hasOwnProperty.call(o, key));

const DTOS: { name: string; kinds: Record<string, string>; is: (o: Obj) => boolean }[] = [
  {
    name: "TextSourceDto",
    kinds: kinds<TextSourceInfo>({ fingerprint: "string", pageCount: "number", warnings: "string[]" }),
    is: (o) => has(o, "pageCount", "warnings"),
  },
  {
    name: "PageTextDto",
    kinds: kinds<PageText>({
      fingerprint: "string", pageIndex: "number", pageReason: "string?", runs: "object[]", fonts: "object[]",
    }),
    is: (o) => has(o, "runs", "fonts"),
  },
  {
    name: "TextRunDto",
    kinds: kinds<TextRun>({
      id: "string", order: "number", line: "number", text: "string", rect: "object", origin: "object",
      dir: "object", ascent: "number", descent: "number", caretOffsets: "number[]", editable: "boolean",
      reason: "string?", metrics: "object?", style: "object?", substituted: "boolean",
    }),
    is: (o) => has(o, "caretOffsets", "editable"),
  },
  {
    name: "RunMetricsDto",
    kinds: kinds<TextRunMetrics>({
      surface: "string[]", tfSize: "number", effectiveSize: "number", charSpacing: "number",
      wordSpacing: "number", hScale: "number", textToUser: "number", letterSpacingPt: "number",
      spaceMode: "string", kernSpace: "number", originalWidth: "number", visibleExtent: "number",
      nextObstacle: "number?",
    }),
    is: (o) => has(o, "tfSize"),
  },
  {
    name: "RunStyleDto",
    kinds: kinds<TextRunStyle>({
      fill: "string?", sizeChangeable: "boolean", colourChangeable: "boolean", face: "string", faces: "object",
    }),
    is: (o) => has(o, "faces", "sizeChangeable"),
  },
  {
    name: "FacesDto",
    kinds: kinds<TextRunStyle["faces"]>({ regular: "object", bold: "object", italic: "object", boldItalic: "object" }),
    is: (o) => has(o, "regular", "boldItalic"),
  },
  {
    name: "FaceOptionDto",
    kinds: kinds<TextFaceOption>({ available: "boolean", surface: "string[]" }),
    is: (o) => has(o, "available", "surface"),
  },
  {
    name: "TextFontDto",
    kinds: kinds<TextFont>({
      key: "string", displayName: "string", familyHint: "string", embedded: "boolean", subset: "boolean",
      alphabet: "string", widths: "number[]", wordSpace: "boolean",
    }),
    is: (o) => has(o, "alphabet", "widths"),
  },
  {
    name: "TextPreviewDto",
    kinds: kinds<TextPreview>({ pagePdf: "string?", verdicts: "object[]", pageProblem: "object?", warnings: "object[]" }),
    is: (o) => has(o, "verdicts"),
  },
  {
    name: "EditVerdictDto",
    kinds: kinds<TextEditVerdict>({
      runId: "string", ok: "boolean", code: "string?", chars: "string[]", reason: "string?", face: "string?",
      field: "string?", detail: "string?", deltaPt: "number", newRect: "object?", caretOffsets: "number[]?",
    }),
    is: (o) => has(o, "ok", "runId"),
  },
  {
    name: "EditProblemDto",
    kinds: kinds<NonNullable<TextPreview["pageProblem"]>>({ code: "string", detail: "string?" }),
    is: (o) => has(o, "code", "detail") && !has(o, "runId") && !has(o, "ok"),
  },
  {
    name: "TextWarningDto",
    kinds: kinds<TextWarning>({ runId: "string", code: "string", detail: "string?" }),
    is: (o) => has(o, "runId", "code") && !has(o, "ok"),
  },
  {
    name: "RectDto",
    kinds: kinds<TextRect>({ x: "number", y: "number", w: "number", h: "number" }),
    is: (o) => has(o, "x", "y", "w", "h"),
  },
  { name: "VecDto", kinds: kinds<TextVec>({ x: "number", y: "number" }), is: (o) => has(o, "x", "y") && !has(o, "w") },
  {
    name: "TextEditIn",
    kinds: kinds<TextEditIn>({ runId: "string", originalText: "string", text: "string", style: "object" }),
    is: (o) => has(o, "originalText", "runId") && !has(o, "kind"),
  },
];

const STYLE_KEYS = keys<SourceTextStyle>({ sizePt: 0, face: 0, fill: 0, letterSpacingPt: 0 });

function objectsIn(value: unknown, out: Obj[] = []): Obj[] {
  if (Array.isArray(value)) value.forEach((v) => objectsIn(v, out));
  else if (value && typeof value === "object") {
    out.push(value as Obj);
    Object.values(value as Obj).forEach((v) => objectsIn(v, out));
  }
  return out;
}

function list(name: string): string[] {
  const value = REASONS[name];
  return Array.isArray(value) ? value : [];
}

const fixtureExists = existsSync(FIXTURE);

describe.skipIf(!fixtureExists)("sourceTextContract: TS types == text-edit-dto-contract.json (T5)", () => {
  const all = fixtureExists ? objectsIn(JSON.parse(readFileSync(FIXTURE, "utf8"))) : [];

  for (const dto of DTOS) {
    it(`DTO ${dto.name}: every sample has exactly the TS keys, each with the TS value kind`, () => {
      const samples = all.filter(dto.is);
      expect(samples.length, `no ${dto.name} sample in the contract file`).toBeGreaterThan(0);
      for (const sample of samples) {
        expect(Object.keys(sample).sort(), dto.name).toEqual(Object.keys(dto.kinds).sort());
        for (const [key, kind] of Object.entries(dto.kinds)) {
          expect(hasKind(kind, sample[key]), `${dto.name}.${key}: ${kind}, got ${JSON.stringify(sample[key])}`).toBe(true);
        }
      }
    });
  }

  it("DTO-03 reason, page reason, problem and warning codes are canonical SCREAMING_SNAKE codes", () => {
    const runOrPage = new Set([...list("run"), ...list("page")]);
    const problems = new Set(list("problem"));
    const warnings = new Set(list("warning"));
    const codes: string[] = [];
    for (const o of all) {
      for (const field of ["reason", "pageReason"]) {
        if (typeof o[field] !== "string") continue;
        expect(runOrPage.has(o[field] as string), `${field} ${String(o[field])}`).toBe(true);
        codes.push(o[field] as string);
      }
      if (typeof o.code !== "string") continue;
      // Verdicts and page problems carry problem codes; warnings carry warning codes.
      const pool = has(o, "runId") && !has(o, "ok") ? warnings : problems;
      expect(pool.has(o.code), `code ${o.code}`).toBe(true);
      codes.push(o.code);
    }
    expect(codes.length).toBeGreaterThan(0);
    for (const code of codes) expect(code).toMatch(/^[A-Z][A-Z0-9_]*$/);
  });

  it("enum-like strings use the TS spellings", () => {
    for (const o of all) {
      if (has(o, "spaceMode")) expect(["glyph", "kern"]).toContain(o.spaceMode);
      if (has(o, "familyHint")) expect(["serif", "sans", "mono"]).toContain(o.familyHint);
      if (has(o, "faces") && typeof o.face === "string") expect(["regular", "bold", "italic", "boldItalic"]).toContain(o.face);
      if (has(o, "field") && o.field !== null) expect(["size", "face", "colour"]).toContain(o.field);
    }
  });

  it("TextEditIn and the sourceText export object carry the fields Rust reads", () => {
    const style = all.filter((o) => has(o, "originalText")).map((o) => o.style as Obj);
    for (const s of style) for (const key of Object.keys(s)) expect(STYLE_KEYS).toContain(key);

    const exported = all.filter((o) => o.kind === "sourceText");
    expect(exported.length, "no sourceText export sample").toBeGreaterThan(0);
    const obj = makeSourceTextObject("id-1", 0, { x: 1, y: 2, w: 3, h: 4 }, {
      runId: "r",
      sourceFingerprint: "fp",
      sourcePageIndex: 0,
      originalText: "a",
      text: "b",
      style: { sizePt: 12, face: "bold", fill: "#000000", letterSpacingPt: 0.5 },
    });
    const ts = toExportDocument({ version: 1, objects: [obj], selectedIds: [] }).objects[0] as unknown as Obj;
    // Rust ignores `id` and `locked`, so the sample may omit them; nothing else may differ.
    const ignored = new Set(["id", "locked"]);
    for (const sample of exported) {
      const expected = Object.keys(ts).filter((k) => !ignored.has(k) || has(sample, k)).sort();
      expect(Object.keys(sample).sort()).toEqual(expected);
      for (const key of Object.keys((sample.style as Obj) ?? {})) expect(STYLE_KEYS).toContain(key);
    }
  });
});
