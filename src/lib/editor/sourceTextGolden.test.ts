/**
 * MEAS (T7 side): the frontend width estimate equals Rust's
 * `fit::estimate_delta_pt` / `estimate_caret_offsets` on every case of the
 * golden file T4 writes (`OFFPDF_UPDATE_GOLDEN=1 cargo test --lib -j 6 meas_01`).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import type { SourceTextStyle, TextFont, TextRun } from "../types";
import { estimateCaretOffsets, estimateDeltaPt, fontsByKey } from "./sourceText";

interface GoldenCase {
  name: string;
  run: TextRun;
  fonts: TextFont[];
  text: string;
  style: SourceTextStyle;
  deltaPt: number;
  caretOffsets: number[];
}

interface Golden {
  about: string;
  version: number;
  tolerance: number;
  cases: GoldenCase[];
}

const golden = JSON.parse(
  readFileSync(join(process.cwd(), "src/lib/editor/__fixtures__/text-measure-golden.json"), "utf8"),
) as Golden;

describe("sourceTextGolden: TS estimate == Rust fit (text-measure-golden.json)", () => {
  it("has the expected shape, a tolerance of at most 1e-6 and every T4 case", () => {
    expect(golden.version).toBe(1);
    expect(golden.tolerance).toBeLessThanOrEqual(1e-6);
    expect(golden.cases.map((c) => c.name)).toEqual([
      "std14-helvetica-12",
      "tz-80",
      "tc-0.5",
      "kern-space",
      "sibling-surface",
      "size-change",
      "letter-spacing",
      "face-bold",
    ]);
  });

  for (const c of golden.cases) {
    it(`MEAS ${c.name}: estimateDeltaPt and estimateCaretOffsets within ${golden.tolerance}`, () => {
      const fonts = fontsByKey(c.fonts);
      const delta = estimateDeltaPt(c.run, fonts, c.text, c.style);
      expect(Math.abs(delta - c.deltaPt), `${c.name} deltaPt ${delta} vs ${c.deltaPt}`).toBeLessThanOrEqual(golden.tolerance);

      const offsets = estimateCaretOffsets(c.run, fonts, c.text, c.style);
      expect(offsets).toHaveLength(c.caretOffsets.length);
      expect(offsets).toHaveLength(Array.from(c.text.normalize("NFC")).length + 1);
      offsets.forEach((at, i) => {
        expect(Math.abs(at - c.caretOffsets[i]), `${c.name} caret ${i}: ${at} vs ${c.caretOffsets[i]}`).toBeLessThanOrEqual(
          golden.tolerance,
        );
      });
    });
  }

  it("the unchanged run text estimates to its own caret offsets (same model on both sides)", () => {
    for (const c of golden.cases) {
      const fonts = fontsByKey(c.fonts);
      expect(estimateDeltaPt(c.run, fonts, c.run.text, {})).toBe(0);
      const own = estimateCaretOffsets(c.run, fonts, c.run.text, {});
      expect(own).toHaveLength(c.run.caretOffsets.length);
    }
  });
});
