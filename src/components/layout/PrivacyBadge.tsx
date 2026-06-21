import { Icon } from "@/components/ui/Icon";

/** The trust badge shown across the app: "Local only · No upload". */
export function PrivacyBadge({ compact = false }: { compact?: boolean }) {
  return (
    <span className="privacy-badge" title="All processing happens on your computer. Nothing is uploaded.">
      <Icon name="shield" size={14} />
      {compact ? <span className="sr-only">Local only</span> : "Local only · No upload"}
    </span>
  );
}
