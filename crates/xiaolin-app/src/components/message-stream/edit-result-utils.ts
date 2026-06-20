export interface EditResult {
  edited: boolean;
  path: string;
  replacements: number;
  bytes: number;
  diffStat: string;
  linesAdded: number;
  linesRemoved: number;
}

export function parseEditResult(result: string): EditResult | null {
  try {
    const parsed = JSON.parse(result);
    if (parsed.edited === true || parsed.created === true || parsed.written === true || parsed.patched === true) {
      const path = parsed.file_path || parsed.path;
      if (!path) return null;

      let linesAdded: number = parsed.linesAdded ?? 0;
      let linesRemoved: number = parsed.linesRemoved ?? 0;

      if (linesAdded === 0 && linesRemoved === 0 && parsed.diffStat) {
        const m = (parsed.diffStat as string).match(/\+(\d+)\s+-(\d+)/);
        if (m) {
          linesAdded = parseInt(m[1], 10);
          linesRemoved = parseInt(m[2], 10);
        }
      }

      return {
        edited: true,
        path,
        replacements: parsed.replacements ?? parsed.edits_applied ?? 1,
        bytes: parsed.bytes ?? 0,
        diffStat: parsed.diffStat ?? "",
        linesAdded,
        linesRemoved,
      };
    }
  } catch { /* not JSON — try plain text below */ }

  const match = result.match(/^edited\s+(.+)$/);
  if (match) {
    return {
      edited: true,
      path: match[1].trim(),
      replacements: 1,
      bytes: 0,
      diffStat: "",
      linesAdded: 0,
      linesRemoved: 0,
    };
  }

  return null;
}

export interface FileChangeSummary {
  totalFiles: number;
  totalAdded: number;
  totalRemoved: number;
  files: Array<{
    path: string;
    linesAdded: number;
    linesRemoved: number;
    replacements: number;
  }>;
}

export function aggregateFileChanges(results: string[]): FileChangeSummary | null {
  const byPath = new Map<string, { linesAdded: number; linesRemoved: number; replacements: number }>();

  for (const raw of results) {
    const edit = parseEditResult(raw);
    if (!edit) continue;
    const existing = byPath.get(edit.path);
    if (existing) {
      existing.linesAdded += edit.linesAdded;
      existing.linesRemoved += edit.linesRemoved;
      existing.replacements += edit.replacements;
    } else {
      byPath.set(edit.path, {
        linesAdded: edit.linesAdded,
        linesRemoved: edit.linesRemoved,
        replacements: edit.replacements,
      });
    }
  }

  if (byPath.size === 0) return null;

  let totalAdded = 0;
  let totalRemoved = 0;
  const files: FileChangeSummary["files"] = [];

  for (const [path, stats] of byPath) {
    totalAdded += stats.linesAdded;
    totalRemoved += stats.linesRemoved;
    files.push({ path, ...stats });
  }

  return { totalFiles: files.length, totalAdded, totalRemoved, files };
}
