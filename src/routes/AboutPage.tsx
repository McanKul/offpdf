import { Card } from "@/components/ui/Card";
import { Icon } from "@/components/ui/Icon";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { PrivacyBadge } from "@/components/layout/PrivacyBadge";
import { useToast } from "@/components/ui/Toast";
import { openPath } from "@/lib/tauriCommands";
import { toAppError } from "@/lib/types";

const CONTACT_EMAIL = "kul3562@gmail.com";

function Point({ icon, title, children }: { icon: Parameters<typeof Icon>[0]["name"]; title: string; children: React.ReactNode }) {
  return (
    <div className="row" style={{ alignItems: "flex-start", gap: 12 }}>
      <div className="page-header__icon" style={{ width: 36, height: 36, flex: "none" }}>
        <Icon name={icon} size={18} />
      </div>
      <div>
        <div style={{ fontWeight: 650 }}>{title}</div>
        <div className="muted" style={{ fontSize: 13, marginTop: 2 }}>
          {children}
        </div>
      </div>
    </div>
  );
}

export function AboutPage() {
  const { toast } = useToast();

  const sendMail = async () => {
    try {
      await openPath(`mailto:${CONTACT_EMAIL}?subject=OffPDF feedback`);
    } catch (e) {
      toast({
        title: "Couldn't open your mail app",
        description: `Email ${CONTACT_EMAIL} directly. (${toAppError(e).message})`,
        variant: "error",
      });
    }
  };

  return (
    <div className="col gap-lg">
      <div className="page-header">
        <div className="page-header__top">
          <div className="page-header__icon">
            <Icon name="info" size={22} />
          </div>
          <h1 className="page-header__title">About OffPDF</h1>
        </div>
        <p className="page-header__desc">
          A local-first desktop utility for people who work with very large PDFs: architects,
          engineers, students and professionals handling plan sets and project documents.
        </p>
      </div>

      <Card padded>
        <div className="col gap-lg">
          <Point icon="shield" title="Local-first processing">
            Every operation runs on your own machine using a bundled PDF engine. Files are read
            from disk and written back to disk. They are never sent anywhere.
          </Point>
          <Point icon="lock" title="No upload, ever">
            OffPDF makes no network requests. No cloud, no account, no telemetry. It works
            completely offline, on a plane or an air-gapped machine.
          </Point>
          <Point icon="fileText" title="Built for very large files">
            Only file paths cross between the interface and the engine, never the file bytes. That
            means multi-gigabyte plan sets can be processed without loading them into memory.
          </Point>
          <Point icon="compress" title="Lossless or targeted compression">
            Keep your text with a lossless cleanup, or set a target size and let OffPDF auto-tune
            resolution and quality to get there.
          </Point>
        </div>
      </Card>

      <Card padded>
        <div className="row gap-sm" style={{ marginBottom: 10 }}>
          <div className="section-label">Why this exists</div>
          <Badge variant="success">Free + Open source</Badge>
        </div>
        <p className="muted" style={{ fontSize: 13.5, lineHeight: 1.6 }}>
          OffPDF was started by <b>McanKul</b> after seeing how often students and professionals had
          to upload private or oversized PDFs to online tools just to do simple work. The project is
          built around a simple rule: PDF work should be fast, local, and private by default.
        </p>
        <div className="mt" style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          <div className="muted" style={{ fontSize: 13 }}>
            Have feedback or a feature request?
          </div>
          <div className="row gap-sm wrap">
            <span className="mono selectable" style={{ fontWeight: 600 }}>
              {CONTACT_EMAIL}
            </span>
            <Button variant="secondary" size="sm" onClick={sendMail} leftIcon={<Icon name="external" size={15} />}>
              Send feedback
            </Button>
          </div>
        </div>
      </Card>

      <Card padded>
        <div className="section-label" style={{ marginBottom: 6 }}>
          Our promise
        </div>
        <p className="muted">"Your PDF files never leave your computer."</p>
        <div className="mt">
          <PrivacyBadge />
        </div>
      </Card>
    </div>
  );
}
