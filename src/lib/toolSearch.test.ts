import { describe, expect, it } from "vitest";
import type { ToolMeta } from "./tools";
import { searchTools } from "./toolSearch";

const tools: ToolMeta[] = [
  {
    id: "merge",
    name: "Merge PDFs",
    description: "Combine several PDFs into one file.",
    longDescription: "Join documents into one PDF.",
    icon: "merge",
    path: "/tools/merge",
    category: "Organize",
    aliases: ["join documents"],
  },
  {
    id: "compress",
    name: "Compress PDF",
    description: "Reduce the file size for smaller storage.",
    longDescription: "Optimize a PDF for smaller storage.",
    icon: "compress",
    path: "/tools/compress",
    category: "Optimize & secure",
  },
  {
    id: "officeToPdf",
    name: "Office / HTML to PDF",
    description: "Convert office documents and HTML files to PDF.",
    longDescription: "Convert documents locally.",
    icon: "fileText",
    path: "/tools/office-to-pdf",
    category: "Convert",
    aliases: ["docx", "word files"],
  },
];

describe("searchTools", () => {
  it("returns all tools for an empty query", () => {
    expect(searchTools(tools, "")).toEqual(tools);
  });

  it("returns category tools for a whitespace-only query", () => {
    expect(searchTools(tools, "   ", "Convert")).toEqual([tools[2]]);
  });

  it("matches case-insensitively", () => {
    expect(searchTools(tools, "cOmPrEsS")).toEqual([tools[1]]);
  });

  it("matches partial names", () => {
    expect(searchTools(tools, "merge pdf")).toEqual([tools[0]]);
  });

  it("matches descriptions", () => {
    expect(searchTools(tools, "smaller storage")).toEqual([tools[1]]);
  });

  it("matches category text", () => {
    expect(searchTools(tools, "secure")).toEqual([tools[1]]);
  });

  it("matches explicit aliases", () => {
    expect(searchTools(tools, "DOCX")).toEqual([tools[2]]);
  });

  it("applies category and query filters together", () => {
    expect(searchTools(tools, "pdf", "Convert")).toEqual([tools[2]]);
    expect(searchTools(tools, "pdf", "Organize")).toEqual([tools[0]]);
  });

  it("returns no results when nothing matches", () => {
    expect(searchTools(tools, "spreadsheet")).toEqual([]);
  });

  it("preserves input order", () => {
    const reversed = [tools[2], tools[0], tools[1]];
    expect(searchTools(reversed, "pdf")).toEqual(reversed);
  });

  it("does not mutate the source array or tool objects", () => {
    const source = tools.map((tool) => ({ ...tool, aliases: tool.aliases ? [...tool.aliases] : undefined }));
    const sourceSnapshot = structuredClone(source);

    const result = searchTools(source, "pdf");

    expect(result).not.toBe(source);
    expect(source).toEqual(sourceSnapshot);
    expect(result[0]).toBe(source[0]);
  });
});
