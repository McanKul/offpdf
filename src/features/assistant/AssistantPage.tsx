import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { PrivacyBadge } from "@/components/layout/PrivacyBadge";
import { PageLightbox } from "@/components/pdf/PageLightbox";
import { WorkspaceFilePicker } from "@/components/pdf";
import { Alert } from "@/components/ui/Alert";
import { Badge } from "@/components/ui/Badge";
import { Button } from "@/components/ui/Button";
import { Icon } from "@/components/ui/Icon";
import { Modal } from "@/components/ui/Modal";
import { useToast } from "@/components/ui/Toast";
import { createTauriIngestionRuntime } from "@/lib/assistant/ingestionTauri";
import type { Citation } from "@/lib/assistant/citations";
import type { IngestionRuntime } from "@/lib/assistant/ingestion";
import {
  getAssistantModel,
  type AssistantModel,
} from "@/lib/assistant/model";
import { citationSource } from "@/lib/assistant/session";
import {
  documentOcrLangs,
  pickSearchOcrLang,
} from "@/lib/ocrLangs";
import { ocrAvailable, ocrListLangs } from "@/lib/tauriCommands";
import { useWorkspace } from "@/state/workspaceStore";
import type { FileInfo } from "@/lib/types";
import { useAssistantSession } from "./useAssistantSession";
import {
  clearSessionFacts,
  clearSessionKeeps,
  clearSessionSummary,
  modelStatusView,
} from "./assistantView";
import { AssistantSessionView } from "./AssistantSessionView";

export interface AssistantPageProps {
  /** A runtime/model adapter; tests and a future setup flow can inject it. */
  readonly model?: AssistantModel;
  readonly runtime?: IngestionRuntime;
}

interface OcrOptionsState {
  readonly loading: boolean;
  /** True once a check has completed (successfully or not). */
  readonly checked: boolean;
  readonly available: boolean;
  readonly languages: readonly string[];
  readonly error: string | null;
}

const INITIAL_OCR_OPTIONS: OcrOptionsState = {
  loading: false,
  checked: false,
  available: false,
  languages: [],
  error: null,
};

/** Load installed OCR packs only after the user explicitly enables OCR. */
function useOcrOptions(enabled: boolean, modelReady: boolean): OcrOptionsState {
  const [options, setOptions] = useState<OcrOptionsState>(INITIAL_OCR_OPTIONS);

  useEffect(() => {
    if (!enabled || !modelReady) return;
    let active = true;
    setOptions((current) => ({
      ...current,
      loading: true,
      checked: false,
      error: null,
    }));
    Promise.all([ocrAvailable(), ocrListLangs()])
      .then(([available, installed]) => {
        if (!active) return;
        const languages = documentOcrLangs(installed);
        setOptions({
          loading: false,
          checked: true,
          available: available && languages.length > 0,
          languages,
          error: null,
        });
      })
      .catch(() => {
        if (!active) return;
        setOptions({
          loading: false,
          checked: true,
          available: false,
          languages: [],
          error: "OCR availability could not be checked on this device.",
        });
      });
    return () => {
      active = false;
    };
  }, [enabled, modelReady]);

  return options;
}

function viewerFile(
  source: { readonly path: string; readonly name: string; readonly pageCount: number } | undefined,
): FileInfo | undefined {
  if (!source) return undefined;
  return {
    path: source.path,
    name: source.name,
    sizeBytes: 0,
    pageCount: Math.max(1, source.pageCount),
    isValidPdf: true,
  };
}

