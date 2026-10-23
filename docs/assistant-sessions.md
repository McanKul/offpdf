# Private assistant sessions

The `/assistant` page is the optional user-facing session around bounded
ingestion (#86) and grounded answers (#87). It is intentionally unavailable until
a local model adapter reports `ready`; the rest of OffPDF does not depend on it.

## Session flow

1. Add PDFs to the shared workspace. The assistant never scans a containing
   folder and never reads a path the user did not add.
2. Process the selected documents. Native text is preferred. OCR runs only when
   the user explicitly enables it and picks an installed Tesseract language.
3. Ask a question or request a summary. The model receives only a bounded set of
   retrieved page chunks.
4. Every citation is validated against the cited chunk and opens the existing
   single-page viewer at that source page.
5. Cancel aborts native OCR or model work. Clear releases the session's text,
   chunks, warnings, questions, answers and citations.

## Data lifetime

- Extracted text, chunks, OCR text and history live in React state only. They
  are not written to local storage, synced, uploaded or sent to a cloud API.
- Leaving the page aborts in-flight work and releases the component's state.
  Closing the app has the same effect. No assistant state is restored on start.
- Clearing a session does not modify the selected PDFs and does not unload or
  delete the local model.
- The local model state is shown on the page. When no model is installed, the
  page is clearly optional and all processing controls stay disabled.

## Accessibility and states

- The question supports `Ctrl+Enter` / `Cmd+Enter`; all controls are native
  buttons, inputs or selects with visible focus styles.
- Answer history is an `aria-live` log and receives focus when a new result
  arrives. Citations are keyboard-focusable buttons.
- Empty, loading, processing, cancelled, error and insufficient-evidence states
  each have explicit copy. Processing shows bounded progress and a cancel
  control. Scanned pages without native text produce an actionable OCR warning.

## Model boundary

`src/lib/assistant/model.ts` defines the product-facing adapter. The current
default is an explicitly unavailable model. A future local runtime supplies an
`AssistantModel` whose `generate` method returns the versioned JSON envelope
validated by `src/lib/assistant/citations.ts`. Prompt construction is shared in
`buildAssistantPrompt`, and citations cannot name a chunk that was not retrieved
for that request.
