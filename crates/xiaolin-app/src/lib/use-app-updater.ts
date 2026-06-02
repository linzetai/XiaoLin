import { useState, useEffect, useCallback, useRef } from "react";
import * as transport from "./transport";

export type UpdateStatus =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "ready"
  | "up-to-date"
  | "error";

export interface UpdateInfo {
  version: string;
  body: string;
  date: string;
}

interface UpdaterState {
  status: UpdateStatus;
  info: UpdateInfo | null;
  progress: number;
  error: string | null;
}

const CHECK_INTERVAL_MS = 60 * 60 * 1000; // 1 hour

export function useAppUpdater(autoCheck = true) {
  const [state, setState] = useState<UpdaterState>({
    status: "idle",
    info: null,
    progress: 0,
    error: null,
  });

  const updateRef = useRef<unknown>(null);
  const timerRef = useRef<ReturnType<typeof setInterval>>(undefined);

  const checkForUpdate = useCallback(async () => {
    if (!transport.isTauri) return;
    setState((s) => ({ ...s, status: "checking", error: null }));

    try {
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();

      if (update) {
        updateRef.current = update;
        setState({
          status: "available",
          info: {
            version: update.version,
            body: update.body ?? "",
            date: update.date ?? "",
          },
          progress: 0,
          error: null,
        });
      } else {
        setState({
          status: "up-to-date",
          info: null,
          progress: 0,
          error: null,
        });
      }
    } catch (err) {
      setState((s) => ({
        ...s,
        status: "error",
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  const downloadAndInstall = useCallback(async () => {
    const update = updateRef.current as
      | { downloadAndInstall: (cb?: (event: { event: string; data: { chunkLength?: number; contentLength?: number } }) => void) => Promise<void> }
      | null;
    if (!update) return;

    setState((s) => ({ ...s, status: "downloading", progress: 0 }));

    let downloaded = 0;
    try {
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          downloaded = 0;
        } else if (event.event === "Progress" && event.data.chunkLength) {
          downloaded += event.data.chunkLength;
        } else if (event.event === "Finished") {
          setState((s) => ({ ...s, status: "ready", progress: 100 }));
        }
        if (event.data.contentLength && event.data.contentLength > 0) {
          const pct = Math.min(100, Math.round((downloaded / event.data.contentLength) * 100));
          setState((s) => ({ ...s, progress: pct }));
        }
      });
      setState((s) => ({ ...s, status: "ready", progress: 100 }));
    } catch (err) {
      setState((s) => ({
        ...s,
        status: "error",
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, []);

  const restartApp = useCallback(async () => {
    if (!transport.isTauri) return;
    try {
      const { relaunch } = await import("@tauri-apps/plugin-process");
      await relaunch();
    } catch {
      // fallback: user can restart manually
    }
  }, []);

  const dismiss = useCallback(() => {
    setState({ status: "idle", info: null, progress: 0, error: null });
  }, []);

  useEffect(() => {
    if (!autoCheck || !transport.isTauri) return;

    const delay = setTimeout(() => {
      checkForUpdate();
    }, 5000);

    timerRef.current = setInterval(checkForUpdate, CHECK_INTERVAL_MS);

    return () => {
      clearTimeout(delay);
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [autoCheck, checkForUpdate]);

  return {
    ...state,
    checkForUpdate,
    downloadAndInstall,
    restartApp,
    dismiss,
  };
}
