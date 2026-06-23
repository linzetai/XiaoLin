import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import "./browser-progress-bar.css";

export interface BrowserProgressBarProps {
  loadState: "loading" | "ready" | "failed";
  /** 每次新导航变化时递增，用于在 loadState 仍为 loading 时重置动画 */
  resetKey?: string | number;
}

type FillPhase = "loading" | "trickling" | "completing" | "done";

export function BrowserProgressBar({ loadState, resetKey }: BrowserProgressBarProps) {
  const [visible, setVisible] = useState(false);
  const [fading, setFading] = useState(false);
  const [fillPhase, setFillPhase] = useState<FillPhase>("loading");
  const [animKey, setAnimKey] = useState(0);
  const timersRef = useRef<number[]>([]);
  const isActiveRef = useRef(false);
  const fillRef = useRef<HTMLDivElement>(null);

  const clearTimers = useCallback(() => {
    for (const id of timersRef.current) {
      window.clearTimeout(id);
    }
    timersRef.current = [];
  }, []);

  const schedule = useCallback((fn: () => void, ms: number) => {
    const id = window.setTimeout(fn, ms);
    timersRef.current.push(id);
  }, []);

  const hide = useCallback(() => {
    setVisible(false);
    setFading(false);
    setFillPhase("loading");
  }, []);

  useEffect(() => {
    clearTimers();

    if (loadState === "loading") {
      isActiveRef.current = true;
      setAnimKey((k) => k + 1);
      setVisible(true);
      setFading(false);
      setFillPhase("loading");
      schedule(() => setFillPhase("trickling"), 15000);
    } else if (loadState === "failed") {
      if (isActiveRef.current) {
        isActiveRef.current = false;
        setFading(true);
        schedule(() => hide(), 150);
      }
    } else if (loadState === "ready") {
      if (isActiveRef.current) {
        isActiveRef.current = false;
        setFillPhase("completing");
        schedule(() => {
          setFillPhase("done");
          setFading(true);
          schedule(() => hide(), 150);
        }, 200);
      }
    }

    return clearTimers;
  }, [loadState, resetKey, clearTimers, schedule, hide]);

  useLayoutEffect(() => {
    if (fillPhase !== "completing") return;
    const el = fillRef.current;
    if (!el) return;

    const currentWidth = getComputedStyle(el).width;
    el.style.animation = "none";
    el.style.width = currentWidth;
    void el.offsetWidth;
    el.style.transition = "width 200ms ease-out";
    el.style.width = "100%";
  }, [fillPhase]);

  if (!visible) {
    return null;
  }

  const isBusy = fillPhase === "loading" || fillPhase === "trickling";

  return (
    <div
      style={{
        position: "relative",
        height: 2,
        pointerEvents: "none",
        overflow: "hidden",
      }}
      className={fading ? "browser-progress-bar--fading" : undefined}
      role="progressbar"
      aria-busy={isBusy}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <div
        key={animKey}
        ref={fillRef}
        className={`browser-progress-bar__fill browser-progress-bar__fill--${fillPhase}`}
        style={{
          height: 2,
          backgroundColor: "var(--tint)",
          width: fillPhase === "loading" ? "0%" : undefined,
        }}
      />
    </div>
  );
}
