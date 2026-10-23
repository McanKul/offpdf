import { renderToStaticMarkup } from "react-dom/server";
import { StaticRouter } from "react-router-dom/server";
import { describe, expect, it } from "vitest";
import { ToastProvider } from "@/components/ui/Toast";
import type { IngestionRuntime } from "@/lib/assistant/ingestion";
import type { AssistantModel } from "@/lib/assistant/model";
import { AssistantPage } from "./AssistantPage";

const RUNTIME: IngestionRuntime = {
  inspect: async () => ({ sizeBytes: 0, pageCount: 1, isValidPdf: true }),
  extractText: async () => ({ pages: [""], truncated: false }),
  ocrText: async () => ({ pages: [""], truncated: false }),
};

const READY: AssistantModel = {
  id: "fake",
  status: {
    state: "ready",
    backendId: "fake-v1",
    title: "Local model ready",
    detail: "The model runs on this device.",
  },
  generate: async () => "",
};

function renderPage(model?: AssistantModel): string {
  return renderToStaticMarkup(
    <StaticRouter location="/assistant">
      <ToastProvider>
        <AssistantPage model={model} runtime={RUNTIME} />
      </ToastProvider>
    </StaticRouter>,
  );
}

describe("AssistantPage", () => {
  it("renders the optional, local-only assistant shell without a model", () => {
    const markup = renderPage();

    expect(markup).toContain("Private assistant");
    expect(markup).toContain("Optional");
    expect(markup).toContain("No local model installed");
    expect(markup).toContain("Text and questions are never uploaded");
    expect(markup).toContain("No answers yet");
    expect(markup).toContain("Drop PDFs, images or Office files here");
    expect(markup).not.toContain("Page 1 of");
  });

  it("renders a ready model as an available, still-empty session", () => {
    const markup = renderPage(READY);

    expect(markup).toContain("Local model ready");
    expect(markup).toContain("Ready");
    expect(markup).toContain("Process at least one document first");
  });
});
