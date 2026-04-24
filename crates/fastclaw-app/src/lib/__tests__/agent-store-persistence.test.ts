import { describe, it, expect, beforeEach } from "vitest";

const STORAGE_KEY = "fastclaw:ui-state";
const UI_STATE_VERSION = 1;

interface PersistedUIState {
  version: number;
  activeAgentId: string;
  agentActiveChats: Record<string, string>;
  agentOpenChats: Record<string, string[]>;
}

function loadUIState(): PersistedUIState | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as PersistedUIState;
    if (parsed.version !== UI_STATE_VERSION) return null;
    return parsed;
  } catch {
    return null;
  }
}

describe("loadUIState (localStorage schema versioning)", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("returns null when nothing is stored", () => {
    expect(loadUIState()).toBeNull();
  });

  it("returns parsed state when version matches", () => {
    const state: PersistedUIState = {
      version: UI_STATE_VERSION,
      activeAgentId: "main",
      agentActiveChats: { main: "chat-1" },
      agentOpenChats: { main: ["chat-1", "chat-2"] },
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    expect(loadUIState()).toEqual(state);
  });

  it("returns null when version is missing (legacy data)", () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      activeAgentId: "main",
      agentActiveChats: {},
      agentOpenChats: {},
    }));
    expect(loadUIState()).toBeNull();
  });

  it("returns null when version does not match", () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      version: 999,
      activeAgentId: "main",
      agentActiveChats: {},
      agentOpenChats: {},
    }));
    expect(loadUIState()).toBeNull();
  });

  it("returns null on corrupted JSON", () => {
    localStorage.setItem(STORAGE_KEY, "not-json{{{");
    expect(loadUIState()).toBeNull();
  });
});
