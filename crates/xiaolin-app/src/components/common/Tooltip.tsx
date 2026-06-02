import { useState, useEffect, useRef, useCallback, useLayoutEffect } from "react";

type Placement = "top" | "bottom" | "left" | "right";

function bestPlacement(rect: DOMRect): Placement {
  const nearLeft = rect.left < 80;
  const nearRight = window.innerWidth - rect.right < 80;
  const nearTop = rect.top < 48;

  if (nearLeft && window.innerWidth - rect.right >= 80) return "right";
  if (nearRight && rect.left >= 80) return "left";
  if (nearTop && window.innerHeight - rect.bottom >= 40) return "bottom";
  if (rect.top >= 40) return "top";
  return "bottom";
}

const GAP = 6;

export function TooltipProvider() {
  const [tip, setTip] = useState<{ text: string; anchorRect: DOMRect; placement: Placement } | null>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout>>(undefined);
  const targetRef = useRef<HTMLElement | null>(null);
  const savedTitleRef = useRef("");
  const tooltipRef = useRef<HTMLDivElement>(null);

  const show = useCallback((el: HTMLElement, text: string) => {
    const rect = el.getBoundingClientRect();
    const placement = bestPlacement(rect);
    setTip({ text, anchorRect: rect, placement });
    setPos(null);
  }, []);

  const hide = useCallback(() => {
    clearTimeout(timerRef.current);
    setTip(null);
    setPos(null);
    if (targetRef.current && savedTitleRef.current) {
      targetRef.current.setAttribute("title", savedTitleRef.current);
      targetRef.current = null;
      savedTitleRef.current = "";
    }
  }, []);

  useLayoutEffect(() => {
    if (!tip || !tooltipRef.current) return;
    const tt = tooltipRef.current;
    const tw = tt.offsetWidth;
    const th = tt.offsetHeight;
    const r = tip.anchorRect;
    let left: number, top: number;

    switch (tip.placement) {
      case "top":
        left = Math.round(r.left + r.width / 2 - tw / 2);
        top = Math.round(r.top - GAP - th);
        break;
      case "bottom":
        left = Math.round(r.left + r.width / 2 - tw / 2);
        top = Math.round(r.bottom + GAP);
        break;
      case "right":
        left = Math.round(r.right + GAP);
        top = Math.round(r.top + r.height / 2 - th / 2);
        break;
      case "left":
        left = Math.round(r.left - GAP - tw);
        top = Math.round(r.top + r.height / 2 - th / 2);
        break;
    }

    left = Math.max(4, Math.min(left, window.innerWidth - tw - 4));
    top = Math.max(4, Math.min(top, window.innerHeight - th - 4));

    setPos({ left, top });
  }, [tip]);

  useEffect(() => {
    const onOver = (e: MouseEvent) => {
      const el = (e.target as HTMLElement).closest?.("[title]") as HTMLElement | null;
      if (!el) return;
      const title = el.getAttribute("title");
      if (!title?.trim()) return;

      if (targetRef.current === el) return;

      hide();

      savedTitleRef.current = title;
      targetRef.current = el;
      el.removeAttribute("title");

      timerRef.current = setTimeout(() => show(el, title), 400);
    };

    const onOut = (e: MouseEvent) => {
      if (!targetRef.current) return;
      if (
        e.target === targetRef.current ||
        targetRef.current.contains(e.target as Node)
      ) {
        if (!targetRef.current.contains(e.relatedTarget as Node)) {
          hide();
        }
      }
    };

    const onScroll = () => hide();

    document.addEventListener("mouseover", onOver, true);
    document.addEventListener("mouseout", onOut, true);
    document.addEventListener("scroll", onScroll, true);
    document.addEventListener("mousedown", hide, true);
    return () => {
      document.removeEventListener("mouseover", onOver, true);
      document.removeEventListener("mouseout", onOut, true);
      document.removeEventListener("scroll", onScroll, true);
      document.removeEventListener("mousedown", hide, true);
      clearTimeout(timerRef.current);
    };
  }, [show, hide]);

  if (!tip) return null;

  return (
    <div
      ref={tooltipRef}
      className="pointer-events-none fixed z-[10000] whitespace-nowrap px-2 py-1 text-center text-[12px] font-medium leading-snug"
      style={{
        left: pos?.left ?? -9999,
        top: pos?.top ?? -9999,
        opacity: pos ? 1 : 0,
        background: "rgba(30, 30, 30, 0.92)",
        color: "rgba(255, 255, 255, 0.9)",
        borderRadius: "4px",
        boxShadow: "0 2px 8px rgba(0,0,0,0.25)",
      }}
    >
      {tip.text}
    </div>
  );
}
