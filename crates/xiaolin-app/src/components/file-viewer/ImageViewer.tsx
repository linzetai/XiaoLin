import {
  useCallback,
  useEffect,
  useRef,
  useState,
  memo,
  type MouseEvent as ReactMouseEvent,
} from "react";
import {
  MagnifyingGlassPlus,
  MagnifyingGlassMinus,
  ArrowsOutSimple,
  ArrowsInSimple,
  Code,
  Image as ImageIcon,
} from "@phosphor-icons/react";
import { readBinaryForViewer } from "../../lib/transport";
import { CodeViewer } from "./CodeViewer";
import { formatFileSize, isSvgPath } from "./file-types";
import { base64ToBlobUrl } from "./blob-utils";

export interface ImageViewerProps {
  filePath: string;
  workDir: string;
  viewMode?: "preview" | "code";
  svgContent?: string;
  onViewModeChange?: (mode: "preview" | "code") => void;
}

const MIN_SCALE = 0.1;
const MAX_SCALE = 5.0;
const WHEEL_ZOOM_FACTOR = 1.1;

function clampScale(value: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, value));
}

function toolbarButtonStyle(active = false): React.CSSProperties {
  return {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    gap: 4,
    padding: "4px 8px",
    border: "1px solid var(--border-primary)",
    borderRadius: 6,
    background: active ? "var(--bg-tertiary)" : "var(--bg-secondary)",
    color: "var(--fill-primary)",
    cursor: "pointer",
    fontSize: 12,
    lineHeight: 1,
  };
}

