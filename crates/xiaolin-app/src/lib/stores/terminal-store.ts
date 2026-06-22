import { create } from "zustand";

const MAX_LINES_PER_SESSION = 2000;
const MAX_DONE_SESSIONS = 20;

export interface TerminalLine {
  text: string;
  timestamp: number;
}

export interface TerminalSession {
  callId: string;
  toolName: string;
  lines: TerminalLine[];
  status: "running" | "done";
  command?: string;
  chatId?: string;
}

interface TerminalState {
  sessions: Record<string, TerminalSession>;
  activeCallId: string | null;

  startSession: (callId: string, toolName: string, command?: string, chatId?: string) => void;
  appendOutput: (callId: string, text: string) => void;
  endSession: (callId: string) => void;
  setActive: (callId: string | null) => void;
  clear: (callId: string) => void;
}

export const useTerminalStore = create<TerminalState>((set) => ({
  sessions: {},
  activeCallId: null,

  startSession: (callId, toolName, command, chatId) => {
    set((s) => ({
      sessions: {
        ...s.sessions,
        [callId]: { callId, toolName, lines: [], status: "running", command, chatId },
      },
      activeCallId: callId,
    }));
  },

  appendOutput: (callId, text) => {
    set((s) => {
      const session = s.sessions[callId];
      if (!session) return s;
      const newLine: TerminalLine = { text, timestamp: Date.now() };
      let lines = [...session.lines, newLine];
      if (lines.length > MAX_LINES_PER_SESSION) {
        lines = lines.slice(lines.length - MAX_LINES_PER_SESSION);
      }
      return {
        sessions: {
          ...s.sessions,
          [callId]: { ...session, lines },
        },
      };
    });
  },

  endSession: (callId) => {
    set((s) => {
      const session = s.sessions[callId];
      if (!session) return s;
      const updated = { ...s.sessions, [callId]: { ...session, status: "done" as const } };
      const doneIds = Object.entries(updated)
        .filter(([, v]) => v.status === "done")
        .map(([k]) => k);
      if (doneIds.length > MAX_DONE_SESSIONS) {
        const toRemove = doneIds.slice(0, doneIds.length - MAX_DONE_SESSIONS);
        for (const id of toRemove) delete updated[id];
      }
      return { sessions: updated };
    });
  },

  setActive: (callId) => {
    set({ activeCallId: callId });
  },

  clear: (callId) => {
    set((s) => {
      const { [callId]: _, ...rest } = s.sessions;
      const activeCallId = s.activeCallId === callId ? null : s.activeCallId;
      return { sessions: rest, activeCallId };
    });
  },
}));
