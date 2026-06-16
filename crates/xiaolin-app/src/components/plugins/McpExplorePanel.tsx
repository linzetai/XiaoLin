import { useState, useMemo, useCallback } from "react";
import type { Icon } from "@phosphor-icons/react";
import {
  MagnifyingGlass, Check, SpinnerGap, WarningCircle,
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, Globe, MapPin, Clock,
  Package, GitBranch,
} from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { usePluginStore } from "../../lib/stores/plugin-store";
import { ICON_SIZE } from "../../lib/ui-tokens";
import type { AddMcpServerParams } from "../../lib/transport";
import registryData from "../../data/mcp-registry.json";

export interface McpRegistryEntry {
  id: string;
  name: string;
  description: string;
  category: string;
  icon: string;
  brandColor?: string;
  author?: string;
  tags?: string[];
  transport: "stdio" | "sse" | "streamable_http";
  command?: string;
  args?: string[];
  url?: string;
  env?: Record<string, string>;
  installHint?: string;
}

export const registry: McpRegistryEntry[] = registryData as McpRegistryEntry[];

const ICON_MAP: Record<string, Icon> = {
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, Globe, MapPin, Clock,
  Package, GitBranch, MagnifyingGlass,
};

type Category = "all" | "development" | "productivity" | "data" | "communication";

const CATEGORIES: { value: Category; labelKey: string }[] = [
  { value: "all", labelKey: "explore.cat_all" },
  { value: "development", labelKey: "explore.cat_development" },
  { value: "productivity", labelKey: "explore.cat_productivity" },
  { value: "data", labelKey: "explore.cat_data" },
  { value: "communication", labelKey: "explore.cat_communication" },
];

const CATEGORY_COLORS: Record<string, { bg: string; fg: string }> = {
  development: { bg: "rgba(99,179,237,0.12)", fg: "var(--blue, #4299E1)" },
  productivity: { bg: "rgba(72,187,120,0.12)", fg: "var(--green, #48BB78)" },
  data: { bg: "rgba(237,137,54,0.12)", fg: "var(--orange, #ED8936)" },
  communication: { bg: "rgba(159,122,234,0.12)", fg: "var(--purple, #9F7AEA)" },
};

