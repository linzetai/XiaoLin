import { useState, useRef, useEffect, useMemo, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { FolderOpen, X, Check } from "@phosphor-icons/react";
import { createPortal } from "react-dom";
import { useProjectStore } from "../../lib/stores";
import { fuzzyMatch } from "../../lib/fuzzy";
import type { ProjectSummary } from "../../lib/transport";

interface ProjectDropdownProps {
  currentProjectId: string | null;
  currentWorkDir: string | null;
  onSelectProject: (project: ProjectSummary) => void;
  onBrowseFolder: () => void;
  onClearProject: () => void;
}

export function ProjectDropdown({
  currentProjectId,
  currentWorkDir,
  onSelectProject,
  onBrowseFolder,
  onClearProject,
}: ProjectDropdownProps) {
  const { t } = useTranslation("sidebar");
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);

  const projects = useProjectStore((s) => s.projects);
  const projectList = useMemo(
    () => Object.values(projects).filter((p) => !p.archived).sort(
      (a, b) => new Date(b.lastOpenedAt).getTime() - new Date(a.lastOpenedAt).getTime()
    ),
    [projects],
  );

  const filtered = useMemo(() => {
    if (!query.trim()) return projectList;
    return projectList
      .map((p) => {
        const nameMatch = fuzzyMatch(query, p.name);
        const pathMatch = fuzzyMatch(query, p.rootPath);
        const best = Math.max(nameMatch?.score ?? -1, pathMatch?.score ?? -1);
        return best >= 0 ? { project: p, score: best } : null;
      })
      .filter((r): r is { project: ProjectSummary; score: number } => r !== null)
      .sort((a, b) => b.score - a.score)
      .map((r) => r.project);
  }, [projectList, query]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setTimeout(() => searchRef.current?.focus(), 50);
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (panelRef.current && !panelRef.current.contains(e.target as Node) &&
          triggerRef.current && !triggerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleKey);
    };
  }, [open]);

  const handleSelect = useCallback((project: ProjectSummary) => {
    onSelectProject(project);
    setOpen(false);
  }, [onSelectProject]);

  const handleBrowse = useCallback(() => {
    setOpen(false);
    onBrowseFolder();
  }, [onBrowseFolder]);

  const handleClear = useCallback(() => {
    onClearProject();
    setOpen(false);
  }, [onClearProject]);

  const currentProject = currentProjectId ? projects[currentProjectId] : null;
  const displayLabel = currentProject
    ? currentProject.name
    : currentWorkDir
      ? currentWorkDir.replace(/^\/home\/[^/]+\//, "~/").replace(/^(.{24}).+/, "$1…")
      : "Work locally";

  const [panelPos, setPanelPos] = useState<{ left: number; bottom: number } | null>(null);

  useEffect(() => {
    if (open && triggerRef.current) {
      const rect = triggerRef.current.getBoundingClientRect();
      setPanelPos({ left: rect.left, bottom: window.innerHeight - rect.top + 4 });
    }
  }, [open]);

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        style={{
          display: "flex", alignItems: "center", gap: 4,
          padding: "3px 8px", borderRadius: 4, border: "none",
          background: open ? "var(--bg-hover)" : "transparent",
          color: "var(--fill-quaternary)", fontSize: 11,
          cursor: "pointer", transition: "background 0.1s, color 0.1s",
        }}
        onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; e.currentTarget.style.color = "var(--fill-tertiary)"; }}
        onMouseLeave={(e) => { if (!open) { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "var(--fill-quaternary)"; } }}
        onClick={() => setOpen(!open)}
        title={currentWorkDir ? t("workDirLabel", { dir: currentWorkDir }) : t("setWorkDir")}
      >
        {currentProject ? (
          <span style={{ width: 7, height: 7, borderRadius: "50%", background: currentProject.color || "#2563EB", flexShrink: 0 }} />
        ) : (
          <FolderOpen size={12} />
        )}
        <span>{displayLabel}</span>
        <span style={{ fontSize: 8, opacity: 0.4 }}>▾</span>
      </button>

      {open && panelPos && createPortal(
        <div
          ref={panelRef}
          style={{
            position: "fixed",
            left: panelPos.left,
            bottom: panelPos.bottom,
            width: 300,
            maxHeight: 340,
            background: "var(--bg-elevated)",
            border: "0.5px solid var(--separator)",
            borderRadius: "var(--radius-sm)",
            boxShadow: "var(--shadow-lg)",
            display: "flex",
            flexDirection: "column",
            animation: "scale-in var(--duration-fast) var(--ease-out)",
            transformOrigin: "bottom left",
            zIndex: 100,
          }}
        >
          {/* Search */}
          <div style={{ padding: "8px 8px 4px", flexShrink: 0 }}>
            <input
              ref={searchRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t("searchProjectOrPath")}
              style={{
                width: "100%", padding: "6px 10px",
                borderRadius: 6, border: "1px solid var(--separator)",
                background: "var(--bg-primary)", fontSize: 12,
                color: "var(--fill-primary)", outline: "none",
              }}
              onFocus={(e) => { e.currentTarget.style.borderColor = "var(--tint)"; }}
              onBlur={(e) => { e.currentTarget.style.borderColor = "var(--separator)"; }}
            />
          </div>

          {/* Project list */}
          <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "4px 4px" }}>
            {filtered.length === 0 && (
              <div style={{ padding: "12px 8px", fontSize: 12, color: "var(--fill-quaternary)", textAlign: "center" }}>
                {query ? t("noMatchingProjects") : t("noProjects")}
              </div>
            )}
            {filtered.map((project) => {
              const isActive = currentProjectId === project.id;
              return (
                <button
                  key={project.id}
                  type="button"
                  onClick={() => handleSelect(project)}
                  style={{
                    display: "flex", alignItems: "center", gap: 8,
                    width: "100%", padding: "7px 8px", borderRadius: 6,
                    border: "none", background: isActive ? "var(--bg-active)" : "transparent",
                    cursor: "pointer", textAlign: "left",
                    transition: "background 0.1s",
                  }}
                  onMouseEnter={(e) => { if (!isActive) e.currentTarget.style.background = "var(--bg-hover)"; }}
                  onMouseLeave={(e) => { if (!isActive) e.currentTarget.style.background = "transparent"; }}
                >
                  <span style={{ width: 8, height: 8, borderRadius: "50%", background: project.color || "#2563EB", flexShrink: 0 }} />
                  <div style={{ flex: 1, minWidth: 0, overflow: "hidden" }}>
                    <div style={{ fontSize: 12, fontWeight: 500, color: "var(--fill-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {project.name}
                    </div>
                    <div style={{ fontSize: 10, color: "var(--fill-quaternary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {project.rootPath.replace(/^\/home\/[^/]+\//, "~/")}
                    </div>
                  </div>
                  {isActive && <Check size={13} weight="bold" style={{ color: "var(--tint)", flexShrink: 0 }} />}
                </button>
              );
            })}
          </div>

          {/* Actions */}
          <div style={{ borderTop: "1px solid var(--separator)", padding: "4px 4px", flexShrink: 0 }}>
            <button
              type="button"
              onClick={handleBrowse}
              style={{
                display: "flex", alignItems: "center", gap: 8,
                width: "100%", padding: "7px 8px", borderRadius: 6,
                border: "none", background: "transparent",
                cursor: "pointer", fontSize: 12, color: "var(--fill-secondary)",
                transition: "background 0.1s",
              }}
              onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
            >
              <FolderOpen size={13} />
              <span>{t("browseFolder")}</span>
            </button>
            {(currentProjectId || currentWorkDir) && (
              <button
                type="button"
                onClick={handleClear}
                style={{
                  display: "flex", alignItems: "center", gap: 8,
                  width: "100%", padding: "7px 8px", borderRadius: 6,
                  border: "none", background: "transparent",
                  cursor: "pointer", fontSize: 12, color: "var(--fill-quaternary)",
                  transition: "background 0.1s",
                }}
                onMouseEnter={(e) => { e.currentTarget.style.background = "var(--bg-hover)"; }}
                onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; }}
              >
                <X size={13} />
                <span>{t("noProject")}</span>
              </button>
            )}
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}