export const ImageViewer = memo(function ImageViewer({
  filePath,
  workDir,
  viewMode = "preview",
  svgContent = "",
  onViewModeChange,
}: ImageViewerProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const blobUrlRef = useRef<string | null>(null);

  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [blobUrl, setBlobUrl] = useState<string | null>(null);
  const [fileSize, setFileSize] = useState(0);

  const [naturalSize, setNaturalSize] = useState<{ w: number; h: number } | null>(null);
  const [scale, setScale] = useState(1);
  const [translate, setTranslate] = useState({ x: 0, y: 0 });
  const [fitScale, setFitScale] = useState(1);
  const [isDragging, setIsDragging] = useState(false);

  const dragRef = useRef<{ active: boolean; startX: number; startY: number; tx: number; ty: number }>({
    active: false,
    startX: 0,
    startY: 0,
    tx: 0,
    ty: 0,
  });

  const isSvg = isSvgPath(filePath);
  const fileName = filePath.split("/").pop() ?? filePath;

  const computeFit = useCallback(
    (imgW: number, imgH: number) => {
      const container = containerRef.current;
      if (!container || imgW <= 0 || imgH <= 0) return 1;

      const { clientWidth, clientHeight } = container;
      const padding = 24;
      const availW = Math.max(1, clientWidth - padding);
      const availH = Math.max(1, clientHeight - padding);
      return Math.min(availW / imgW, availH / imgH, 1);
    },
    [],
  );

  const applyFitToWindow = useCallback(() => {
    if (!naturalSize) return;
    const fit = computeFit(naturalSize.w, naturalSize.h);
    setFitScale(fit);
    setScale(fit);
    setTranslate({ x: 0, y: 0 });
  }, [computeFit, naturalSize]);

  const applyActualSize = useCallback(() => {
    setScale(1);
    setTranslate({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      setLoading(true);
      setError(null);
      setNaturalSize(null);
      setScale(1);
      setTranslate({ x: 0, y: 0 });

      if (blobUrlRef.current) {
        URL.revokeObjectURL(blobUrlRef.current);
        blobUrlRef.current = null;
      }
      setBlobUrl(null);

      try {
        const result = await readBinaryForViewer(filePath, workDir);
        if (cancelled) return;

        const url = base64ToBlobUrl(result.base64, result.mime);
        blobUrlRef.current = url;
        setBlobUrl(url);
        setFileSize(result.size);
      } catch (err) {
        if (cancelled) return;
        console.warn("[ImageViewer] failed to load image:", filePath, err);
        setError("无法加载图片");
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    void load();

    return () => {
      cancelled = true;
      if (blobUrlRef.current) {
        URL.revokeObjectURL(blobUrlRef.current);
        blobUrlRef.current = null;
      }
    };
  }, [filePath, workDir]);

  useEffect(() => {
    if (!naturalSize) return;
    applyFitToWindow();
  }, [naturalSize, applyFitToWindow]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !naturalSize) return;

    const observer = new ResizeObserver(() => {
      const fit = computeFit(naturalSize.w, naturalSize.h);
      setFitScale((prev) => {
        setScale((prevScale) => {
          if (Math.abs(prevScale - prev) < 0.005) return fit;
          return prevScale;
        });
        return fit;
      });
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, [computeFit, naturalSize]);

  const handleImageLoad = useCallback(
    (e: React.SyntheticEvent<HTMLImageElement>) => {
      const img = e.currentTarget;
      const size = { w: img.naturalWidth, h: img.naturalHeight };
      setNaturalSize(size);
    },
    [],
  );

  useEffect(() => {
    const container = containerRef.current;
    if (!container || viewMode === "code") return;

    const onWheel = (e: WheelEvent) => {
      if (!naturalSize) return;
      e.preventDefault();

      const rect = container.getBoundingClientRect();
      const mouseX = e.clientX - rect.left - rect.width / 2;
      const mouseY = e.clientY - rect.top - rect.height / 2;
      const factor = e.deltaY < 0 ? WHEEL_ZOOM_FACTOR : 1 / WHEEL_ZOOM_FACTOR;

      setScale((prevScale) => {
        const newScale = clampScale(prevScale * factor);
        setTranslate((prev) => {
          const px = (mouseX - prev.x) / prevScale;
          const py = (mouseY - prev.y) / prevScale;
          return {
            x: mouseX - px * newScale,
            y: mouseY - py * newScale,
          };
        });
        return newScale;
      });
    };

    container.addEventListener("wheel", onWheel, { passive: false });
    return () => container.removeEventListener("wheel", onWheel);
  }, [naturalSize, viewMode]);

  const handleMouseDown = useCallback(
    (e: ReactMouseEvent<HTMLDivElement>) => {
      const pan = scale > fitScale * 1.01 || scale > 1.01;
      if (viewMode === "code" || e.button !== 0 || !pan) return;
      e.preventDefault();
      dragRef.current = {
        active: true,
        startX: e.clientX,
        startY: e.clientY,
        tx: translate.x,
        ty: translate.y,
      };
      setIsDragging(true);
    },
    [translate.x, translate.y, viewMode, scale, fitScale],
  );

  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!dragRef.current.active) return;
      const dx = e.clientX - dragRef.current.startX;
      const dy = e.clientY - dragRef.current.startY;
      setTranslate({
        x: dragRef.current.tx + dx,
        y: dragRef.current.ty + dy,
      });
    };

    const handleMouseUp = () => {
      if (dragRef.current.active) {
        dragRef.current.active = false;
        setIsDragging(false);
      }
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
    return () => {
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };
  }, []);

  const zoomBy = useCallback((factor: number) => {
    setScale((prev) => clampScale(prev * factor));
  }, []);

  const scalePercent = Math.round(scale * 100);
  const canPan = scale > fitScale * 1.01 || scale > 1.01;
  const showCode = isSvg && viewMode === "code";

  if (showCode) {
    return (
      <div
        style={{
          display: "flex",
          flexDirection: "column",
          height: "100%",
          minHeight: 0,
        }}
      >
        <ImageToolbar
          fileName={fileName}
          naturalSize={naturalSize}
          fileSize={fileSize}
          scalePercent={scalePercent}
          isSvg={isSvg}
          viewMode={viewMode}
          onViewModeChange={onViewModeChange}
          onFit={applyFitToWindow}
          onActualSize={applyActualSize}
          onZoomIn={() => zoomBy(WHEEL_ZOOM_FACTOR)}
          onZoomOut={() => zoomBy(1 / WHEEL_ZOOM_FACTOR)}
          hideZoomControls
        />
        <CodeViewer content={svgContent} language="xml" />
      </div>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
      }}
    >
      <ImageToolbar
        fileName={fileName}
        naturalSize={naturalSize}
        fileSize={fileSize}
        scalePercent={scalePercent}
        isSvg={isSvg}
        viewMode={viewMode}
        onViewModeChange={onViewModeChange}
        onFit={applyFitToWindow}
        onActualSize={applyActualSize}
        onZoomIn={() => zoomBy(WHEEL_ZOOM_FACTOR)}
        onZoomOut={() => zoomBy(1 / WHEEL_ZOOM_FACTOR)}
      />

      <div
        ref={containerRef}
        onMouseDown={handleMouseDown}
        style={{
          flex: 1,
          minHeight: 0,
          overflow: "hidden",
          position: "relative",
          background: "var(--bg-primary)",
          cursor: isDragging ? "grabbing" : canPan ? "grab" : "default",
        }}
      >
        {loading && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--fill-secondary)",
              fontSize: 13,
            }}
          >
            加载中…
          </div>
        )}
        {error && !loading && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--fill-secondary)",
              fontSize: 13,
            }}
          >
            {error}
          </div>
        )}
        {!loading && !error && blobUrl && (
          <div
            style={{
              position: "absolute",
              left: "50%",
              top: "50%",
              transform: `translate(calc(-50% + ${translate.x}px), calc(-50% + ${translate.y}px)) scale(${scale})`,
              transformOrigin: "center center",
            }}
          >
            <img
              src={blobUrl}
              alt={fileName}
              draggable={false}
              onLoad={handleImageLoad}
              style={{
                display: "block",
                maxWidth: "none",
                maxHeight: "none",
                userSelect: "none",
                pointerEvents: "none",
              }}
            />
          </div>
        )}
      </div>
    </div>
  );
});

