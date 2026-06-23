import { useRef, useEffect, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { Globe } from "@phosphor-icons/react";
import { useBrowserStore, browserResizeWebview } from "../../lib/stores/browser-store";
import { AgentControlOverlay } from "./AgentControlOverlay";

interface BrowserPlaceholderProps {
  pageId: string | null;
  webviewVisible: boolean;
}

export function BrowserPlaceholder({ pageId, webviewVisible }: BrowserPlaceholderProps) {
  const { t } = useTranslation("browser");
  const containerRef = useRef<HTMLDivElement>(null);
  const rafRef = useRef<number | null>(null);
  const page = useBrowserStore((s) => (pageId ? s.pages[pageId] : null));
  const layoutTransitioning = useBrowserStore((s) => s.layoutTransitioning);

  const reportLayout = useCallback(() => {
    if (!pageId || !webviewVisible || layoutTransitioning) return;
    const el = containerRef.current;
    if (!el) return;

    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;

    void browserResizeWebview(pageId, rect.x, rect.y, rect.width, rect.height, window.devicePixelRatio);
  }, [pageId, webviewVisible, layoutTransitioning]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const observer = new ResizeObserver(() => {
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(() => {
        rafRef.current = null;
        reportLayout();
      });
    });

    observer.observe(el);
    reportLayout();

    return () => {
      observer.disconnect();
      if (rafRef.current != null) cancelAnimationFrame(rafRef.current);
    };
  }, [reportLayout]);

  useEffect(() => {
    reportLayout();
  }, [webviewVisible, pageId, layoutTransitioning, reportLayout]);

  const isEmpty = !page?.url || page.url === "about:blank";
  const isLoading = page?.loadState.state === "loading";
  const isFailed = page?.loadState.state === "failed";

  return (
    <div
      ref={containerRef}
      style={{
        flex: 1,
        minHeight: 0,
        position: "relative",
        background: "var(--bg-secondary)",
        overflow: "hidden",
        marginLeft: 4,
      }}
    >
      {isEmpty && !isLoading && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            gap: 8,
            color: "var(--fill-quaternary)",
            pointerEvents: "none",
            zIndex: 1,
          }}
        >
          <Globe size={32} weight="thin" />
          <span style={{ fontSize: 13 }}>{t("enterUrlToStart")}</span>
        </div>
      )}
      {isFailed && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            padding: 16,
            color: "var(--fill-tertiary)",
            fontSize: 12,
            textAlign: "center",
            pointerEvents: "none",
            zIndex: 1,
          }}
        >
          {(page.loadState.state === "failed" && page.loadState.message) || t("loadFailed")}
        </div>
      )}
      {pageId && <AgentControlOverlay pageId={pageId} />}
    </div>
  );
}
