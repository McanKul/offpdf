import { Alert } from "@/components/ui/Alert";
import { fileSizeTier } from "@/lib/formatBytes";
import type { FileInfo } from "@/lib/types";

/** Shows the strongest large-file warning for the current selection. */
export function LargeFileWarning({ files }: { files: FileInfo[] }) {
  const tiers = files.map((f) => fileSizeTier(f.sizeBytes));
  if (tiers.includes("veryLarge")) {
    return (
      <Alert variant="warning" title="Very large file detected">
        Make sure you have enough free disk space (we recommend at least 3× the
        input size). Processing may take several minutes — keep the app open. You
        can press Cancel at any time to stop it.
      </Alert>
    );
  }
  if (tiers.includes("large")) {
    return (
      <Alert variant="info" title="This is a large PDF">
        Processing may take time. Please keep the app open until it finishes.
        Everything stays on your computer.
      </Alert>
    );
  }
  return null;
}
