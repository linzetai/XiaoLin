import { lazy, Suspense, type ComponentProps } from "react";
import type { CodeViewer as CodeViewerComponent } from "./CodeViewer";
import type { MarkdownViewer as MarkdownViewerComponent } from "./MarkdownViewer";

const CodeViewerLazy = lazy(() =>
  import("./CodeViewer").then((m) => ({ default: m.CodeViewer })),
);

const MarkdownViewerLazy = lazy(() =>
  import("./MarkdownViewer").then((m) => ({ default: m.MarkdownViewer })),
);

function ViewerFallback() {
  return (
    <div
      className="animate-pulse rounded"
      style={{ background: "var(--bg-tertiary)", height: "100%", minHeight: 120 }}
    />
  );
}

export function CodeViewer(props: ComponentProps<typeof CodeViewerComponent>) {
  return (
    <Suspense fallback={<ViewerFallback />}>
      <CodeViewerLazy {...props} />
    </Suspense>
  );
}

export function MarkdownViewer(props: ComponentProps<typeof MarkdownViewerComponent>) {
  return (
    <Suspense fallback={<ViewerFallback />}>
      <MarkdownViewerLazy {...props} />
    </Suspense>
  );
}

export type { CodeViewerProps } from "./CodeViewer";
export type { MarkdownViewerProps } from "./MarkdownViewer";
