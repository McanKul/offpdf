// Keep the PNG consumed by `tauri icon` in sync with the canonical launcher art.
import { copyFileSync } from "node:fs";

const source = new URL(
  "../src-tauri/icons/source/offpdf-app-icon-1024.png",
  import.meta.url,
);
const output = new URL("./icon-src.png", import.meta.url);

copyFileSync(source, output);
console.log("Wrote", output.pathname, "from", source.pathname);
