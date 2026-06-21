/**
 * Pre-flight free-disk-space guard. `ensure(dir, requiredBytes)` resolves true
 * if there's enough space, otherwise pops a confirm modal and resolves with the
 * user's choice. Render the returned `modal` element inside the page.
 */
import { useCallback, useRef, useState, type ReactElement } from "react";
import { Modal } from "@/components/ui/Modal";
import { Button } from "@/components/ui/Button";
import { checkDiskSpace } from "@/lib/tauriCommands";
import { formatBytes } from "@/lib/formatBytes";
import type { DiskSpaceInfo } from "@/lib/types";

export function useDiskGuard(): {
  ensure: (dir: string, requiredBytes: number) => Promise<boolean>;
  modal: ReactElement;
} {
  const [info, setInfo] = useState<DiskSpaceInfo | null>(null);
  const resolverRef = useRef<((v: boolean) => void) | undefined>(undefined);

  const ensure = useCallback(async (dir: string, requiredBytes: number) => {
    let res: DiskSpaceInfo;
    try {
      res = await checkDiskSpace(dir, requiredBytes);
    } catch {
      // If we can't measure free space, don't block the user.
      return true;
    }
    if (res.sufficient) return true;
    setInfo(res);
    return new Promise<boolean>((resolve) => {
      resolverRef.current = resolve;
    });
  }, []);

  const close = useCallback((value: boolean) => {
    setInfo(null);
    resolverRef.current?.(value);
    resolverRef.current = undefined;
  }, []);

  const modal = (
    <Modal
      open={info !== null}
      onClose={() => close(false)}
      title="Not enough disk space"
      footer={
        <>
          <Button variant="secondary" onClick={() => close(false)}>
            Cancel
          </Button>
          <Button variant="danger" onClick={() => close(true)}>
            Continue anyway
          </Button>
        </>
      }
    >
      {info && (
        <>
          <p>
            This operation may need about <b>{formatBytes(info.requiredBytes)}</b> of free space,
            but only <b>{formatBytes(info.availableBytes)}</b> is available on the output drive.
          </p>
          <p className="mt-sm">
            Free up space or pick another folder. You can continue anyway, but the operation may
            fail partway through.
          </p>
        </>
      )}
    </Modal>
  );

  return { ensure, modal };
}
