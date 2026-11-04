
import { describe, expect, it } from "vitest";
import { CATEGORIES, TOOLS } from "./tools";

describe("Tool registry", () => {
  it("should contain exactly 23 tools", () => {
    expect(TOOLS).toHaveLength(23);
  });

  it("should have unique tool IDs", () => {
    const ids = TOOLS.map((tool) => tool.id);

    expect(new Set(ids).size).toBe(ids.length);
  });

  it("should have unique tool paths", () => {
    const paths = TOOLS.map((tool) => tool.path);

    expect(new Set(paths).size).toBe(paths.length);
  });

  it("should have paths starting with /tools/", () => {
    for (const tool of TOOLS) {
      expect(tool.path.startsWith("/tools/")).toBe(true);
    }
  });

  it("should have non-empty names and descriptions", () => {
    for (const tool of TOOLS) {
      expect(tool.name.trim()).not.toBe("");
      expect(tool.description.trim()).not.toBe("");
      expect(tool.longDescription.trim()).not.toBe("");
    }
  });

  it("should only use valid categories", () => {
    for (const tool of TOOLS) {
      expect(CATEGORIES).toContain(tool.category);
    }
  });

  it("should have at least one tool in each category", () => {
    for (const category of CATEGORIES) {
      expect(
        TOOLS.some((tool) => tool.category === category)
      ).toBe(true);
    }
  });
});
