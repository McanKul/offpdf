/**
 * Why a line can't be changed (SPEC §D.6.3): a non-modal dialog anchored to the
 * run, focus on its first button. "Add text here" places a new text box over
 * the line (nothing is hidden); Esc or Close returns focus to the run.
 */
import { useEffect, useId, useRef } from "react";
import type { CssRect } from "@/lib/editor";
import { UI, reasonCopy } from "@/lib/editor/sourceTextCopy";
import type { TextRun } from "@/lib/types";

/** Width the popover is laid out for (CSS px); it never overflows the page sideways. */
const POPOVER_WIDTH = 300;
/** Below the run unless that leaves less than this much page under it. */
const POPOVER_MIN_ROOM = 170;
const GAP = 6;

export interface ReasonPopoverProps {
  run: TextRun;
  /** The run's box on the page (CSS px). */
  anchor: CssRect;
  pageWidth: number;
  pageHeight: number;
  onAddTextHere: () => void;
  onClose: () => void;
}

export function ReasonPopover({ run, anchor, pageWidth, pageHeight, onAddTextHere, onClose }: ReasonPopoverProps) {
  const titleId = useId();
  const firstRef = useRef<HTMLButtonElement>(null);
  const copy = reasonCopy(run.reason);

  useEffect(() => {
    firstRef.current?.focus();
  }, []);

  const below = anchor.y + anchor.h + POPOVER_MIN_ROOM <= pageHeight;
  const left = Math.max(0, Math.min(anchor.x, pageWidth - POPOVER_WIDTH));
  const style = below
    ? { left, top: anchor.y + anchor.h + GAP }
    : { left, top: anchor.y - GAP, transform: "translateY(-100%)" };

  return (
    <div
      className="st-popover"
      role="dialog"
      aria-modal="false"
      aria-labelledby={titleId}
      data-source-text=""
      style={style}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }}
    >
      <div className="st-popover__title" id={titleId}>
        {UI.popover.title}
      </div>
      <div className="st-popover__reason">{copy.title}</div>
      <p className="st-popover__body">{copy.body}</p>
      <p className="st-popover__footer">{UI.popover.footer}</p>
      <div className="st-popover__actions">
        <button ref={firstRef} type="button" className="btn btn--secondary btn--sm" onClick={onAddTextHere}>
          {UI.popover.addTextHere}
        </button>
        <button type="button" className="btn btn--ghost btn--sm" onClick={onClose}>
          {UI.popover.close}
        </button>
      </div>
    </div>
  );
}
