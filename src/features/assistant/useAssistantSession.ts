import { useCallback, useEffect, useReducer, useRef } from "react";
import {
  ingestPdfSources,
  isIngestionCancelled,
  type IngestionRuntime,
} from "@/lib/assistant/ingestion";
import type { AssistantModel } from "@/lib/assistant/model";
import {
  assistantErrorMessage,
  assistantIngestionProgress,
  assistantSessionReducer,
  hasSessionData,
  INITIAL_ASSISTANT_SESSION,
  isAbortError,
  runGroundedAnswer,
  runGroundedSummary,
} from "@/lib/assistant/session";
import type { WorkspaceFile } from "@/lib/types";

export interface UseAssistantSessionOptions {
  readonly files: readonly WorkspaceFile[];
  readonly model: AssistantModel;
  readonly runtime: IngestionRuntime;
}

export interface AssistantSessionController {
  readonly state: ReturnType<typeof assistantSessionReducer>;
  readonly modelReady: boolean;
  readonly busy: boolean;
  readonly hasData: boolean;
  readonly hasChunks: boolean;
  readonly processDocuments: (options: {
    readonly ocrEnabled: boolean;
    readonly ocrLanguage: string;
  }) => Promise<void>;
  readonly ask: (query: string) => Promise<void>;
  readonly summarize: () => Promise<void>;
  readonly cancel: () => void;
  readonly clear: () => void;
}

/**
 * Owns one in-memory assistant session. Aborting a run also aborts native OCR
 * and tells the model adapter to stop. Nothing here writes to storage.
 */
export function useAssistantSession({
  files,
  model,
  runtime,
}: UseAssistantSessionOptions): AssistantSessionController {
  const [state, dispatch] = useReducer(
    assistantSessionReducer,
    INITIAL_ASSISTANT_SESSION,
  );
  const abortRef = useRef<AbortController | null>(null);
  const runIdRef = useRef(0);
  const turnSeqRef = useRef(0);
  const chunksRef = useRef(state.chunks);
  chunksRef.current = state.chunks;

  const nextTurnId = useCallback(() => {
    turnSeqRef.current += 1;
    return `turn-${turnSeqRef.current}`;
  }, []);

  const beginRun = useCallback(() => {
    abortRef.current?.abort();
    const controller = new AbortController();
    abortRef.current = controller;
    runIdRef.current += 1;
    return { controller, runId: runIdRef.current };
  }, []);

  const finishRun = useCallback((runId: number) => {
    if (runIdRef.current === runId) abortRef.current = null;
  }, []);

  useEffect(
    () => () => {
      abortRef.current?.abort();
      runIdRef.current += 1;
    },
    [],
  );

  const processDocuments = useCallback(
    async ({
      ocrEnabled,
      ocrLanguage,
    }: {
      ocrEnabled: boolean;
      ocrLanguage: string;
    }) => {
      if (model.status.state !== "ready") return;
      if (files.length === 0) {
        dispatch({
          type: "fail",
          message: "Add at least one PDF to the workspace before processing.",
        });
        return;
      }
      if (ocrEnabled && ocrLanguage.trim().length === 0) {
        dispatch({
          type: "fail",
          message: "Choose an installed OCR language before using OCR.",
        });
        return;
      }

      const { controller, runId } = beginRun();
      dispatch({ type: "start-processing" });

      const sourceNames = new Map(files.map((file) => [file.uid, file.name]));
      try {
        const result = await ingestPdfSources(
          files.map((file) => ({
            path: file.path,
            id: file.uid,
            name: file.name,
          })),
          runtime,
          {
            ocr: ocrEnabled
              ? { enabled: true, language: ocrLanguage }
              : { enabled: false },
            signal: controller.signal,
            onProgress: (progress) => {
              if (runIdRef.current !== runId) return;
              dispatch({
                type: "progress",
                progress: assistantIngestionProgress(progress, sourceNames),
              });
            },
          },
        );
        if (runIdRef.current !== runId) return;
        dispatch({ type: "processing-done", result });
      } catch (error) {
        if (runIdRef.current !== runId) return;
        if (
          controller.signal.aborted ||
          isIngestionCancelled(error) ||
          isAbortError(error)
        ) {
          dispatch({ type: "cancel" });
        } else {
          dispatch({ type: "fail", message: assistantErrorMessage(error) });
        }
      } finally {
        finishRun(runId);
      }
    },
    [beginRun, files, finishRun, model, runtime],
  );

  const ask = useCallback(
    async (query: string) => {
      if (model.status.state !== "ready") return;
      if (state.phase !== "idle") return;
      if (chunksRef.current.length === 0) {
        dispatch({
          type: "fail",
          message: "Process the selected documents before asking a question.",
        });
        return;
      }
      if (query.trim().length === 0) {
        dispatch({
          type: "fail",
          message: "Enter a question about the processed documents.",
        });
        return;
      }

      const { controller, runId } = beginRun();
      dispatch({ type: "start-answering", kind: "answer", request: query });
      try {
        const turn = await runGroundedAnswer(
          nextTurnId(),
          query,
          chunksRef.current,
          model,
          controller.signal,
        );
        if (runIdRef.current !== runId) return;
        dispatch({ type: "turn", turn });
      } catch (error) {
        if (runIdRef.current !== runId) return;
        if (controller.signal.aborted || isAbortError(error)) {
          dispatch({ type: "cancel" });
        } else {
          dispatch({ type: "fail", message: assistantErrorMessage(error) });
        }
      } finally {
        finishRun(runId);
      }
    },
    [beginRun, finishRun, model, nextTurnId, state.phase],
  );

  const summarize = useCallback(async () => {
    if (model.status.state !== "ready") return;
    if (state.phase !== "idle") return;
    if (chunksRef.current.length === 0) {
      dispatch({
        type: "fail",
        message: "Process the selected documents before requesting a summary.",
      });
      return;
    }

    const { controller, runId } = beginRun();
    dispatch({ type: "start-answering", kind: "summary" });
    try {
      const turn = await runGroundedSummary(
        nextTurnId(),
        chunksRef.current,
        model,
        controller.signal,
      );
      if (runIdRef.current !== runId) return;
      dispatch({ type: "turn", turn });
    } catch (error) {
      if (runIdRef.current !== runId) return;
      if (controller.signal.aborted || isAbortError(error)) {
        dispatch({ type: "cancel" });
      } else {
        dispatch({ type: "fail", message: assistantErrorMessage(error) });
      }
    } finally {
      finishRun(runId);
    }
  }, [beginRun, finishRun, model, nextTurnId, state.phase]);

  const cancel = useCallback(() => {
    runIdRef.current += 1;
    abortRef.current?.abort();
    abortRef.current = null;
    dispatch({ type: "cancel" });
  }, []);

  const clear = useCallback(() => {
    runIdRef.current += 1;
    abortRef.current?.abort();
    abortRef.current = null;
    dispatch({ type: "clear" });
  }, []);

  return {
    state,
    modelReady: model.status.state === "ready",
    busy: state.phase !== "idle",
    hasData: hasSessionData(state),
    hasChunks: state.chunks.length > 0,
    processDocuments,
    ask,
    summarize,
    cancel,
    clear,
  };
}
