import { create } from "zustand";
import * as transport from "../transport";
import type { FileArtifact } from "../transport";
import { languageFromPath, isImagePath, isSvgPath } from "../file-utils";
import { useChatMetaStore } from "./chat-meta-store";

const MAX_OPEN_FILES = 10;
const MAX_ARTIFACTS_PER_SESSION = 500;
const MAX_CACHED_SESSIONS = 20;

export interface OpenFile {
  path: string;
  content: string;
  size: number;
  isReadonly: boolean;
  language: string;
  viewMode: "code" | "preview";
  lastAccessed: number;
  line?: number;
}

export interface FileViewerState {
  openFiles: Record<string, OpenFile>;
  activeFilePath: string | null;
  artifacts: FileArtifact[];
  fileListCollapsed: boolean;
  lastOpenError: string | null;

  sessionArtifacts: Record<string, FileArtifact[]>;
  sessionOpenFiles: Record<string, Record<string, OpenFile>>;

  openFile: (path: string, workDir: string, line?: number) => Promise<void>;
  closeFile: (path: string) => void;
  setActiveFile: (path: string) => void;
  setViewMode: (path: string, mode: "code" | "preview") => void;
  toggleFileList: () => void;
  updateArtifacts: (artifacts: FileArtifact[]) => void;
  addArtifact: (artifact: FileArtifact) => void;
  switchSession: (newSessionId: string, oldSessionId: string | null) => void;
  clearOpenError: () => void;
}


