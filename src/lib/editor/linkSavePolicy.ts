/**
 * Edit-PDF Save gates for link hydrate / multi-source rewrite (issue #35 r2).
 *
 * Empty objects block Save only when this session never hydrated a link.
 * Incomplete hydrate is per failed source — complete sources still rewrite.
 */

export function emptyObjectsBlockSave(args: {
  objectCount: number;
  hadHydratedLinks: boolean;
}): boolean {
  return args.objectCount === 0 && !args.hadHydratedLinks;
}

export function incompleteSourceIds(
  sourceIds: readonly string[],
  failedIds: ReadonlySet<string>,
): string[] {
  return sourceIds.filter((id) => failedIds.has(id));
}

export function incompleteSourcePaths(
  files: readonly { uid: string; path: string }[],
  failedUids: ReadonlySet<string>,
): string[] {
  return files.filter((f) => failedUids.has(f.uid)).map((f) => f.path);
}

export function shouldRewriteSourceLinks(args: {
  sourceId: string;
  incompleteSourceIds: readonly string[];
}): boolean {
  return !args.incompleteSourceIds.includes(args.sourceId);
}
