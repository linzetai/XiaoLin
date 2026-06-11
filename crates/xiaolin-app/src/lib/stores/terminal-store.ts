import { create } from "zustand";

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
}

interface TerminalState {
  sessions: Record<string, TerminalSession>;
  activeCallId: string | null;

  startSession: (callId: string, toolName: string, command?: string) => void;
  appendOutput: (callId: string, text: string) => void;
  endSession: (callId: string) => void;
  setActive: (callId: string | null) => void;
  clear: (callId: string) => void;
}

export const useTerminalStore = create<TerminalState>((set) => ({
  sessions: {},
  activeCallId: null,

  startSession: (callId, toolName, command) => {
    set((s) => ({
      sessions: {
        ...s.sessions,
        [callId]: { callId, toolName, lines: [], status: "running", command },
      },
      activeCallId: callId,
    }));
  },

  appendOutput: (callId, text) => {
    set((s) => {
      const session = s.sessions[callId];
      if (!session) return s;
      const newLine: TerminalLine = { text, timestamp: Date.now() };
      return {
        sessions: {
          ...s.sessions,
          [callId]: { ...session, lines: [...session.lines, newLine] },
        },
      };
    });
  },

  endSession: (callId) => {
    set((s) => {
      const session = s.sessions[callId];
      if (!session) return s;
      return {
        sessions: {
          ...s.sessions,
          [callId]: { ...session, status: "done" },
        },
      };
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
