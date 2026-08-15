/**
 * Detect whether we are running inside the Tauri desktop webview.
 * Browser-only `npm run dev` has no IPC, file dialogs, or path-based drops.
 */
export function isTauriRuntime(): boolean {
  if (typeof window === "undefined") return false;
  // Tauri 2 injects __TAURI_INTERNALS__ (and often __TAURI__).
  const w = window as unknown as {
    __TAURI_INTERNALS__?: unknown;
    __TAURI__?: unknown;
    isTauri?: boolean;
  };
  return Boolean(w.__TAURI_INTERNALS__ || w.__TAURI__ || w.isTauri);
}
