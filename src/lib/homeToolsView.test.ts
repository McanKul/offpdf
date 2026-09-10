import { describe, expect, it } from "vitest";
import { homeToolsView } from "./homeToolsView";
import { searchTools } from "./toolSearch";
import { TOOLS } from "./tools";

const FEATURED_IDS = ["merge", "compress", "reorder"] as const;

function ids(tools: { id: string }[]) {
  return tools.map((tool) => tool.id);
}

function featuredFromRegistry() {
  return FEATURED_IDS.map((id) => {
    const tool = TOOLS.find((entry) => entry.id === id);
    if (!tool) throw new Error(`TOOLS is missing featured id ${id}`);
    return tool;
  });
}

describe("homeToolsView", () => {
  it("HS-VIEW-EMPTY-ALL: empty/whitespace + All → featured merge/compress/reorder, grid excludes those ids", () => {
    const featured = featuredFromRegistry();
    const rest = TOOLS.filter((tool) => !FEATURED_IDS.includes(tool.id as (typeof FEATURED_IDS)[number]));

    for (const query of ["", "   "]) {
      const view = homeToolsView({ tools: TOOLS, query, category: "All" });
      expect(ids(view.featured)).toEqual([...FEATURED_IDS]);
      expect(view.featured).toEqual(featured);
      expect(ids(view.grid).some((id) => FEATURED_IDS.includes(id as (typeof FEATURED_IDS)[number]))).toBe(false);
      expect(view.grid).toEqual(rest);
      expect(view.noResults).toBe(false);
    }
  });

  it('HS-VIEW-EMPTY-CATEGORY: empty + "Convert" → featured [], grid equals searchTools Convert', () => {
    const view = homeToolsView({ tools: TOOLS, query: "", category: "Convert" });
    expect(view.featured).toEqual([]);
    expect(view.grid).toEqual(searchTools(TOOLS, "", "Convert"));
    expect(view.noResults).toBe(false);
  });

  it('HS-VIEW-QUERY: query "docx" + All → featured [], grid equals searchTools(docx) and includes officeToPdf', () => {
    const view = homeToolsView({ tools: TOOLS, query: "docx", category: "All" });
    expect(view.featured).toEqual([]);
    expect(view.grid).toEqual(searchTools(TOOLS, "docx"));
    expect(ids(view.grid)).toContain("officeToPdf");
    expect(view.noResults).toBe(false);
  });

  it("HS-VIEW-QUERY-HIDES-FEATURED: non-empty query matching a featured tool hides the strip but keeps the id in the grid", () => {
    const view = homeToolsView({ tools: TOOLS, query: "merge", category: "All" });
    expect(view.featured).toEqual([]);
    expect(ids(view.grid)).toContain("merge");
    expect(view.grid).toEqual(searchTools(TOOLS, "merge"));
    expect(view.noResults).toBe(false);
  });

  it('HS-VIEW-INTERSECT: query "pdf" + Organize → featured [], grid equals searchTools intersection', () => {
    const view = homeToolsView({ tools: TOOLS, query: "pdf", category: "Organize" });
    expect(view.featured).toEqual([]);
    expect(view.grid).toEqual(searchTools(TOOLS, "pdf", "Organize"));
    expect(view.noResults).toBe(false);
  });

  it("HS-VIEW-NONE: nonsense query → grid [], noResults true", () => {
    const view = homeToolsView({ tools: TOOLS, query: "zzzxxyyzz", category: "All" });
    expect(view.grid).toEqual([]);
    expect(view.featured).toEqual([]);
    expect(view.noResults).toBe(true);
  });
});
