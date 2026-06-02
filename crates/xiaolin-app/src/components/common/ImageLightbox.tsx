import { useState, useEffect, useCallback, useRef } from "react";
import { X, ZoomIn, ZoomOut, RotateCw } from "lucide-react";

interface LightboxState {
  src: string;
  alt?: string;
}

export function openLightbox(src: string, alt?: string) {
  window.dispatchEvent(
    new CustomEvent("xiaolin:lightbox", { detail: { src, alt } }),
  );
}

export function ImageLightbox() {
  const [state, setState] = useState<LightboxState | null>(null);
  const [scale, setScale] = useState(1);
  const [rotate, setRotate] = useState(0);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const dragging = useRef(false);
  const dragStart = useRef({ x: 0, y: 0 });
  const translateStart = useRef({ x: 0, y: 0 });

  const reset = useCallback(() => {
    setScale(1);
    setRotate(0);
    setTranslate({ x: 0, y: 0 });
  }, []);

  const close = useCallback(() => {
    setState(null);
    reset();
  }, [reset]);

  useEffect(() => {
    const handler = (e: Event) => {
      const { src, alt } = (e as CustomEvent).detail;
      setState({ src, alt });
      reset();
    };
    window.addEventListener("xiaolin:lightbox", handler);
    return () => window.removeEventListener("xiaolin:lightbox", handler);
  }, [reset]);

  useEffect(() => {
    if (!state) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
      if (e.key === "=" || e.key === "+") setScale((s) => Math.min(s + 0.25, 5));
      if (e.key === "-") setScale((s) => Math.max(s - 0.25, 0.25));
      if (e.key === "0") reset();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state, close, reset]);

  const onWheel = useCallback((e: React.WheelEvent) => {
    e.stopPropagation();
    const delta = e.deltaY > 0 ? -0.1 : 0.1;
    setScale((s) => Math.min(Math.max(s + delta, 0.25), 5));
  }, []);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      if (e.button !== 0) return;
      dragging.current = true;
      dragStart.current = { x: e.clientX, y: e.clientY };
      translateStart.current = { ...translate };
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    },
    [translate],
  );

  const onPointerMove = useCallback((e: React.PointerEvent) => {
    if (!dragging.current) return;
    setTranslate({
      x: translateStart.current.x + (e.clientX - dragStart.current.x),
      y: translateStart.current.y + (e.clientY - dragStart.current.y),
    });
  }, []);

  const onPointerUp = useCallback(() => {
    dragging.current = false;
  }, []);

  if (!state) return null;

  const btnClass =
    "flex h-8 w-8 items-center justify-center rounded-full transition-colors hover:bg-white/15";

  return (
    <div
      className="fixed inset-0 z-[10001] flex items-center justify-center"
      style={{ background: "rgba(0, 0, 0, 0.85)" }}
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      <div className="absolute top-4 right-4 flex items-center gap-1">
        <button className={btnClass} onClick={() => setScale((s) => Math.min(s + 0.25, 5))} title="放大">
          <ZoomIn size={16} strokeWidth={1.5} color="rgba(255,255,255,0.8)" />
        </button>
        <button className={btnClass} onClick={() => setScale((s) => Math.max(s - 0.25, 0.25))} title="缩小">
          <ZoomOut size={16} strokeWidth={1.5} color="rgba(255,255,255,0.8)" />
        </button>
        <button className={btnClass} onClick={() => setRotate((r) => r + 90)} title="旋转">
          <RotateCw size={16} strokeWidth={1.5} color="rgba(255,255,255,0.8)" />
        </button>
        <span
          className="mx-1 min-w-[40px] text-center text-[12px] font-medium"
          style={{ color: "rgba(255,255,255,0.6)" }}
        >
          {Math.round(scale * 100)}%
        </span>
        <button className={btnClass} onClick={close} title="关闭">
          <X size={16} strokeWidth={1.5} color="rgba(255,255,255,0.8)" />
        </button>
      </div>

      <img
        src={state.src}
        alt={state.alt || ""}
        draggable={false}
        onWheel={onWheel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        className="select-none"
        style={{
          maxWidth: "90vw",
          maxHeight: "90vh",
          objectFit: "contain",
          transform: `translate(${translate.x}px, ${translate.y}px) scale(${scale}) rotate(${rotate}deg)`,
          cursor: dragging.current ? "grabbing" : "grab",
          transition: dragging.current ? "none" : "transform 150ms ease",
        }}
      />
    </div>
  );
}
