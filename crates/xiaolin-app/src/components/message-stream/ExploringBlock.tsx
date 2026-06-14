import { useState, useMemo } from "react";
import { useTranslation } from "react-i18next";
import { type ToolCall, extractKeyInfo, getToolCategory } from "./StepIndicator";

interface ExploringBlockProps {
  tools: ToolCall[];
  streaming?: boolean;
}

export function isExploringEligible(tool: ToolCall): boolean {
  const cat = getToolCategory(tool.name);
  return cat === "read" || cat === "search";
}

export function ExploringBlock({ tools, streaming }: ExploringBlockProps) {
  const { t } = useTranslation("chat");
  const [expanded, setExpanded] = useState(false);

  const isActive = streaming && tools.some((t) => t.status === "running");
  const hasError = tools.some((t) => t.status === "error");

  const grouped = useMemo(() => {
    const reads: string[] = [];
    const searches: { query: string; path?: string }[] = [];
    const lists: string[] = [];

    for (const tool of tools) {
      const cat = getToolCategory(tool.name);
      const info = extractKeyInfo(tool);
      if (cat === "read") {
        const short = info ? info.split("/").pop() ?? info : tool.name;
        reads.push(short);
      } else if (cat === "search") {
        const args = tool.args as Record<string, unknown> | undefined;
        searches.push({
          query: (args?.pattern as string) ?? (args?.query as string) ?? info ?? "",
          path: (args?.path as string) ?? undefined,
        });
      } else {
        lists.push(info ?? tool.name);
      }
    }
    return { reads, searches, lists };
  }, [tools]);

  return (
    <div className="my-0.5">
      {/* Header */}
      <button
        type="button"
        className="flex items-center gap-1.5 px-1 py-0.5 rounded text-left transition-colors duration-100 w-full"
        style={{ minHeight: "24px" }}
        onClick={() => setExpanded((v) => !v)}
        onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "var(--step-hover-bg)"; }}
        onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = ""; }}
      >
        {/* Status dot */}
        <span className="flex h-[14px] w-[14px] shrink-0 items-center justify-center">
          {isActive ? (
            <span
              className="inline-block h-[6px] w-[6px] rounded-full"
              style={{
                borderWidth: "1.5px",
                borderStyle: "solid",
                borderColor: "var(--tint) transparent transparent transparent",
                animation: "spin 0.8s linear infinite",
              }}
            />
          ) : hasError ? (
            <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: "var(--red)" }} />
          ) : (
            <span className="inline-block h-[6px] w-[6px] rounded-full" style={{ background: "var(--green)" }} />
          )}
        </span>

        <span className="text-[12px] font-medium" style={{ color: "var(--fill-tertiary)" }}>
          {isActive ? t("exploring_active", "Exploring") : t("exploring_done", "Explored")}
        </span>

        <span className="text-[11px] tabular-nums" style={{ color: "var(--fill-quaternary)" }}>
          {tools.length} {t("exploring_items", "项")}
        </span>

        {/* Chevron */}
        <svg
          width={10}
          height={10}
          viewBox="0 0 10 10"
          fill="none"
          className="ml-auto shrink-0 transition-transform duration-150"
          style={{ transform: expanded ? "rotate(90deg)" : "rotate(0deg)", color: "var(--fill-quaternary)" }}
        >
          <path d="M3.5 2L7 5L3.5 8" stroke="currentColor" strokeWidth={1.2} strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>

      {/* Content lines — collapsed hides detail, expanded shows full */}
      <div
        className="pl-5 text-[11px]"
        style={{
          color: "var(--fill-quaternary)",
          fontFamily: "var(--font-mono)",
          maxHeight: expanded ? 200 : 22,
          overflow: "hidden",
          transition: "max-height 200ms ease-out",
        }}
      >
        {grouped.reads.length > 0 && (
          <div className="flex items-baseline gap-1 py-[1px]">
            <span className="shrink-0" style={{ color: "var(--tint)" }}>Read</span>
            <span className="truncate min-w-0">
              {expanded ? grouped.reads.join(", ") : grouped.reads.length <= 3 ? grouped.reads.join(", ") : `${grouped.reads.slice(0, 3).join(", ")} +${grouped.reads.length - 3}`}
            </span>
          </div>
        )}
        {grouped.searches.length > 0 && grouped.searches.map((s, i) => (
          <div key={i} className="flex items-baseline gap-1 py-[1px]">
            <span className="shrink-0" style={{ color: "var(--tint)" }}>Search</span>
            <span className="truncate min-w-0">
              {s.query}{s.path ? <span className="opacity-50"> in {s.path}</span> : null}
            </span>
          </div>
        ))}
        {grouped.lists.length > 0 && (
          <div className="flex items-baseline gap-1 py-[1px]">
            <span className="shrink-0" style={{ color: "var(--tint)" }}>List</span>
            <span className="truncate min-w-0">{grouped.lists.join(", ")}</span>
          </div>
        )}
      </div>
    </div>
  );
}
