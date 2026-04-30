import { useState, useEffect, useRef } from "react";
import * as transport from "./transport";

function mimeFromExt(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase();
  if (ext === "jpg" || ext === "jpeg") return "image/jpeg";
  if (ext === "webp") return "image/webp";
  return "image/png";
}

const avatarCache = new Map<string, { url: string; refCount: number; loading?: Promise<string | null> }>();

async function resolveAvatarUrl(filePath: string): Promise<string | null> {
  if (!transport.isTauri || !filePath) return null;
  try {
    const { readFile } = await import("@tauri-apps/plugin-fs");
    const bytes = await readFile(filePath);
    const blob = new Blob([bytes], { type: mimeFromExt(filePath) });
    return URL.createObjectURL(blob);
  } catch {
    try {
      const { convertFileSrc } = await import("@tauri-apps/api/core");
      return convertFileSrc(filePath);
    } catch {
      return null;
    }
  }
}

function acquireAvatar(filePath: string): Promise<string | null> {
  const cached = avatarCache.get(filePath);
  if (cached) {
    cached.refCount++;
    if (cached.url) return Promise.resolve(cached.url);
    if (cached.loading) return cached.loading;
  }
  const entry = { url: "", refCount: cached ? cached.refCount : 1, loading: undefined as Promise<string | null> | undefined };
  const p = resolveAvatarUrl(filePath).then((result) => {
    if (result) entry.url = result;
    entry.loading = undefined;
    return result;
  });
  entry.loading = p;
  avatarCache.set(filePath, entry);
  return p;
}

function releaseAvatar(filePath: string) {
  const cached = avatarCache.get(filePath);
  if (!cached) return;
  cached.refCount--;
  if (cached.refCount <= 0) {
    if (cached.url && cached.url.startsWith("blob:")) {
      URL.revokeObjectURL(cached.url);
    }
    avatarCache.delete(filePath);
  }
}

export async function loadAvatarBlobUrl(filePath: string): Promise<string | null> {
  return acquireAvatar(filePath);
}

/**
 * Resolves a local file path to a displayable blob: URL.
 * Uses shared reference-counted cache — same path across multiple
 * components reads the file system only once.
 */
export function useAvatarUrl(filePath: string | undefined | null): string | undefined {
  const [url, setUrl] = useState<string | undefined>(undefined);
  const acquiredRef = useRef<string | null>(null);

  useEffect(() => {
    if (acquiredRef.current) {
      releaseAvatar(acquiredRef.current);
      acquiredRef.current = null;
    }
    setUrl(undefined);

    if (!filePath) return;
    let cancelled = false;
    acquireAvatar(filePath).then((result) => {
      if (cancelled || !result) return;
      acquiredRef.current = filePath;
      setUrl(result);
    });
    return () => {
      cancelled = true;
      if (acquiredRef.current) {
        releaseAvatar(acquiredRef.current);
        acquiredRef.current = null;
      }
    };
  }, [filePath]);

  return url;
}
