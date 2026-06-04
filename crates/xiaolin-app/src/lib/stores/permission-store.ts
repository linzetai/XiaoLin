import { create } from "zustand";
import * as transport from "../transport";

export interface PermissionStoreState {
  presets: transport.PermissionPreset[];
  presetsLoaded: boolean;
  /** Active preset ID per session. Key is sessionId. */
  sessionPresetIds: Record<string, string>;
  loadPresets: () => Promise<void>;
  getSessionPreset: (sessionId: string) => string;
  setSessionPreset: (sessionId: string, presetId: string) => Promise<void>;
  fetchSessionPreset: (sessionId: string) => Promise<void>;
}

export const usePermissionStore = create<PermissionStoreState>((set, get) => ({
  presets: [],
  presetsLoaded: false,
  sessionPresetIds: {},

  loadPresets: async () => {
    try {
      const presets = await transport.getPermissionPresets();
      set({ presets, presetsLoaded: true });
    } catch {
      /* keep empty */
    }
  },

  getSessionPreset: (sessionId: string) => {
    return get().sessionPresetIds[sessionId] ?? "";
  },

  setSessionPreset: async (sessionId: string, presetId: string) => {
    set((s) => ({
      sessionPresetIds: { ...s.sessionPresetIds, [sessionId]: presetId },
    }));
    try {
      await transport.setSessionPermission(sessionId, presetId);
    } catch {
      /* revert on error */
      set((s) => {
        const next = { ...s.sessionPresetIds };
        delete next[sessionId];
        return { sessionPresetIds: next };
      });
    }
  },

  fetchSessionPreset: async (sessionId: string) => {
    try {
      const info = await transport.getSessionPermission(sessionId);
      if (info.hasOverride && info.presetId) {
        set((s) => ({
          sessionPresetIds: {
            ...s.sessionPresetIds,
            [sessionId]: info.presetId,
          },
        }));
      }
    } catch {
      /* ignore */
    }
  },
}));

let _unsub: (() => void) | undefined;

export function initPermissionListener(): void {
  _unsub?.();
  _unsub = transport.onPermissionsChanged((sessionId, presetId) => {
    usePermissionStore.setState((s) => ({
      sessionPresetIds: { ...s.sessionPresetIds, [sessionId]: presetId },
    }));
  });
}

export function teardownPermissionListener(): void {
  _unsub?.();
  _unsub = undefined;
}
