import { create } from "zustand";
import * as transport from "../transport";
import type { FileArtifact } from "../transport";
import { languageFromPath, isImagePath, isSvgPath } from "../file-utils";
import { useChatMetaStore } from "./chat-meta-store";

const MAX_OPEN_FILES = 10;
const MAX_ARTIFACTS_PER_SESSION = 500;

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
        }
      }

      result = { content, size, isReadonly: true };
    } else {
      try {
        result = await transport.readFileForViewer(resolved, workDir);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        console.warn("[file-viewer] failed to open file:", resolved, e);
        set({ lastOpenError: `无法打开文件：${msg}` });
        return;
      }
    }

    const now = Date.now();
    let openFiles = { ...get().openFiles };

    if (Object.keys(openFiles).length >= MAX_OPEN_FILES) {
      const lru = pickLruPath(openFiles);
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
      const { [path]: _, ...openFiles } = state.openFiles;
      const activeFilePath =
        state.activeFilePath === path ? pickMostRecentOpenFile(openFiles) : state.activeFilePath;
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

      const openFiles = sessionOpenFiles[newSessionId] ?? {};
      const artifacts = sessionArtifacts[newSessionId] ?? [];
      const activeFilePath = pickMostRecentOpenFile(openFiles);

      return {
        sessionOpenFiles,
        sessionArtifacts,
        openFiles,
        artifacts,
        activeFilePath,
      };
    });

    transport
      .listArtifacts(newSessionId)
      .then((artifacts) => {
        if (useChatMetaStore.getState().activeChatId !== newSessionId) return;
        useFileViewerStore.setState((state) => ({
          artifacts,
          sessionArtifacts: { ...state.sessionArtifacts, [newSessionId]: artifacts },
        }));
      })
      .catch(() => {});
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
    .then((artifacts) => {
      if (useChatMetaStore.getState().activeChatId !== sessionId) return;
      useFileViewerStore.setState((s) => ({
        artifacts,
        sessionArtifacts: { ...s.sessionArtifacts, [sessionId]: artifacts },
      }));
    })
    .catch((e) => {
      console.warn("[file-viewer] reload artifacts failed:", e);
    });
}
