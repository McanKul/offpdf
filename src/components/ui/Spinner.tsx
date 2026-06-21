export function Spinner({ className }: { className?: string }) {
  return <span className={["spinner", className ?? ""].join(" ")} role="status" aria-label="Loading" />;
}
