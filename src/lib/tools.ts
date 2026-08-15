/** Central registry of tools — used by the sidebar, home grid, routes and each
 * feature page so labels/icons/paths/categories stay consistent in one place. */

import type { IconName } from "@/components/ui/Icon";
import type { ToolId } from "./types";

export type ToolCategory = "Organize" | "Convert" | "Optimize & secure";

export interface ToolMeta {
  id: ToolId;
  name: string;
  description: string;
  longDescription: string;
  icon: IconName;
  path: string;
  category: ToolCategory;
}

export const TOOLS: ToolMeta[] = [
  {
    id: "merge",
    name: "Merge PDFs",
    description: "Combine several PDFs into one file, in any order.",
    longDescription:
      "Select multiple PDFs, drag to set the order, and merge them into a single document. Files are read and written directly from disk — nothing is loaded into memory or uploaded.",
    icon: "merge",
    path: "/tools/merge",
    category: "Organize",
  },
  {
    id: "split",
    name: "Split PDF",
    description: "Break the document into several files — every N pages or by ranges.",
    longDescription:
      "Split the combined document into multiple files: every N pages, or by custom ranges (each range becomes its own file, e.g. 1-5, 6-10). To keep just a subset in one file, use Organize pages and delete the rest.",
    icon: "split",
    path: "/tools/split",
    category: "Organize",
  },
  {
    id: "reorder",
    name: "Organize pages",
    description: "Reorder, delete and rotate pages — all on one preview.",
    longDescription:
      "Edit the combined document directly: drag page thumbnails to reorder, ✕ to delete a page, ⟳ to rotate it, and Undo to bring pages back. Then save it all as one PDF.",
    icon: "reorder",
    path: "/tools/reorder",
    category: "Organize",
  },
  {
    id: "pageNumbers",
    name: "Page numbers",
    description: "Stamp page numbers onto every page.",
    longDescription:
      "Add page numbers to the combined document at the position you choose, optionally starting from a custom number. The text is added as a real PDF layer (no rasterizing).",
    icon: "hash",
    path: "/tools/page-numbers",
    category: "Organize",
  },
  {
    id: "watermark",
    name: "Watermark",
    description: "Stamp diagonal text (e.g. DRAFT) across every page.",
    longDescription:
      "Add a semi-transparent diagonal text watermark to every page (e.g. CONFIDENTIAL, your name). Added as a real text layer — text stays selectable.",
    icon: "droplet",
    path: "/tools/watermark",
    category: "Organize",
  },
  {
    id: "crop",
    name: "Crop pages",
    description: "Trim margins from every page (lossless).",
    longDescription:
      "Trim a percentage off each edge of every page. Lossless — it just changes the visible page area, keeping all text and vectors.",
    icon: "crop",
    path: "/tools/crop",
    category: "Organize",
  },
  {
    id: "stamp",
    name: "Stamp / Sign",
    description: "Place text (name, date, APPROVED) on a page.",
    longDescription:
      "Stamp a line of text — a typed signature, a date, initials or “APPROVED” — at any corner of a chosen page, in a colour and size you pick. Added as a real text layer (not rasterized).",
    icon: "stamp",
    path: "/tools/stamp",
    category: "Organize",
  },
  {
    id: "editPdf",
    name: "Edit PDF",
    description: "Add text, images and shapes to a PDF.",
    longDescription: "Add text, images and shapes to a PDF, then save a new copy.",
    icon: "fileText",
    path: "/tools/edit-pdf",
    category: "Organize",
  },
  {
    id: "poster",
    name: "Poster / tile print",
    description: "Split a big plan (A1/A0) into A4 sheets you can print at home.",
    longDescription:
      "Tile one large page across several smaller sheets (A4, A3 or Letter) so a big architecture plan or poster can be printed on a normal home printer and taped together. Lossless and fully vector — each sheet is a true-scale window onto the original. Print every sheet at 100% (actual size).",
    icon: "poster",
    path: "/tools/poster",
    category: "Organize",
  },
  {
    id: "nup",
    name: "N-up / Booklet",
    description: "Print 2 or 4 pages per sheet, or impose a foldable booklet.",
    longDescription:
      "Lay several pages of the combined document onto each sheet (2-up or 4-up) to save paper, or reorder them as a saddle-stitch booklet: print double-sided, fold the stack in half, and the pages read in order. Fully vector and lossless.",
    icon: "grip",
    path: "/tools/nup",
    category: "Organize",
  },
  {
    id: "compare",
    name: "Compare PDFs",
    description: "Spot differences between two PDFs, page by page.",
    longDescription:
      "Open two PDFs and step through them page by page, side by side — or switch to an overlay that paints changed areas in red. Great for checking what changed between two revisions of a plan or document.",
    icon: "compare",
    path: "/tools/compare",
    category: "Organize",
  },
  {
    id: "officeToPdf",
    name: "Office / HTML to PDF",
    description: "Convert Word, Excel, PowerPoint & HTML files to PDF.",
    longDescription:
      "Convert Office documents (Word, Excel, PowerPoint, OpenDocument) and HTML files to PDF locally using LibreOffice. Drop several at once. (You can also just drop one into any tool — it’s converted on import.)",
    icon: "fileText",
    path: "/tools/office-to-pdf",
    category: "Convert",
  },
  {
    id: "pdfToOffice",
    name: "PDF to Office",
    description: "Convert a PDF to editable Word or PowerPoint.",
    longDescription:
      "Convert the combined document to an editable Office file (Word or PowerPoint) using LibreOffice. Best effort — complex layouts may not be reproduced exactly.",
    icon: "fileText",
    path: "/tools/pdf-to-office",
    category: "Convert",
  },
  {
    id: "images",
    name: "PDF to images",
    description: "Export each page as a PNG or JPG image.",
    longDescription:
      "Turn the pages of the combined document into image files (PNG or JPG) at a chosen resolution — one image per page, saved to your output folder.",
    icon: "image",
    path: "/tools/images",
    category: "Convert",
  },
  {
    id: "ocr",
    name: "OCR (make searchable)",
    description: "Add a searchable text layer to scanned PDFs.",
    longDescription:
      "Recognise the text in a scanned PDF and add an invisible, selectable/searchable text layer over each page, using Tesseract. The page images are kept; you can then search and copy text.",
    icon: "scanText",
    path: "/tools/ocr",
    category: "Convert",
  },
  {
    id: "pdfa",
    name: "PDF/A (archive)",
    description: "Convert to the PDF/A archival format.",
    longDescription:
      "Convert the combined document to PDF/A-2b, the ISO format for long-term archiving (self-contained, embedded fonts), using LibreOffice. Complex layouts may shift slightly.",
    icon: "badge",
    path: "/tools/pdfa",
    category: "Convert",
  },
  {
    id: "compress",
    name: "Compress PDF",
    description: "Reduce file size — pick a target size, or keep text (lossless).",
    longDescription:
      "Make a PDF smaller. Enter a target size and OffPDF auto-tunes resolution and quality to get close (best for scans and large image/plan PDFs — text becomes part of the image). Or choose “Keep text” for a lossless cleanup that preserves the selectable text layer.",
    icon: "compress",
    path: "/tools/compress",
    category: "Optimize & secure",
  },
  {
    id: "protect",
    name: "Protect PDF",
    description: "Add a password and AES-256 encryption.",
    longDescription:
      "Encrypt the document with a password (AES-256). Set an open password, and optionally a separate owner password to restrict editing. Everything happens locally.",
    icon: "lock",
    path: "/tools/protect",
    category: "Optimize & secure",
  },
  {
    id: "unlock",
    name: "Unlock PDF",
    description: "Remove a password from a protected PDF (you must know it).",
    longDescription:
      "Remove the password/encryption from a PDF you can open. Pick the protected file, enter its current password, and get an unprotected copy. This removes a known password — it does not crack one.",
    icon: "unlock",
    path: "/tools/unlock",
    category: "Optimize & secure",
  },
  {
    id: "repair",
    name: "Repair PDF",
    description: "Rebuild a damaged or unreadable PDF's structure.",
    longDescription:
      "Rewrites the PDF from the ground up, fixing many broken cross-reference tables and structural issues that stop a file from opening. Page content is preserved.",
    icon: "wrench",
    path: "/tools/repair",
    category: "Optimize & secure",
  },
  {
    id: "blankPages",
    name: "Remove blank pages",
    description: "Find and remove empty pages from scanned PDFs.",
    longDescription:
      "Scan the loaded documents for blank pages (including flat scanner-gray pages), review the detected pages as thumbnails, keep any false positives, and save a cleaned copy. Detection runs locally at a tiny resolution.",
    icon: "file",
    path: "/tools/blank-pages",
    category: "Organize",
  },
  {
    id: "metadata",
    name: "Edit metadata",
    description: "View and edit the document's title, author and more.",
    longDescription:
      "Read and edit the PDF's Info metadata — Title, Author, Subject, Keywords, Creator, Producer — or clear all of it in one go (sanitize). Non-ASCII text such as Turkish is stored as UTF-16, so it survives intact.",
    icon: "info",
    path: "/tools/metadata",
    category: "Optimize & secure",
  },
  {
    id: "textExport",
    name: "PDF to text",
    description: "Export the document's text to a .txt file.",
    longDescription:
      "Extract the text of the whole document (or a page range) into a UTF-8 .txt file with the page layout preserved, using the local poppler engine. Scanned PDFs need OCR first to have any text.",
    icon: "fileText",
    path: "/tools/text-export",
    category: "Convert",
  },
];

export const CATEGORIES: ToolCategory[] = ["Organize", "Convert", "Optimize & secure"];

const BY_ID = new Map<ToolId, ToolMeta>(TOOLS.map((t) => [t.id, t]));

/** Returns the tool meta, or a safe fallback for ids no longer in the nav. */
export function getTool(id: ToolId): ToolMeta {
  return (
    BY_ID.get(id) ?? {
      id,
      name: id,
      description: "",
      longDescription: "",
      icon: "fileText",
      path: "/",
      category: "Organize",
    }
  );
}
