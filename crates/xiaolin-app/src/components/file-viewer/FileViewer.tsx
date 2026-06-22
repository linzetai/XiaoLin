import { memo } from "react";
import { isImagePath } from "../../lib/file-utils";
import type { OpenFile } from "../../lib/stores/file-viewer-store";
import { CodeViewer, MarkdownViewer, ImageViewer } from "./index";

export interface FileViewerProps {
  file: OpenFile;
  workDir: string;
  wordWrap: boolean;
  onViewModeChange: (mode: "code" | "preview") => void;
}

export const FileViewer = memo(function FileViewer({
  file,
  workDir,
  wordWrap,
  onViewModeChange,
}: FileViewerProps) {
  if (isImagePath(file.path)) {
    return (
      <ImageViewer
        filePath={file.path}
        workDir={workDir}
        viewMode={file.viewMode}
        svgContent={file.content}
        onViewModeChange={onViewModeChange}
      />
    );
  }

  if (file.language === "markdown") {
    return (
      <MarkdownViewer
        content={file.content}
        filePath={file.path}
        workDir={workDir}
        viewMode={file.viewMode}
        line={file.line}
        wordWrap={wordWrap}
      />
    );
  }

  return (
    <CodeViewer
      content={file.content}
      language={file.language}
      line={file.line}
      wordWrap={wordWrap}
    />
  );
});
