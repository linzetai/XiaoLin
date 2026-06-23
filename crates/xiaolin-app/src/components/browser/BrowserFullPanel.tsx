import { BrowserPanelBody } from "./BrowserTabContent";

export function BrowserFullPanel() {
  return (
    <div
      className="browser-full-panel"
      style={{
        flex: 1,
        minWidth: 0,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        transition: "flex 0.3s ease, opacity 0.3s ease",
        overflow: "hidden",
      }}
    >
      <BrowserPanelBody />
    </div>
  );
}
