import type { ToolCategory, ToolMeta } from "./tools";

/** Returns tools matching the query and optional category without mutating the registry. */
export function searchTools(
  tools: readonly ToolMeta[],
  query: string,
  category: ToolCategory | "All" = "All",
): ToolMeta[] {
  const normalizedQuery = query.trim().toLowerCase();

  return tools.filter((tool) => {
    if (category !== "All" && tool.category !== category) return false;
    if (normalizedQuery === "") return true;

    const searchableFields = [tool.name, tool.description, tool.category, ...(tool.aliases ?? [])];
    return searchableFields.some((field) => field.toLowerCase().includes(normalizedQuery));
  });
}
