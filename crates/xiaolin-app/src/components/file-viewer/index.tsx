import { lazy, Suspense, type ComponentProps } from "react";
import type { CodeViewer as CodeViewerComponent } from "./CodeViewer";

const CodeViewerLazy = lazy(() =>
  import("./CodeViewer").then((m) => ({ default: m.CodeViewer })),
);

function CodeViewerFallback() {
  return (
    <div
      className="animate-pulse rounded"
      style={{ background: "var(--bg-tertiary)", height: "100%", minHeight: 120 }}
    />
  );
}

export function CodeViewer(props: ComponentProps<typeof CodeViewerComponent>) {
  return (
    <Suspense fallback={<CodeViewerFallback />}>
      <CodeViewerLazy {...props} />
    </Suspense>
  );
}

export type { CodeViewerProps } from "./CodeViewer";
export { languageFromPath, languageFromExtension, EXT_TO_LANG } from "./cm-languages";
