/**
 * App settings, persisted to localStorage. All settings are local UI state.
 *
 * `offlineMode` is always true for the MVP — the app makes no network calls of
 * any kind. It is surfaced as a (locked) indicator in Settings.
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";

export type Theme = "light" | "dark" | "system";

interface SettingsState {
  theme: Theme;
  setTheme: (theme: Theme) => void;

  /** Remembered output folder, reused as the default for the next job. */
  lastOutputFolder: string | null;
  setLastOutputFolder: (folder: string | null) => void;

  /** Always true for MVP. The app never goes online. */
  readonly offlineMode: true;

  /** One-time dismissal of the large-file info banner. */
  largeFileNoticeDismissed: boolean;
  dismissLargeFileNotice: () => void;
}

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      theme: "system",
      setTheme: (theme) => set({ theme }),

      lastOutputFolder: null,
      setLastOutputFolder: (folder) => set({ lastOutputFolder: folder }),

      offlineMode: true,

      largeFileNoticeDismissed: false,
      dismissLargeFileNotice: () => set({ largeFileNoticeDismissed: true }),
    }),
    {
      name: "offpdf.settings",
      version: 1,
      // offlineMode is a constant; don't persist it (always re-derived as true).
      partialize: (state) => ({
        theme: state.theme,
        lastOutputFolder: state.lastOutputFolder,
        largeFileNoticeDismissed: state.largeFileNoticeDismissed,
      }),
    },
  ),
);

/** Resolve the effective theme, honoring the OS preference for "system". */
export function resolveTheme(theme: Theme): "light" | "dark" {
  if (theme === "system") {
    if (typeof window !== "undefined" && window.matchMedia) {
      return window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
    }
    return "light";
  }
  return theme;
}
