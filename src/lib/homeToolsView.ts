import { searchTools } from "./toolSearch";
import type { ToolCategory, ToolMeta } from "./tools";

const FEATURED_IDS = ["merge", "compress", "reorder"] as const;
const FEATURED_ID_SET = new Set<string>(FEATURED_IDS);

export type HomeToolsCategory = ToolCategory | "All";

export function homeToolsView({
  tools,
  query,
  category,
}: {
  tools: readonly ToolMeta[];
  query: string;
  category: HomeToolsCategory;
}): { featured: ToolMeta[]; grid: ToolMeta[]; noResults: boolean } {
  if (query.trim() !== "") {
    const grid = searchTools(tools, query, category);
    return { featured: [], grid, noResults: grid.length === 0 };
  }

  if (category === "All") {
    const featured = FEATURED_IDS.flatMap((id) => {
      const tool = tools.find((entry) => entry.id === id);
      return tool ? [tool] : [];
    });
    return {
      featured,
      grid: tools.filter((tool) => !FEATURED_ID_SET.has(tool.id)),
      noResults: false,
    };
  }

  return {
    featured: [],
    grid: searchTools(tools, query, category),
    noResults: false,
  };
}
