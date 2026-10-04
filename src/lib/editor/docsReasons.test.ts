/**
 * `docs/EDIT_TEXT.md` stays in step with the reason codes and the UI copy (SPEC §E.8, §F).
 *
 * - Every code in `text-reasons.json` is named in the doc (as `CODE`).
 * - User-facing sentences sit inside `<!-- ui-copy:start -->` … `<!-- ui-copy:end -->` blocks.
 *   The forbidden-word scan runs only inside those blocks, so the rest of the doc can name the
 *   technique Edit text rejects.
 * - Every table inside a block is checked cell by cell against `sourceTextCopy.ts`, so the doc
 *   quotes the shipped copy exactly. A table the test does not recognise fails the test.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import * as copy from "./sourceTextCopy";
import { FILE_ERROR_COPY, PAGE_REASON_COPY, PROBLEM_COPY, REASON_COPY, WARNING_COPY } from "./sourceTextCopy";
import type { ReasonCopy } from "./sourceTextCopy";

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

const DOC_PATH = "docs/EDIT_TEXT.md";
const START = "<!-- ui-copy:start -->";
const END = "<!-- ui-copy:end -->";
/** Same list as `sourceTextCopy.test.ts` (SPEC hard rule 7). */
const FORBIDDEN = [/\bcover(s|ed)?\b/i, /white-out/i, /flatten/i, /like Word/i, /we['’]ll/i, /\boverlay\b/i];
/** An empty copy value is written as an em dash in a table cell. */
const EMPTY_CELL = "—";

const doc = readFileSync(join(process.cwd(), DOC_PATH), "utf8");
const reasons: ReasonsJson = JSON.parse(readFileSync(join(process.cwd(), "src/lib/editor/text-reasons.json"), "utf8"));

interface Block {
  line: number;
  lines: string[];
}

/** The ui-copy blocks, in order; throws on a nested, unclosed or stray marker. */
function uiCopyBlocks(text: string): Block[] {
  const blocks: Block[] = [];
  let open: Block | null = null;
  text.split("\n").forEach((raw, i) => {
    const line = raw.trim();
    if (line === START) {
      if (open) throw new Error(`${DOC_PATH}:${i + 1}: ui-copy block opened twice`);
      open = { line: i + 1, lines: [] };
    } else if (line === END) {
      if (!open) throw new Error(`${DOC_PATH}:${i + 1}: ui-copy end without a start`);
      blocks.push(open);
      open = null;
    } else if (line.includes("ui-copy:")) {
      throw new Error(`${DOC_PATH}:${i + 1}: malformed ui-copy marker`);
    } else if (open) {
      (open as Block).lines.push(raw);
    }
  });
  if (open) throw new Error(`${DOC_PATH}:${(open as Block).line}: ui-copy block is never closed`);
  return blocks;
}

function cells(row: string): string[] {
  return row
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

function unquote(cell: string): string {
  const m = /^`([^`]+)`$/.exec(cell);
  return m ? m[1] : cell;
}

function copyValue(cell: string): string {
  return cell === EMPTY_CELL ? "" : cell;
}

interface Table {
  line: number;
  header: string[];
  rows: string[][];
}

/** Markdown tables inside a block (a table = consecutive lines starting with `|`). */
function tables(block: Block): Table[] {
  const out: Table[] = [];
  let current: Table | null = null;
  block.lines.forEach((raw, i) => {
    const line = raw.trim();
    if (!line.startsWith("|")) {
      current = null;
      return;
    }
    if (!current) {
      current = { line: block.line + i + 1, header: cells(line), rows: [] };
      out.push(current);
    } else if (!/^\|[\s:|-]+\|$/.test(line)) {
      current.rows.push(cells(line));
    }
  });
  return out;
}

type Kind = "reasons" | "errors" | "messages" | "keys";

const HEADERS: Record<Kind, string[]> = {
  reasons: ["Code", "Level", "When", "Short", "Title", "What OffPDF says"],
  errors: ["Code", "Title", "Message", "Suggestion"],
  messages: ["Code", "Message"],
  keys: ["Key", "Sentence"],
};

function kindOf(t: Table): Kind {
  const found = (Object.keys(HEADERS) as Kind[]).find((k) => HEADERS[k].join("|") === t.header.join("|"));
  if (!found) throw new Error(`${DOC_PATH}:${t.line}: unknown table inside a ui-copy block: ${t.header.join(" | ")}`);
  return found;
}

/** `UI.banner.mode` → the string at that path of the copy module. */
function resolveKey(path: string): unknown {
  return path.split(".").reduce<unknown>((value, key) => {
    if (value && typeof value === "object" && Object.prototype.hasOwnProperty.call(value, key)) {
      return (value as Record<string, unknown>)[key];
    }
    return undefined;
  }, copy);
}

interface Parsed {
  blocks: Block[];
  allTables: Array<{ t: Table; kind: Kind }>;
}

let parsedDoc: Parsed | null = null;

/** Parsed once, inside the tests, so a malformed marker or table fails a named test. */
function parsed(): Parsed {
  if (!parsedDoc) {
    const blocks = uiCopyBlocks(doc);
    parsedDoc = { blocks, allTables: blocks.flatMap((b) => tables(b).map((t) => ({ t, kind: kindOf(t) }))) };
  }
  return parsedDoc;
}

const rowsOf = (kind: Kind) =>
  parsed()
    .allTables.filter((x) => x.kind === kind)
    .flatMap((x) => x.t.rows.map((r) => ({ r, line: x.t.line })));

function sorted(list: string[]): string[] {
  return list.slice().sort();
}

describe("docs/EDIT_TEXT.md ↔ text-reasons.json and sourceTextCopy.ts", () => {
  it("names every code of text-reasons.json", () => {
    const lists = [reasons.run, reasons.page, reasons.image, reasons.file, reasons.problem, reasons.save, reasons.warning];
    const missing = [...new Set(lists.flat())].filter((code) => !doc.includes(`\`${code}\``));
    expect(missing).toEqual([]);
  });

  it("has well-formed ui-copy blocks whose tables are all recognised", () => {
    const { blocks, allTables } = parsed();
    expect(blocks.length).toBeGreaterThan(0);
    expect(allTables.length).toBeGreaterThan(0);
    for (const { t } of allTables) {
      for (const row of t.rows) expect(row.length, `${DOC_PATH}:${t.line} ${row.join(" | ")}`).toBe(t.header.length);
    }
  });

  it("keeps the forbidden words out of every ui-copy block", () => {
    const hits: string[] = [];
    for (const block of parsed().blocks) {
      block.lines.forEach((line, i) => {
        for (const re of FORBIDDEN) if (re.test(line)) hits.push(`${DOC_PATH}:${block.line + i + 1} ${re}: ${line.trim()}`);
      });
    }
    expect(hits).toEqual([]);
  });

  it("quotes every run and page reason exactly as the app shows it", () => {
    const rows = rowsOf("reasons");
    const seen: Record<string, string[]> = { run: [], page: [] };
    for (const { r, line } of rows) {
      const [codeCell, level, , short, title, body] = r;
      const code = unquote(codeCell);
      const where = `${DOC_PATH}:${line} ${code}`;
      expect(["run", "page"], where).toContain(level);
      const table: Record<string, ReasonCopy> = level === "run" ? REASON_COPY : PAGE_REASON_COPY;
      const expected = table[code];
      expect(expected, `${where}: not a ${level} reason`).toBeDefined();
      expect({ short, title, body }, where).toEqual({ short: expected.short, title: expected.title, body: expected.body });
      seen[level].push(code);
    }
    expect(sorted(seen.run)).toEqual(sorted(reasons.run));
    expect(sorted(seen.page)).toEqual(sorted(reasons.page));
  });

  it("quotes every Save and file error exactly as the app shows it", () => {
    const seen: string[] = [];
    for (const { r, line } of rowsOf("errors")) {
      const [codeCell, title, message, suggestion] = r;
      const code = unquote(codeCell);
      const expected = FILE_ERROR_COPY[code];
      expect(expected, `${DOC_PATH}:${line} ${code}: no FILE_ERROR_COPY row`).toBeDefined();
      expect(
        { title: copyValue(title), message: copyValue(message), suggestion: copyValue(suggestion) },
        `${DOC_PATH}:${line} ${code}`,
      ).toEqual(expected);
      seen.push(code);
    }
    expect(sorted(seen)).toEqual(sorted(Object.keys(FILE_ERROR_COPY)));
  });

  it("quotes every edit problem and warning message exactly", () => {
    const messages: Record<string, string> = { ...PROBLEM_COPY, ...WARNING_COPY };
    const seen: string[] = [];
    for (const { r, line } of rowsOf("messages")) {
      const [codeCell, message] = r;
      const code = unquote(codeCell);
      expect(messages[code], `${DOC_PATH}:${line} ${code}: not a problem or warning code`).toBeDefined();
      expect(message, `${DOC_PATH}:${line} ${code}`).toBe(messages[code]);
      seen.push(code);
    }
    expect(sorted(seen)).toEqual(sorted([...reasons.problem, ...reasons.warning]));
  });

  it("quotes every keyed UI sentence exactly", () => {
    const rows = rowsOf("keys");
    expect(rows.length).toBeGreaterThan(0);
    for (const { r, line } of rows) {
      const [keyCell, sentence] = r;
      const key = unquote(keyCell);
      const value = resolveKey(key);
      expect(typeof value, `${DOC_PATH}:${line} ${key}: not a string in sourceTextCopy.ts`).toBe("string");
      expect(sentence, `${DOC_PATH}:${line} ${key}`).toBe(value);
    }
  });
});