interface ImageToolbarProps {
  fileName: string;
  naturalSize: { w: number; h: number } | null;
  fileSize: number;
  scalePercent: number;
  isSvg: boolean;
  viewMode: "preview" | "code";
  onViewModeChange?: (mode: "preview" | "code") => void;
  onFit: () => void;
  onActualSize: () => void;
  onZoomIn: () => void;
  onZoomOut: () => void;
  hideZoomControls?: boolean;
}

function ImageToolbar({
  fileName,
  naturalSize,
  fileSize,
  scalePercent,
  isSvg,
  viewMode,
  onViewModeChange,
  onFit,
  onActualSize,
  onZoomIn,
  onZoomOut,
  hideZoomControls = false,
}: ImageToolbarProps) {
  const dimText =
    naturalSize && naturalSize.w > 0
      ? `${naturalSize.w}×${naturalSize.h} px`
      : "—";
  const sizeText = fileSize > 0 ? formatFileSize(fileSize) : "—";

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: 8,
        padding: "6px 10px",
        borderBottom: "1px solid var(--border-primary)",
        background: "var(--bg-secondary)",
        flexShrink: 0,
        flexWrap: "wrap",
      }}
    >
      <span
        style={{
          fontSize: 12,
          fontWeight: 600,
          color: "var(--fill-primary)",
          fontFamily: "var(--font-mono)",
          marginRight: 4,
        }}
      >
        {fileName}
      </span>

      <span style={{ fontSize: 11, color: "var(--fill-secondary)" }}>
        {dimText} · {sizeText}
      </span>

      <div style={{ flex: 1 }} />

      {!hideZoomControls && (
        <>
          <button type="button" title="缩小" style={toolbarButtonStyle()} onClick={onZoomOut}>
            <MagnifyingGlassMinus size={14} />
          </button>
          <span style={{ fontSize: 11, color: "var(--fill-secondary)", minWidth: 40, textAlign: "center" }}>
            {scalePercent}%
          </span>
          <button type="button" title="放大" style={toolbarButtonStyle()} onClick={onZoomIn}>
            <MagnifyingGlassPlus size={14} />
          </button>
          <button type="button" title="适应窗口" style={toolbarButtonStyle()} onClick={onFit}>
            <ArrowsOutSimple size={14} />
          </button>
          <button type="button" title="原始大小 (100%)" style={toolbarButtonStyle()} onClick={onActualSize}>
            <ArrowsInSimple size={14} />
          </button>
        </>
      )}

      {isSvg && onViewModeChange && (
        <>
          <button
            type="button"
            title="图片预览"
            style={toolbarButtonStyle(viewMode === "preview")}
            onClick={() => onViewModeChange("preview")}
          >
            <ImageIcon size={14} />
          </button>
          <button
            type="button"
            title="源码查看"
            style={toolbarButtonStyle(viewMode === "code")}
            onClick={() => onViewModeChange("code")}
          >
            <Code size={14} />
          </button>
        </>
      )}
    </div>
  );
}