export function McpExplorePanel() {
  const { t } = useTranslation("plugins");
  const plugins = usePluginStore((s) => s.plugins);
  const addPlugin = usePluginStore((s) => s.addPlugin);

  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<Category>("all");
  const [installingIds, setInstallingIds] = useState<Set<string>>(new Set());
  const [errorId, setErrorId] = useState<string | null>(null);

  const installedIds = useMemo(
    () => new Set(plugins.map((p) => p.id)),
    [plugins],
  );

  const filtered = useMemo(() => {
    let items = registry;
    if (category !== "all") {
      items = items.filter((e) => e.category === category);
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      items = items.filter(
        (e) =>
          e.name.toLowerCase().includes(q) ||
          e.description.toLowerCase().includes(q) ||
          (e.tags ?? []).some((tag) => tag.toLowerCase().includes(q)),
      );
    }
    return items;
  }, [search, category]);

  const handleInstall = useCallback(
    async (entry: McpRegistryEntry) => {
      if (installingIds.has(entry.id)) return;
      setInstallingIds((prev) => new Set(prev).add(entry.id));
      setErrorId(null);

      const params: AddMcpServerParams = {
        id: entry.id,
        transport: entry.transport,
        ...(entry.command ? { command: entry.command } : {}),
        ...(entry.args ? { args: entry.args } : {}),
        ...(entry.url ? { url: entry.url } : {}),
        ...(entry.env ? { env: entry.env } : {}),
      };

      const ok = await addPlugin(params);
      setInstallingIds((prev) => { const next = new Set(prev); next.delete(entry.id); return next; });
      if (!ok) setErrorId(entry.id);
    },
    [addPlugin, installingIds],
  );

  return (
    <div className="mx-auto w-full px-6 py-5" style={{ maxWidth: "var(--content-max-w)" }}>
      {/* Search */}
      <div className="relative mb-4">
        <MagnifyingGlass
          size={ICON_SIZE.sm}
          className="absolute left-3 top-1/2 -translate-y-1/2"
          style={{ color: "var(--fill-quaternary)" }}
        />
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("explore.search_placeholder")}
          className="w-full rounded-xl py-2.5 pl-9 pr-3 text-[13px] outline-none transition-all"
          style={{
            background: "var(--bg-tertiary)",
            border: "1px solid var(--separator)",
            color: "var(--fill-primary)",
          }}
        />
      </div>

      {/* Category pills */}
      <div className="mb-5 flex flex-wrap gap-1.5">
        {CATEGORIES.map((cat) => {
          const active = category === cat.value;
          return (
            <button
              key={cat.value}
              onClick={() => setCategory(cat.value)}
              className="rounded-full px-3 py-1 text-[11px] font-medium transition-colors"
              style={{
                background: active ? "var(--tint)" : "var(--bg-tertiary)",
                color: active ? "#fff" : "var(--fill-secondary)",
                border: "none",
                cursor: "pointer",
              }}
            >
              {t(cat.labelKey)}
            </button>
          );
        })}
      </div>

      {/* Card grid */}
      {filtered.length === 0 ? (
        <div className="py-12 text-center">
          <p className="text-[13px]" style={{ color: "var(--fill-quaternary)" }}>
            {t("explore.no_results")}
          </p>
        </div>
      ) : (
        <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))" }}>
          {filtered.map((entry, i) => (
            <ExploreCard
              key={entry.id}
              entry={entry}
              installed={installedIds.has(entry.id)}
              installing={installingIds.has(entry.id)}
              hasError={errorId === entry.id}
              index={i}
              onInstall={handleInstall}
              t={t}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ExploreCard({
  entry, installed, installing, hasError, index, onInstall, t,
}: {
  entry: McpRegistryEntry;
  installed: boolean;
  installing: boolean;
  hasError: boolean;
  index: number;
  onInstall: (e: McpRegistryEntry) => void;
  t: (key: string) => string;
}) {
  const IconComp = ICON_MAP[entry.icon] ?? Cube;
  const catColor = CATEGORY_COLORS[entry.category] ?? CATEGORY_COLORS.development;
  const brand = entry.brandColor ?? catColor.fg;

  return (
    <div
      className="pv-stagger flex flex-col rounded-[var(--radius-sm)] p-4 transition-all duration-200 hover:-translate-y-0.5"
      style={{
        "--stagger-i": index,
        background: "var(--bg-primary)",
        border: "0.5px solid var(--separator)",
        boxShadow: "var(--shadow-sm)",
      } as React.CSSProperties}
      onMouseEnter={(e) => { (e.currentTarget.style.boxShadow) = "var(--shadow-md)"; }}
      onMouseLeave={(e) => { (e.currentTarget.style.boxShadow) = "var(--shadow-sm)"; }}
    >
      {/* Top row: icon + name + author */}
      <div className="flex items-start gap-3 mb-2">
        <div
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-[10px]"
          style={{ background: `color-mix(in srgb, ${brand} 12%, transparent)` }}
        >
          <IconComp size={ICON_SIZE.lg} style={{ color: brand }} />
        </div>
        <div className="min-w-0 flex-1">
          <span className="block truncate text-[13px] font-semibold" style={{ color: "var(--fill-primary)" }}>
            {entry.name}
          </span>
          {entry.author && (
            <span className="block text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
              {entry.author}
            </span>
          )}
        </div>
      </div>

      {/* Category badge */}
      <span
        className="mb-1.5 inline-flex w-fit rounded-full px-2 py-0.5 text-[10px] font-medium"
        style={{ background: catColor.bg, color: catColor.fg }}
      >
        {t(`explore.cat_${entry.category}`)}
      </span>

      {/* Description */}
      <p
        className="mb-2 text-[12px] leading-relaxed"
        style={{ color: "var(--fill-tertiary)", display: "-webkit-box", WebkitLineClamp: 2, WebkitBoxOrient: "vertical", overflow: "hidden" }}
      >
        {entry.description}
      </p>

      {/* Tags */}
      {entry.tags && entry.tags.length > 0 && (
        <div className="mb-3 flex flex-wrap gap-1">
          {entry.tags.map((tag) => (
            <span
              key={tag}
              className="rounded px-1.5 py-0.5 text-[10px]"
              style={{ background: "var(--bg-tertiary)", color: "var(--fill-quaternary)" }}
            >
              {tag}
            </span>
          ))}
        </div>
      )}

      {/* Spacer to push install button to bottom */}
      <div className="flex-1" />

      {/* Install action */}
      <div className="mt-auto">
        {hasError && (
          <div className="mb-1.5 flex items-center gap-1 text-[11px]" style={{ color: "var(--red)" }}>
            <WarningCircle size={ICON_SIZE.xs} />
            {t("explore.install_failed")}
          </div>
        )}
        {installed ? (
          <span
            className="flex w-full items-center justify-center gap-1 rounded-md py-1.5 text-[11px] font-medium"
            style={{ color: "var(--green)", background: "rgba(72,187,120,0.08)" }}
          >
            <Check size={ICON_SIZE.xs} weight="bold" />
            {t("explore.installed")}
          </span>
        ) : (
          <button
            onClick={() => onInstall(entry)}
            disabled={installing}
            className="w-full rounded-md py-1.5 text-[11px] font-semibold transition-colors hover:opacity-90"
            style={{
              background: "var(--tint)",
              color: "#fff",
              border: "none",
              cursor: installing ? "wait" : "pointer",
              opacity: installing ? 0.7 : 1,
            }}
          >
            {installing ? (
              <SpinnerGap size={ICON_SIZE.xs} className="animate-spin" />
            ) : (
              t("explore.install")
            )}
          </button>
        )}
      </div>
    </div>
  );
}