export function AssistantPage({
  model = getAssistantModel(),
  runtime,
}: AssistantPageProps) {
  const files = useWorkspace((state) => state.files);
  const { toast } = useToast();
  const ingestionRuntime = useMemo(
    () => runtime ?? createTauriIngestionRuntime(),
    [runtime],
  );
  const session = useAssistantSession({
    files,
    model,
    runtime: ingestionRuntime,
  });
  const [query, setQuery] = useState("");
  const [ocrEnabled, setOcrEnabled] = useState(false);
  const [ocrLanguage, setOcrLanguage] = useState("");
  const [clearOpen, setClearOpen] = useState(false);
  const [viewer, setViewer] = useState<{ sourceId: string; page: number } | null>(
    null,
  );
  const ocr = useOcrOptions(ocrEnabled, session.modelReady);
  const ocrChecking = ocrEnabled && session.modelReady && !ocr.checked;
  const modelView = modelStatusView(model.status);

  useEffect(() => {
    if (!ocrEnabled || ocr.languages.length === 0) return;
    if (ocrLanguage && ocr.languages.includes(ocrLanguage)) return;
    const preferred = pickSearchOcrLang([...ocr.languages]).split("+")[0];
    setOcrLanguage(preferred || ocr.languages[0]);
  }, [ocrEnabled, ocr.languages, ocrLanguage]);

  const activeSource = viewer
    ? citationSource(session.state.sources, viewer.sourceId)
    : undefined;
  const activeFile = viewerFile(activeSource);
  const pageCount = activeSource
    ? Math.max(1, activeSource.pageCount)
    : 1;

  const handleOpenCitation = (citation: Citation) => {
    const source = citationSource(session.state.sources, citation.sourceId);
    if (!source) {
      toast({
        title: "That source is no longer in the session",
        description: "Process the documents again to refresh the citation.",
        variant: "error",
      });
      return;
    }
    setViewer({ sourceId: citation.sourceId, page: citation.page });
  };

  const handleClear = () => {
    session.clear();
    setViewer(null);
    setQuery("");
    setClearOpen(false);
    toast({
      title: "Assistant session cleared",
      description: "Extracted text, citations and history were released.",
      variant: "success",
    });
  };

  return (
    <div className="tool-page assistant-page">
      <div className="page-header">
        <Link to="/" className="page-header__back">
          <Icon name="home" size={14} />
          All tools
        </Link>
        <div className="page-header__top">
          <div className="page-header__icon">
            <Icon name="sparkles" size={22} />
          </div>
          <h1 className="page-header__title">Private assistant</h1>
          <Badge variant={modelView.badgeVariant}>{modelView.badgeLabel}</Badge>
        </div>
        <p className="page-header__desc">
          Ask questions and summarize PDFs already in your workspace. Answers
          stay on this device, and every citation opens the page it came from.
          The assistant is optional; the rest of OffPDF works without it.
        </p>
        <div className="assistant-page__privacy">
          <PrivacyBadge />
          <span className="muted">
            Text and questions are never uploaded. Session data stays in memory
            only.
          </span>
        </div>
      </div>

      <AssistantSessionView
        modelStatus={model.status}
        session={session.state}
        files={files}
        documents={<WorkspaceFilePicker selectable={false} />}
        ocrEnabled={ocrEnabled}
        ocrLanguage={ocrLanguage}
        ocrLoading={ocr.loading || ocrChecking}
        ocrAvailable={ocr.available}
        ocrLanguages={ocr.languages}
        ocrError={ocr.error}
        query={query}
        onQueryChange={setQuery}
        onOcrEnabledChange={setOcrEnabled}
        onOcrLanguageChange={setOcrLanguage}
        onProcess={() => {
          void session.processDocuments({ ocrEnabled, ocrLanguage });
        }}
        onAsk={() => {
          void session.ask(query);
        }}
        onSummarize={() => {
          void session.summarize();
        }}
        onCancel={session.cancel}
        onOpenClear={() => setClearOpen(true)}
        onOpenCitation={handleOpenCitation}
      />

      <Modal
        open={clearOpen}
        onClose={() => setClearOpen(false)}
        title="Clear assistant session?"
        footer={
          <div className="spread" style={{ width: "100%" }}>
            <Button
              variant="secondary"
              onClick={() => setClearOpen(false)}
            >
              Keep session
            </Button>
            <Button
              variant="danger"
              onClick={handleClear}
              leftIcon={<Icon name="trash" size={15} />}
            >
              Clear session
            </Button>
          </div>
        }
      >
        <p>{clearSessionSummary(session.state)}</p>
        <ul className="assistant-clear-list">
          {clearSessionFacts().map((fact) => (
            <li key={fact}>{fact}</li>
          ))}
        </ul>
        <p>This does not:</p>
        <ul className="assistant-clear-list">
          {clearSessionKeeps().map((fact) => (
            <li key={fact}>{fact}</li>
          ))}
        </ul>
        <Alert variant="info" title="About closing">
          Closing OffPDF also releases this in-memory session. No assistant
          state is saved or synced.
        </Alert>
      </Modal>

      <PageLightbox
        file={activeFile}
        page={viewer?.page ?? null}
        pageCount={pageCount}
        onChange={(page) =>
          setViewer((current) => (current ? { ...current, page } : current))
        }
        onClose={() => setViewer(null)}
      />
    </div>
  );
}
