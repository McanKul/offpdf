// Live-check regression (v0.4 Edit text): the inline editor's message row sits over the line
// below the chip. It must not swallow clicks there, or clicking that line does nothing at all
// instead of trying Done (B11: "click another run → try Done"). Only the format bar takes the
// pointer. happy-dom does no hit testing, so this pins the stylesheet contract; the live
// Playwright run (scratchpad live-check) covers the behaviour.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const CSS = readFileSync(join(process.cwd(), "src/styles/source-text.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** Declarations of every rule whose selector list is exactly `selector`. */
function declarations(selector: string): string[] {
  const out: string[] = [];
  for (const m of CSS.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (m[1].trim() === selector) out.push(...m[2].split(";").map((d) => d.trim()).filter(Boolean));
  }
  return out;
}

function pointerEvents(selector: string): string | undefined {
  const d = declarations(selector).filter((x) => x.startsWith("pointer-events"));
  return d.length ? d[d.length - 1].split(":")[1].trim() : undefined;
}

describe("Edit text editor docks", () => {
  it("let the pointer through the message row", () => {
    expect(pointerEvents(".st-dock")).toBe("none");
    expect(pointerEvents(".st-msg")).toBeUndefined();
  });

  it("keep the format bar clickable inside either dock", () => {
    expect(pointerEvents(".st-dock > .st-bar")).toBe("auto");
  });
});
