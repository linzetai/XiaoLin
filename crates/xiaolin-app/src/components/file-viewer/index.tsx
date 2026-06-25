import { lazy, Suspense, type ComponentProps } from "react";
import type { MarkdownViewer as MarkdownViewerComponent } from "./MarkdownViewer";
import type { ImageViewer as ImageViewerComponent } from "./ImageViewer";
export { CodeViewer } from "./CodeViewer";

const MarkdownViewerLazy = lazy(() =>
  import("./MarkdownViewer").then((m) => ({ default: m.MarkdownViewer })),
);

const ImageViewerLazy = lazy(() =>
  import("./ImageViewer").then((m) => ({ default: m.ImageViewer })),
);

function ViewerFallback() {
  return (
    <div
      className="animate-pulse rounded"
      style={{ background: "var(--bg-tertiary)", height: "100%", minHeight: 120 }}
    />
  );
}

export function MarkdownViewer(props: ComponentProps<typeof MarkdownViewerComponent>) {
  return (
    <Suspense fallback={<ViewerFallback />}>
      <MarkdownViewerLazy {...props} />
    </Suspense>
  );
}

export function ImageViewer(props: ComponentProps<typeof ImageViewerComponent>) {
  return (
    <Suspense fallback={<ViewerFallback />}>
      <ImageViewerLazy {...props} />
    </Suspense>
  );
}

export type { CodeViewerProps } from "./CodeViewer";
export type { MarkdownViewerProps } from "./MarkdownViewer";
export type { ImageViewerProps } from "./ImageViewer";
export { isImagePath, isSvgPath, formatFileSize } from "./file-types";
