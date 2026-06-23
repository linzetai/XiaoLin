import { create } from "zustand";
import type {
  GitStatus,
  DiffHunk,
  Branch,
  CommitResult,
} from "../../../../xiaolin-protocol/generated/protocol";
import * as transport from "../transport";
import { useProjectStore } from "./project-store";

interface GitState {
  status: GitStatus | null;
  branches: Branch[];
  currentBranch: string;
  selectedDiff: DiffHunk[];
  selectedFile: string | null;
  selectedFileStaged: boolean;
  loading: boolean;
  lastRefreshAt: number;
}

interface GitActions {
  refresh: () => Promise<void>;
  selectFile: (path: string, staged: boolean) => Promise<void>;
  clearSelection: () => void;
  stageFiles: (files?: string[]) => Promise<void>;
  unstageFiles: (files?: string[]) => Promise<void>;
  commitChanges: (message: string) => Promise<CommitResult | null>;
  revertFiles: (files: string[]) => Promise<void>;
  fetchBranches: () => Promise<void>;
}

type GitStore = GitState & GitActions;

export const useGitStore = create<GitStore>((set) => ({
  status: null,
  branches: [],
  currentBranch: "",
  selectedDiff: [],
  selectedFile: null,
  selectedFileStaged: false,
  loading: false,
  lastRefreshAt: 0,

  refresh: async () => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;

    set({ loading: true });
    try {
      const data = await transport.gitStatus(activeProjectId);
      if (data) {
        set({
          status: data,
          currentBranch: data.branch,
          lastRefreshAt: Date.now(),
        });
      }
    } finally {
      set({ loading: false });
    }
  },

  selectFile: async (path: string, staged: boolean) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;

    set({ selectedFile: path, selectedFileStaged: staged });
    const hunks = await transport.gitDiff(activeProjectId, path, staged);
    set({ selectedDiff: hunks });
  },

  clearSelection: () => {
    set({ selectedFile: null, selectedFileStaged: false, selectedDiff: [] });
  },

  stageFiles: async (files?: string[]) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;
    await transport.gitStage(activeProjectId, files ?? []);
  },

  unstageFiles: async (files?: string[]) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;
    await transport.gitUnstage(activeProjectId, files ?? []);
  },

  commitChanges: async (message: string) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return null;
    const result = await transport.gitCommit(activeProjectId, message);
    return result;
  },

  revertFiles: async (files: string[]) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;
    await transport.gitRevert(activeProjectId, files);
  },

  fetchBranches: async () => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (!activeProjectId) return;
    const data = await transport.gitBranches(activeProjectId);
    set({
      branches: data.branches,
      currentBranch: data.current,
    });
  },
}));

let eventUnsub: (() => void) | null = null;
let projectUnsub: (() => void) | null = null;
let visibilityUnsub: (() => void) | null = null;
let staleCheckTimer: ReturnType<typeof setInterval> | null = null;

const STALE_REFRESH_MS = 5 * 60 * 1000;

function handleVisibilityChange() {
  if (document.visibilityState === "visible") {
    void useGitStore.getState().refresh();
  }
}

function checkStaleRefresh() {
  const activeProjectId = useProjectStore.getState().activeProjectId;
  if (!activeProjectId) return;
  const { lastRefreshAt } = useGitStore.getState();
  if (Date.now() - lastRefreshAt >= STALE_REFRESH_MS) {
    void useGitStore.getState().refresh();
  }
}

export function initGitStore() {
  destroyGitStore();

  eventUnsub = transport.onGitStatusChanged((projectId, status) => {
    const activeProjectId = useProjectStore.getState().activeProjectId;
    if (projectId === activeProjectId && status) {
      useGitStore.setState({
        status,
        currentBranch: status.branch,
        lastRefreshAt: Date.now(),
      });
    }
  });

  projectUnsub = useProjectStore.subscribe((state, prev) => {
    if (state.activeProjectId !== prev.activeProjectId) {
      useGitStore.setState({
        status: null,
        branches: [],
        selectedDiff: [],
        selectedFile: null,
      });
      if (state.activeProjectId) {
        useGitStore.getState().refresh();
      }
    }
  });

  document.addEventListener("visibilitychange", handleVisibilityChange);
  visibilityUnsub = () => document.removeEventListener("visibilitychange", handleVisibilityChange);
  staleCheckTimer = setInterval(checkStaleRefresh, STALE_REFRESH_MS);

  useGitStore.getState().refresh();
}

export function destroyGitStore() {
  if (eventUnsub) {
    eventUnsub();
    eventUnsub = null;
  }
  if (projectUnsub) {
    projectUnsub();
    projectUnsub = null;
  }
  if (visibilityUnsub) {
    visibilityUnsub();
    visibilityUnsub = null;
  }
  if (staleCheckTimer) {
    clearInterval(staleCheckTimer);
    staleCheckTimer = null;
  }
}
