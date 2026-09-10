export function isHomeSearchFocusHotkey(e: {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
}): boolean {
  return (e.key === "k" || e.key === "K") && (e.ctrlKey || e.metaKey) && !e.altKey;
}