/** Resolve absolute or relative paths against a work directory. */
export function resolveFilePath(path: string, workDir: string): string {
  const trimmed = path.trim();
  if (!trimmed) return "";
  if (trimmed.startsWith("/") || /^[A-Za-z]:[\\/]/.test(trimmed)) {
    return trimmed;
  }
  const base = workDir.replace(/\/+$/, "");
  const relative = trimmed.replace(/^\.?\//, "");
  return `${base}/${relative}`;
}


function defaultViewMode(path: string): "code" | "preview" {
  const lower = path.toLowerCase();
  if (lower.endsWith(".md") || lower.endsWith(".mdx")) return "preview";
  if (isSvgPath(path)) return "preview";
  return "code";
}

function pickMostRecentOpenFile(
  openFiles: Record<string, OpenFile>,
  excludePath?: string,
): string | null {
  let best: string | null = null;
  let bestTs = -1;
  for (const [path, file] of Object.entries(openFiles)) {
    if (path === excludePath) continue;
    if (file.lastAccessed > bestTs) {
      bestTs = file.lastAccessed;
      best = path;
    }
  }
  return best;
}

function pickLruPath(openFiles: Record<string, OpenFile>, excludePath?: string): string | null {
  let lru: string | null = null;
  let lruTs = Infinity;
  for (const [path, file] of Object.entries(openFiles)) {
    if (path === excludePath) continue;
    if (file.lastAccessed < lruTs) {
      lruTs = file.lastAccessed;
      lru = path;
    }
  }
  return lru;
}

/** Merge server-fetched artifacts with locally-buffered ones (from WS events). */
function mergeArtifacts(local: FileArtifact[], server: FileArtifact[]): FileArtifact[] {
  const seen = new Set<string>();
  const merged: FileArtifact[] = [];
  for (const a of [...local, ...server]) {
    const key = `${a.path}\0${a.timestamp}`;
    if (seen.has(key)) continue;
    seen.add(key);
    merged.push(a);
  }
  return merged.length > MAX_ARTIFACTS_PER_SESSION
    ? merged.slice(0, MAX_ARTIFACTS_PER_SESSION)
    : merged;
}

function prependArtifact(list: FileArtifact[], artifact: FileArtifact): FileArtifact[] {
  if (list.some((a) => a.path === artifact.path && a.timestamp === artifact.timestamp)) {
    return list;
  }
  const updated = [artifact, ...list];
  return updated.length > MAX_ARTIFACTS_PER_SESSION
    ? updated.slice(0, MAX_ARTIFACTS_PER_SESSION)
    : updated;
}

function parseArtifactData(data: Record<string, unknown>): FileArtifact | null {
  if (typeof data.path !== "string") return null;
  const op = data.operation;
  if (op !== "created" && op !== "modified" && op !== "deleted") return null;
  return {
    path: data.path,
    operation: op,
    timestamp: typeof data.timestamp === "string" ? data.timestamp : new Date().toISOString(),
    toolCallId: typeof data.toolCallId === "string" ? data.toolCallId : "",
    bytes: typeof data.bytes === "number" ? data.bytes : 0,
  };
}

export const useFileViewerStore = create<FileViewerState>((set, get) => ({
  openFiles: {},
  activeFilePath: null,
  artifacts: [],
  fileListCollapsed: false,
  lastOpenError: null,
  sessionArtifacts: {},
  sessionOpenFiles: {},

  openFile: async (path, workDir, line) => {
    const resolved = resolveFilePath(path, workDir);
    if (!resolved || !workDir) return;

    const sessionAtStart = useChatMetaStore.getState().activeChatId;
    set({ lastOpenError: null });

    const existing = get().openFiles[resolved];
    if (existing) {
      set({
        activeFilePath: resolved,
        openFiles: {
          ...get().openFiles,
          [resolved]: {
            ...existing,
            lastAccessed: Date.now(),
            ...(line != null ? { line } : {}),
          },
        },
      });
      return;
    }

    let result: { content: string; size: number; isReadonly: boolean };

    if (isImagePath(resolved)) {
      let content = "";
      let size = 0;

      if (isSvgPath(resolved)) {
        try {
          const text = await transport.readFileForViewer(resolved, workDir);
          content = text.content;
          size = text.size;
        } catch (e) {
          console.warn("[file-viewer] failed to read SVG text:", resolved, e);
          set({ lastOpenError: "无法打开文件，请确认文件路径和权限是否正确" });
          return;
        }
      }

      result = { content, size, isReadonly: true };
    } else {
      try {
        result = await transport.readFileForViewer(resolved, workDir);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        console.warn("[file-viewer] failed to open file:", resolved, e);
        console.warn("[file-viewer] open error detail:", msg);
        set({ lastOpenError: "无法打开文件，请确认文件路径和权限是否正确" });
        return;
      }
    }

    if (useChatMetaStore.getState().activeChatId !== sessionAtStart) return;

    const now = Date.now();
    let openFiles = { ...get().openFiles };

    if (Object.keys(openFiles).length >= MAX_OPEN_FILES) {
      const lru = pickLruPath(openFiles, get().activeFilePath ?? undefined);
      if (lru) {
        const { [lru]: _, ...rest } = openFiles;
        openFiles = rest;
      }
    }

    openFiles[resolved] = {
      path: resolved,
      content: result.content,
      size: result.size,
      isReadonly: result.isReadonly,
      language: isImagePath(resolved) ? "image" : languageFromPath(resolved),
      viewMode: defaultViewMode(resolved),
      lastAccessed: now,
      line,
    };

    set({ openFiles, activeFilePath: resolved });
  },

  closeFile: (path) => {
    set((state) => {
      if (!state.openFiles[path]) return state;
      const paths = Object.keys(state.openFiles);
      const { [path]: _, ...openFiles } = state.openFiles;

      let activeFilePath = state.activeFilePath;
      if (activeFilePath === path) {
        const idx = paths.indexOf(path);
        const next = paths[idx + 1] ?? paths[idx - 1] ?? null;
        activeFilePath = next && openFiles[next] ? next : pickMostRecentOpenFile(openFiles);
      }
      return { openFiles, activeFilePath };
    });
  },

  setActiveFile: (path) => {
    const file = get().openFiles[path];
    if (!file) return;
    set({
      activeFilePath: path,
      openFiles: {
        ...get().openFiles,
        [path]: { ...file, lastAccessed: Date.now() },
      },
    });
  },

  setViewMode: (path, mode) => {
    const file = get().openFiles[path];
    if (!file) return;
    set({
      openFiles: {
        ...get().openFiles,
        [path]: { ...file, viewMode: mode },
      },
    });
  },

  toggleFileList: () => set((s) => ({ fileListCollapsed: !s.fileListCollapsed })),

  updateArtifacts: (artifacts) => set({ artifacts }),

  addArtifact: (artifact) => {
    set((state) => {
      const updated = prependArtifact(state.artifacts, artifact);
      if (updated === state.artifacts) return state;
      return { artifacts: updated };
    });
  },

  clearOpenError: () => set({ lastOpenError: null }),

  switchSession: (newSessionId, oldSessionId) => {
    set((state) => {
      const sessionOpenFiles = { ...state.sessionOpenFiles };
      const sessionArtifacts = { ...state.sessionArtifacts };

      if (oldSessionId) {
        sessionOpenFiles[oldSessionId] = state.openFiles;
        sessionArtifacts[oldSessionId] = state.artifacts;
      }

      const cachedKeys = Object.keys(sessionOpenFiles);
      if (cachedKeys.length > MAX_CACHED_SESSIONS) {
        const toEvict = cachedKeys
          .filter((k) => k !== newSessionId && k !== oldSessionId)
          .slice(0, cachedKeys.length - MAX_CACHED_SESSIONS);
        for (const k of toEvict) {
          delete sessionOpenFiles[k];
          delete sessionArtifacts[k];
        }
      }

      const openFiles = sessionOpenFiles[newSessionId] ?? {};
      const artifacts = sessionArtifacts[newSessionId] ?? [];
      const activeFilePath = pickMostRecentOpenFile(openFiles);

      return {
        sessionOpenFiles,
        sessionArtifacts,
        openFiles,
        artifacts,
        activeFilePath,
        lastOpenError: null,
      };
    });

    if (!newSessionId.startsWith("new-")) {
      transport
        .listArtifacts(newSessionId)
        .then((serverArtifacts) => {
          if (useChatMetaStore.getState().activeChatId !== newSessionId) return;
          useFileViewerStore.setState((state) => {
            const merged = mergeArtifacts(state.artifacts, serverArtifacts);
            return {
              artifacts: merged,
              sessionArtifacts: { ...state.sessionArtifacts, [newSessionId]: merged },
            };
          });
        })
        .catch((e) => {
          console.warn("[file-viewer] failed to load artifacts for session:", newSessionId, e);
        });
    }
  },

}));

let _unsubFileArtifact: (() => void) | undefined;

export function initFileArtifactListener(getActiveChatId: () => string): void {
  _unsubFileArtifact?.();
  _unsubFileArtifact = transport.onWsEvent("file_artifact", (msg) => {
    const chatId = getActiveChatId();
    if (!chatId) return;

    const data = (msg as { data?: Record<string, unknown> })?.data;
    if (!data) return;

    const sessionId =
      typeof data.sessionId === "string"
        ? data.sessionId
        : typeof data.session_id === "string"
          ? data.session_id
          : null;
    if (sessionId && sessionId !== chatId) return;

    const artifact = parseArtifactData(data);
    if (!artifact) return;

    useFileViewerStore.setState((s) => {
      const updated = prependArtifact(s.artifacts, artifact);
      if (updated === s.artifacts) return s;
      return {
        artifacts: updated,
        sessionArtifacts: { ...s.sessionArtifacts, [chatId]: updated },
      };
    });
  });
}

export function teardownFileArtifactListener(): void {
  _unsubFileArtifact?.();
  _unsubFileArtifact = undefined;
}

export function reloadArtifactsForCurrentSession(): void {
  const sessionId = useChatMetaStore.getState().activeChatId;
  if (!sessionId || sessionId.startsWith("new-")) return;
  transport
    .listArtifacts(sessionId)
    .then((serverArtifacts) => {
      if (useChatMetaStore.getState().activeChatId !== sessionId) return;
      useFileViewerStore.setState((s) => {
        const merged = mergeArtifacts(s.artifacts, serverArtifacts);
        return {
          artifacts: merged,
          sessionArtifacts: { ...s.sessionArtifacts, [sessionId]: merged },
        };
      });
    })
    .catch((e) => {
      console.warn("[file-viewer] reload artifacts failed:", e);
    });
}
