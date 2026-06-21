/**
 * Progress bar. Pass a `value` (0–100) for determinate progress, or omit it
 * (undefined/null) for an indeterminate animated bar — used when the PDF engine
 * doesn't report granular progress.
 */
export function Progress({ value }: { value?: number | null }) {
  const indeterminate = value === undefined || value === null;
  const pct = indeterminate ? 0 : Math.max(0, Math.min(100, value));
  return (
    <div
      className={`progress ${indeterminate ? "progress--indeterminate" : ""}`}
      role="progressbar"
      aria-valuenow={indeterminate ? undefined : pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div className="progress__bar" style={indeterminate ? undefined : { width: `${pct}%` }} />
    </div>
  );
}
