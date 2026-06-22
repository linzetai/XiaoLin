import { FileViewerTab } from "../file-viewer/FileViewerTab";

/**
 * Workspace tab adapter for the built-in file viewer.
 * The `xiaolin:open-file` global event listener is in AppShell.tsx
 * so it stays mounted regardless of which tab is active.
 */
export function FilesTabContent() {
  return <FileViewerTab />;
}
