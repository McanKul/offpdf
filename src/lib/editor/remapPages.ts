import type { EditDocument } from "./types";
import { cloneObject } from "./serialize";

export function samePageKeys(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((k, i) => k === b[i]);
}

/**
 * Rebind objects from assembled `pageIndex` onto a new page-key list.
 * Keys are `${file.uid}#${1-based page}` (see useCombinedDoc). Export still
 * uses 0-based pageIndex; this helper never writes pageKey into the document.
 */
export function remapEditDocument(
  doc: EditDocument,
  oldKeys: string[],
  newKeys: string[],
): { document: EditDocument; droppedIds: string[] } {
  const indexOf = new Map<string, number>();
  for (let i = 0; i < newKeys.length; i++) {
    if (!indexOf.has(newKeys[i])) indexOf.set(newKeys[i], i);
  }

  const droppedIds: string[] = [];
  const objects = [];
  for (const o of doc.objects) {
    const key = oldKeys[o.pageIndex];
    const next = key === undefined ? undefined : indexOf.get(key);
    if (next === undefined) {
      droppedIds.push(o.id);
      continue;
    }
    const clone = cloneObject(o);
    clone.pageIndex = next;
    if (clone.kind === "link" && clone.action.type === "goto") {
      const destKey = oldKeys[clone.action.destPageIndex];
      const nextDest = destKey === undefined ? undefined : indexOf.get(destKey);
      if (nextDest === undefined) {
        droppedIds.push(o.id);
        continue;
      }
      clone.action = { type: "goto", destPageIndex: nextDest };
    }
    objects.push(clone);
  }
  const keep = new Set(objects.map((o) => o.id));
  return {
    document: {
      version: doc.version,
      objects,
      selectedIds: doc.selectedIds.filter((id) => keep.has(id)),
    },
    droppedIds,
  };
}

export type KeyRebindPlan = {
  present: EditDocument;
  past: EditDocument[];
  future: EditDocument[];
  droppedIds: string[];
  historyDropped: boolean;
};

export function planKeyRebind(
  present: EditDocument,
  past: EditDocument[],
  future: EditDocument[],
  oldKeys: string[],
  newKeys: string[],
): KeyRebindPlan | null {
  if (samePageKeys(oldKeys, newKeys)) return null;

  const now = remapEditDocument(present, oldKeys, newKeys);
  const nextPast: EditDocument[] = [];
  const nextFuture: EditDocument[] = [];
  let historyDropped = false;
  for (const d of past) {
    const r = remapEditDocument(d, oldKeys, newKeys);
    if (r.droppedIds.length > 0) historyDropped = true;
    nextPast.push(r.document);
  }
  for (const d of future) {
    const r = remapEditDocument(d, oldKeys, newKeys);
    if (r.droppedIds.length > 0) historyDropped = true;
    nextFuture.push(r.document);
  }

  const wipeHistory = now.droppedIds.length > 0 || historyDropped;
  return {
    present: now.document,
    past: wipeHistory ? [] : nextPast,
    future: wipeHistory ? [] : nextFuture,
    droppedIds: now.droppedIds,
    historyDropped,
  };
}

export function rebindNeedsConfirm(plan: KeyRebindPlan): boolean {
  return plan.droppedIds.length > 0 || plan.historyDropped;
}

/** Keep the viewed page by stable key after the assembled list changes. */
export function resolveViewPageIndex(
  pageKeys: string[],
  currentIndex: number,
  prevKey: string | undefined,
): number {
  if (pageKeys.length === 0) return 0;
  if (prevKey) {
    const found = pageKeys.indexOf(prevKey);
    if (found >= 0) return found;
  }
  if (currentIndex >= 0 && currentIndex < pageKeys.length) return currentIndex;
  return pageKeys.length - 1;
}
