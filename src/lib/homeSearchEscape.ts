export function homeSearchEscapeAction(e: { key: string }): { clear: boolean; blur: boolean } {
  if (e.key === "Escape") return { clear: true, blur: true };
  return { clear: false, blur: false };
}
