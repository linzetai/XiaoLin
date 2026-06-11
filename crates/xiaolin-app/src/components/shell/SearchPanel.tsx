import { useRef, useEffect, useCallback, useMemo, type CSSProperties } from "react";
import { MagnifyingGlass, X, SpinnerGap, User, Robot, CaretDown } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { useSearchStore, type SearchResult } from "../../lib/stores/search-store";
import { useChatMetaStore } from "../../lib/stores";

function formatRelativeTime(ts: string): string {
  if (!ts) return "";
  const d = new Date(ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts) ? ts : ts.replace(" ", "T") + "Z");
  const diff = Date.now() - d.getTime();
  if (diff < 0 || Number.isNaN(diff)) return "";
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d`;
  const weeks = Math.floor(days / 7);
  if (weeks < 5) return `${weeks}w`;
  return `${Math.floor(days / 30)}mo`;
}

function parseSnippet(snippet: string): Array<{ text: string; highlight: boolean }> {
  const parts: Array<{ text: string; highlight: boolean }> = [];
  const re = /<b>(.*?)<\/b>/gi;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(snippet)) !== null) {
    if (m.index > last) {
      parts.push({ text: snippet.slice(last, m.index), highlight: false });
    }
    parts.push({ text: m[1], highlight: true });
    last = m.index + m[0].length;
  }
  if (last < snippet.length) {
    parts.push({ text: snippet.slice(last), highlight: false });
  }
  if (parts.length === 0) {
    parts.push({ text: snippet, highlight: false });
  }
  return parts;
}

function workDirLabel(path: string | null): string {
  if (!path) return "";
  const segments = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return segments[segments.length - 1] ?? path;
}

type DatePreset = "today" | "week" | "month" | null;

function datePresetRange(preset: DatePreset): { date_from?: string; date_to?: string } {
  if (!preset) return {};
  const now = new Date();
  const to = now.toISOString().slice(0, 10);
  if (preset === "today") return { date_from: to, date_to: to };
  const from = new Date(now);
  if (preset === "week") from.setDate(from.getDate() - 7);
  if (preset === "month") from.setDate(from.getDate() - 30);
  return { date_from: from.toISOString().slice(0, 10), date_to: to };
}

function activeDatePreset(filters: { date_from?: string; date_to?: string }): DatePreset {
  const range = datePresetRange("today");
  if (filters.date_from === range.date_from && filters.date_to === range.date_to) return "today";
  const week = datePresetRange("week");
  if (filters.date_from === week.date_from && filters.date_to === week.date_to) return "week";
  const month = datePresetRange("month");
  if (filters.date_from === month.date_from && filters.date_to === month.date_to) return "month";
  return null;
}

const chipStyle = (active: boolean): CSSProperties => ({
  fontSize: 11,
  padding: "3px 8px",
  borderRadius: 12,
  border: "none",
  cursor: "pointer",
  background: active ? "var(--tint)" : "var(--bg-hover)",
  color: active ? "#fff" : "var(--fill-tertiary)",
  transition: "background 0.1s, color 0.1s",
});

function SearchResultItem({
  result,
  onClick,
}: {
  result: SearchResult;
  onClick: () => void;
}) {
  const snippetParts = useMemo(() => parseSnippet(result.snippet), [result.snippet]);
  const isUser = result.role === "user";

  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: "block",
        width: "100%",
        textAlign: "left",
        padding: "8px 10px",
        borderRadius: 6,
        border: "none",
        background: "transparent",
        cursor: "pointer",
        transition: "background 0.1s",
      }}
      onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
      onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 3 }}>
        <span style={{ fontSize: 13, fontWeight: 600, color: "var(--fill-primary)", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {result.session_title || "Untitled"}
        </span>
        <span style={{ fontSize: 11, color: "var(--fill-quaternary)", flexShrink: 0 }}>
          {formatRelativeTime(result.timestamp)}
        </span>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4 }}>
        {isUser ? (
          <User size={11} style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />
        ) : (
          <Robot size={11} style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />
        )}
        <span style={{ fontSize: 11, color: "var(--fill-quaternary)" }}>
          {isUser ? "User" : "Assistant"}
        </span>
        {result.work_dir && (
          <span style={{ fontSize: 11, color: "var(--fill-quaternary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            · {workDirLabel(result.work_dir)}
          </span>
        )}
      </div>
      <div style={{ fontSize: 12, lineHeight: 1.45, color: "var(--fill-secondary)", overflow: "hidden", display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical" }}>
        {snippetParts.map((part, i) =>
          part.highlight ? (
            <span key={i} style={{ fontWeight: 600, color: "var(--tint)" }}>{part.text}</span>
          ) : (
            <span key={i}>{part.text}</span>
          ),
        )}
      </div>
    </button>
  );
}

function SkeletonItem() {
  return (
    <div style={{ padding: "8px 10px" }}>
      <div style={{ height: 12, width: "60%", borderRadius: 4, background: "var(--bg-hover)", marginBottom: 8, animation: "pulse 1.5s ease-in-out infinite" }} />
      <div style={{ height: 10, width: "40%", borderRadius: 4, background: "var(--bg-hover)", marginBottom: 8, animation: "pulse 1.5s ease-in-out infinite" }} />
      <div style={{ height: 10, width: "90%", borderRadius: 4, background: "var(--bg-hover)", animation: "pulse 1.5s ease-in-out infinite" }} />
    </div>
  );
}

export function SearchPanel() {
  const { t } = useTranslation("sidebar");
  const inputRef = useRef<HTMLInputElement>(null);

  const query = useSearchStore((s) => s.query);
  const results = useSearchStore((s) => s.results);
  const loading = useSearchStore((s) => s.loading);
  const filters = useSearchStore((s) => s.filters);
  const hasMore = useSearchStore((s) => s.hasMore);
  const indexStatus = useSearchStore((s) => s.indexStatus);
  const setQuery = useSearchStore((s) => s.setQuery);
  const setFilters = useSearchStore((s) => s.setFilters);
  const closePanel = useSearchStore((s) => s.closePanel);
  const loadMore = useSearchStore((s) => s.loadMore);
  const navigateToResult = useSearchStore((s) => s.navigateToResult);

  const chats = useChatMetaStore((s) => s.chats);

  const workDirs = useMemo(() => {
    const dirs = new Set<string>();
    for (const r of results) {
      if (r.work_dir) dirs.add(r.work_dir);
    }
    for (const chat of Object.values(chats)) {
      if (chat.workDir) dirs.add(chat.workDir);
    }
    return Array.from(dirs).sort();
  }, [results, chats]);

  const datePreset = activeDatePreset(filters);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        closePanel();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [closePanel]);

  const handleDatePreset = useCallback((preset: DatePreset) => {
    if (preset === datePreset) {
      setFilters({ date_from: undefined, date_to: undefined });
    } else {
      setFilters(datePresetRange(preset));
    }
  }, [datePreset, setFilters]);

  const indexProgress = indexStatus && indexStatus.total_count > 0
    ? Math.min(100, Math.round((indexStatus.indexed_count / indexStatus.total_count) * 100))
    : 0;

  const showEmpty = !query.trim();
  const showNoResults = query.trim() && !loading && results.length === 0;

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        flex: 1,
        minHeight: 0,
        background: "var(--bg-shell)",
      }}
    >
      {/* Search input header */}
      <div style={{ padding: "8px 8px 4px", flexShrink: 0 }}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 6,
            height: 32,
            borderRadius: 8,
            padding: "0 8px",
            background: "var(--bg-hover)",
          }}
        >
          <MagnifyingGlass style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("globalSearchPlaceholder")}
            style={{
              flex: 1,
              minWidth: 0,
              background: "transparent",
              border: "none",
              outline: "none",
              fontSize: 13,
              color: "var(--fill-primary)",
            }}
          />
          {loading && <SpinnerGap className="animate-spin" style={{ color: "var(--fill-quaternary)", flexShrink: 0 }} />}
          <button
            type="button"
            onClick={closePanel}
            title={t("closePanel")}
            style={{
              background: "none",
              border: "none",
              padding: 2,
              cursor: "pointer",
              color: "var(--fill-quaternary)",
              display: "flex",
              alignItems: "center",
              borderRadius: 4,
            }}
          >
            <X weight="bold" />
          </button>
        </div>
      </div>

      {/* Index progress */}
      {indexStatus?.is_indexing && (
        <div style={{ padding: "4px 10px 6px", flexShrink: 0 }}>
          <div style={{ fontSize: 11, color: "var(--fill-tertiary)", marginBottom: 4 }}>
            {t("indexingProgress", { indexed: indexStatus.indexed_count, total: indexStatus.total_count })}
          </div>
          <div style={{ height: 3, borderRadius: 2, background: "var(--bg-hover)", overflow: "hidden" }}>
            <div
              style={{
                height: "100%",
                width: `${indexProgress}%`,
                background: "var(--tint)",
                transition: "width 0.3s ease",
              }}
            />
          </div>
        </div>
      )}

      {/* Filters */}
      <div style={{ padding: "2px 8px 6px", display: "flex", flexWrap: "wrap", gap: 4, flexShrink: 0 }}>
        <button type="button" style={chipStyle(datePreset === "today")} onClick={() => handleDatePreset("today")}>
          {t("filterToday")}
        </button>
        <button type="button" style={chipStyle(datePreset === "week")} onClick={() => handleDatePreset("week")}>
          {t("filterThisWeek")}
        </button>
        <button type="button" style={chipStyle(datePreset === "month")} onClick={() => handleDatePreset("month")}>
          {t("filterThisMonth")}
        </button>
        {workDirs.length > 0 && (
          <div style={{ position: "relative", display: "flex", alignItems: "center" }}>
            <select
              value={filters.work_dir ?? ""}
              onChange={(e) => setFilters({ work_dir: e.target.value || undefined })}
              style={{
                ...chipStyle(!!filters.work_dir),
                appearance: "none",
                paddingRight: 20,
                maxWidth: 140,
                overflow: "hidden",
                textOverflow: "ellipsis",
              }}
            >
              <option value="">{t("filterAllProjects")}</option>
              {workDirs.map((dir) => (
                <option key={dir} value={dir}>{workDirLabel(dir)}</option>
              ))}
            </select>
            <CaretDown size={10} style={{ position: "absolute", right: 6, pointerEvents: "none", color: filters.work_dir ? "#fff" : "var(--fill-quaternary)" }} />
          </div>
        )}
        {(filters.work_dir || filters.date_from) && (
          <button
            type="button"
            style={chipStyle(false)}
            onClick={() => setFilters({ work_dir: undefined, date_from: undefined, date_to: undefined })}
          >
            {t("filterClear")}
          </button>
        )}
      </div>

      {/* Results area */}
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "0 2px 8px 8px" }}>
        {showEmpty && (
          <div style={{ padding: "24px 10px", textAlign: "center", fontSize: 12, color: "var(--fill-quaternary)" }}>
            {t("globalSearchHint")}
            <div style={{ marginTop: 8, fontSize: 11, color: "var(--fill-quaternary)", opacity: 0.7 }}>
              {t("globalSearchShortcut")}
            </div>
          </div>
        )}

        {loading && results.length === 0 && (
          <>
            <SkeletonItem />
            <SkeletonItem />
            <SkeletonItem />
          </>
        )}

        {showNoResults && (
          <div style={{ padding: "24px 10px", textAlign: "center", fontSize: 12, color: "var(--fill-quaternary)" }}>
            {t("globalSearchNoResults", { query })}
          </div>
        )}

        {results.map((result, i) => (
          <SearchResultItem
            key={`${result.session_id}-${result.turn_id}-${result.role}-${i}`}
            result={result}
            onClick={() => navigateToResult(result)}
          />
        ))}

        {hasMore && results.length > 0 && (
          <button
            type="button"
            onClick={() => void loadMore()}
            disabled={loading}
            style={{
              display: "block",
              width: "calc(100% - 8px)",
              margin: "4px 0",
              padding: "8px",
              borderRadius: 6,
              border: "none",
              background: "var(--bg-hover)",
              color: "var(--fill-tertiary)",
              fontSize: 12,
              cursor: loading ? "not-allowed" : "pointer",
              opacity: loading ? 0.6 : 1,
            }}
          >
            {loading ? t("loadingMore") : t("loadMore")}
          </button>
        )}
      </div>
    </div>
  );
}
