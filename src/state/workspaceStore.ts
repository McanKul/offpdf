/**
 * Shared document workspace. Files loaded in any tool live here, so they persist
 * across tools — load once, then delete / reorder / rotate / compress the same
 * document(s) without re-picking. A finished job's output can be added back here
 * to keep editing (chaining tools, e.g. Merge → Reorder).
 *
 * In-memory only (module-level): persists across route navigation within a
 * session, but not stored to disk (paths could become stale).
 */
import { create } from "zustand";
import { getFileInfo, imageToPdf, officeToPdf } from "@/lib/tauriCommands";
import { isImagePath, isOfficePath, SUPPORTED_RE } from "@/lib/fileTypes";
import { toAppError, type FileInfo, type WorkspaceFile } from "@/lib/types";

let uidSeq = 0;
let imageConversionQueue: Promise<void> = Promise.resolve();
let activeAddOperations = 0;

function imageToPdfSerial(path: string): Promise<string> {
  const conversion = imageConversionQueue.then(() => imageToPdf(path));
  imageConversionQueue = conversion.then(
    () => undefined,
    () => undefined,
  );
  return conversion;
}

export interface AddResult {
  added: number;
  /** Names of files that were not valid PDFs. */
  invalid: string[];
  /** True if some dropped/picked items were not supported at all. */
  notPdf: boolean;
  /** Conversion/load errors, e.g. "report.docx: LibreOffice not found". */
  errors: string[];
}

function baseName(p: string): string {
  return p.split(/[\\/]/).pop() || p;
}

interface WorkspaceState {
  files: WorkspaceFile[];
  /** Index of the file single-file tools act on. */
  activeIndex: number;
  loading: boolean;

  addPaths: (paths: string[]) => Promise<AddResult>;
  removeAt: (index: number) => void;
  clear: () => void;
  reorder: (from: number, to: number) => void;
  setActive: (index: number) => void;
}

export const useWorkspace = create<WorkspaceState>((set, get) => ({
  files: [],
  activeIndex: 0,
  loading: false,

  addPaths: async (paths) => {
    const supported = paths.filter((p) => SUPPORTED_RE.test(p));
    // True if some dropped/picked items were not a supported type at all.
    const notPdf = supported.length < paths.length;
    if (supported.length === 0) return { added: 0, invalid: [], notPdf, errors: [] };

    activeAddOperations += 1;
    set({ loading: true });
    const errors: string[] = [];
    try {
      const existing = new Set(get().files.map((f) => f.path));
      const convertedImages = new Map<string, string>();
      const infos: Array<FileInfo | null> = [];

      // Image decoders can use hundreds of megabytes for a single large photo.
      // Keep conversions serial, and convert duplicate image paths only once
      // while still allowing the resulting PDF to be added more than once.
      for (const p of supported) {
        try {
          if (isImagePath(p)) {
            let pdfPath = convertedImages.get(p);
            if (!pdfPath) {
              pdfPath = await imageToPdfSerial(p);
              convertedImages.set(p, pdfPath);
            }
            infos.push(await getFileInfo(pdfPath));
          } else if (isOfficePath(p)) {
            infos.push(await getFileInfo(await officeToPdf(p)));
          } else if (existing.has(p)) {
            infos.push(null); // already loaded
          } else {
            infos.push(await getFileInfo(p));
          }
        } catch (e) {
          errors.push(`${baseName(p)}: ${toAppError(e).message}`);
          infos.push(null);
        }
      }
      const valid = infos.filter((i): i is FileInfo => !!i && i.isValidPdf);
      const invalid = infos
        .filter((i): i is FileInfo => !!i && !i.isValidPdf)
        .map((i) => i.name);

      const withUid: WorkspaceFile[] = valid.map((info) => ({ ...info, uid: `f${++uidSeq}` }));

      set((state) => {
        const wasEmpty = state.files.length === 0;
        const files = [...state.files, ...withUid];
        return {
          files,
          activeIndex: wasEmpty && files.length > 0 ? 0 : state.activeIndex,
        };
      });
      return { added: valid.length, invalid, notPdf, errors };
    } catch {
      return { added: 0, invalid: [], notPdf, errors };
    } finally {
      activeAddOperations -= 1;
      if (activeAddOperations === 0) set({ loading: false });
    }
  },

  removeAt: (index) =>
    set((state) => {
      const files = state.files.filter((_, i) => i !== index);
      let activeIndex = state.activeIndex;
      if (index < activeIndex) activeIndex -= 1;
      if (activeIndex >= files.length) activeIndex = Math.max(0, files.length - 1);
      return { files, activeIndex };
    }),

  clear: () => set({ files: [], activeIndex: 0 }),

  reorder: (from, to) =>
    set((state) => {
      if (
        from === to ||
        from < 0 ||
        to < 0 ||
        from >= state.files.length ||
        to >= state.files.length
      ) {
        return state;
      }
      const files = [...state.files];
      const [moved] = files.splice(from, 1);
      files.splice(to, 0, moved);
      // Keep the active file active after a reorder.
      let activeIndex = state.activeIndex;
      if (state.activeIndex === from) activeIndex = to;
      else if (from < state.activeIndex && to >= state.activeIndex) activeIndex -= 1;
      else if (from > state.activeIndex && to <= state.activeIndex) activeIndex += 1;
      return { files, activeIndex };
    }),

  setActive: (index) => set({ activeIndex: index }),
}));
