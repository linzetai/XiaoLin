import { useEffect, useState } from "react";

/**
 * Live wall-clock timer shared by sub-agent surfaces (SubAgentCard, CoordinatorPanel).
 *
 * While `isActive`, ticks every `intervalMs` and returns `baseMs` plus the time
 * elapsed since the hook (re)started. When inactive, returns `baseMs` unchanged
 * (the final/persisted elapsed). Keeping this in one place avoids duplicated
 * setInterval logic and divergent timing behaviour across components.
 */
export function useElapsedTimer(
  isActive: boolean,
  baseMs = 0,
  intervalMs = 1000,
): number {
  const [elapsed, setElapsed] = useState(baseMs);

  useEffect(() => {
    if (!isActive) {
      setElapsed(baseMs);
      return;
    }
    const start = Date.now();
    setElapsed(baseMs);
    const id = setInterval(() => {
      setElapsed(baseMs + (Date.now() - start));
    }, intervalMs);
    return () => clearInterval(id);
  }, [isActive, baseMs, intervalMs]);

  return elapsed;
}

/** Format a millisecond duration as a compact `123ms` / `1.5s` string. */
export function formatElapsed(ms?: number): string {
  if (ms == null) return "—";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}
