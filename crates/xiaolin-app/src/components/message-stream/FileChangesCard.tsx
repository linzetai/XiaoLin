import { useState, useCallback, type CSSProperties } from "react";
import { CaretRight } from "@phosphor-icons/react";
import type { FileChangeSummary } from "./edit-result-utils";

const VISIBLE_LIMIT = 5;

const cardStyle: CSSProperties = {
  border: "1px solid var(--border, var(--separator))",
  borderRadius: 12,
  overflow: "hidden",
  margin: "10px 0 16px",
};

const topStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  padding: "8px 14px",
  fontSize: 12,
  fontWeight: 500,
  color: "var(--fill-secondary)",
};

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 6,
  padding: "6px 14px",
  fontSize: 12,
  fontFamily: "var(--font-mono)",
  color: "var(--fill-secondary)",
  borderTop: "1px solid var(--border-subtle, var(--separator))",
  cursor: "pointer",
  transition: "background 0.1s",
};

export function FileChangesCard({ summary }: { summary: FileChangeSummary }) {
  const [expanded, setExpanded] = useState(false);
  const visibleFiles = expanded ? summary.files : summary.files.slice(0, VISIBLE_LIMIT);
  const hiddenCount = summary.files.length - VISIBLE_LIMIT;

  const handleFileClick = useCallback((path: string) => {
    window.dispatchEvent(
      new CustomEvent("xiaolin:open-file", {
        detail: { path, source: "file-changes-card" },
      }),
    );
  }, []);

  return (
    <div style={cardStyle}>
      <div style={topStyle}>
        <span>{summary.totalFiles} file{summary.totalFiles > 1 ? "s" : ""} changed</span>
        {(summary.totalAdded > 0 || summary.totalRemoved > 0) && (
          <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, marginLeft: 6 }}>
            <span style={{ color: "var(--green-text, var(--green))" }}>+{summary.totalAdded}</span>
            {" "}
            <span style={{ color: "var(--red-text, var(--red))" }}>-{summary.totalRemoved}</span>
          </span>
        )}
        <button
          type="button"
          disabled
          title="Undo is not yet available"
          style={{
            marginLeft: "auto",
            fontSize: 11,
            color: "var(--fill-quaternary)",
            background: "none",
            border: "none",
            cursor: "not-allowed",
            padding: "2px 4px",
            borderRadius: 4,
            opacity: 0.5,
          }}
        >
          Undo
        </button>
      </div>

      {visibleFiles.map((file) => {
        const fileName = file.path.split("/").pop() || file.path;
        return (
          <div
            key={file.path}
            role="button"
            tabIndex={0}
            style={rowStyle}
            onClick={() => handleFileClick(file.path)}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                handleFileClick(file.path);
              }
            }}
            onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
            onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
          >
            <span
              style={{
                width: 6,
                height: 6,
                borderRadius: "50%",
                background: "var(--orange, #ED8936)",
                flexShrink: 0,
              }}
            />
            <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {fileName}
            </span>
            {(file.linesAdded > 0 || file.linesRemoved > 0) && (
              <span style={{ fontSize: 11, flexShrink: 0 }}>
                <span style={{ color: "var(--green-text, var(--green))" }}>+{file.linesAdded}</span>
                {" "}
                <span style={{ color: "var(--red-text, var(--red))" }}>-{file.linesRemoved}</span>
              </span>
            )}
            <CaretRight style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />
          </div>
        );
      })}

      {!expanded && hiddenCount > 0 && (
        <div
          style={{
            ...rowStyle,
            justifyContent: "center",
            color: "var(--fill-tertiary)",
            fontSize: 11,
            cursor: "pointer",
          }}
          onClick={() => setExpanded(true)}
          onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
          onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
        >
          show {hiddenCount} more
        </div>
      )}
    </div>
  );
}
