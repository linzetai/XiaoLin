import { useState, useMemo, useCallback } from "react";
import type { Icon } from "@phosphor-icons/react";
import {
  MagnifyingGlass, Check, SpinnerGap, WarningCircle,
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, Globe, MapPin, Clock,
  Package, GitBranch, X, Key,
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

export const MCP_ICON_MAP: Record<string, Icon> = {
  FolderOpen, GithubLogo, Database, Browser, ChatCircle,
  Brain, TreeStructure, Cube, Globe, MapPin, Clock,
  Package, GitBranch, MagnifyingGlass,
};

const ICON_MAP = MCP_ICON_MAP;

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

function needsEnvInput(entry: McpRegistryEntry): boolean {
  if (!entry.env) return false;
  return Object.values(entry.env).some((v) => v === "");
}

export function McpExplorePanel() {
  const { t } = useTranslation("plugins");
  const plugins = usePluginStore((s) => s.plugins);
  const addPlugin = usePluginStore((s) => s.addPlugin);

  const [search, setSearch] = useState("");
  const [category, setCategory] = useState<Category>("all");
  const [installingIds, setInstallingIds] = useState<Set<string>>(new Set());
  const [errorId, setErrorId] = useState<string | null>(null);
  const [envDialogEntry, setEnvDialogEntry] = useState<McpRegistryEntry | null>(null);

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

  const doInstall = useCallback(
    async (entry: McpRegistryEntry, envOverrides?: Record<string, string>) => {
      setInstallingIds((prev) => new Set(prev).add(entry.id));
      setErrorId(null);

      const finalEnv = envOverrides ?? entry.env;
      const params: AddMcpServerParams = {
        id: entry.id,
        transport: entry.transport,
        ...(entry.command ? { command: entry.command } : {}),
        ...(entry.args ? { args: entry.args } : {}),
        ...(entry.url ? { url: entry.url } : {}),
        ...(finalEnv && Object.keys(finalEnv).length > 0 ? { env: finalEnv } : {}),
      };

      const ok = await addPlugin(params);
      setInstallingIds((prev) => { const next = new Set(prev); next.delete(entry.id); return next; });
      if (!ok) setErrorId(entry.id);
    },
    [addPlugin],
  );

  const handleInstall = useCallback(
    (entry: McpRegistryEntry) => {
      if (installingIds.has(entry.id)) return;
      if (needsEnvInput(entry)) {
        setEnvDialogEntry(entry);
      } else {
        doInstall(entry);
      }
    },
    [installingIds, doInstall],
  );

  const handleEnvSubmit = useCallback(
    (env: Record<string, string>) => {
      if (!envDialogEntry) return;
      setEnvDialogEntry(null);
      doInstall(envDialogEntry, env);
    },
    [envDialogEntry, doInstall],
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
          className="explore-search w-full rounded-xl py-2.5 pl-9 pr-3 text-[13px] outline-none transition-all"
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
                background: active ? "color-mix(in srgb, var(--tint) 14%, transparent)" : "var(--bg-tertiary)",
                color: active ? "var(--tint)" : "var(--fill-secondary)",
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
        <div className="explore-card-grid grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(240px, 1fr))" }}>
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

      {envDialogEntry && (
        <EnvConfigDialog
          entry={envDialogEntry}
          onSubmit={handleEnvSubmit}
          onCancel={() => setEnvDialogEntry(null)}
        />
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
            className="flex w-full items-center justify-center rounded-md py-1.5 text-[11px] font-semibold transition-colors hover:opacity-90"
            style={{
              background: "color-mix(in srgb, var(--tint) 14%, transparent)",
              color: "var(--tint)",
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

function EnvConfigDialog({
  entry,
  onSubmit,
  onCancel,
}: {
  entry: McpRegistryEntry;
  onSubmit: (env: Record<string, string>) => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("plugins");
  const emptyKeys = Object.entries(entry.env ?? {})
    .filter(([, v]) => v === "")
    .map(([k]) => k);
  const [values, setValues] = useState<Record<string, string>>(
    () => Object.fromEntries(emptyKeys.map((k) => [k, ""])),
  );

  const allFilled = emptyKeys.every((k) => values[k]?.trim());
  const IconComp = ICON_MAP[entry.icon] ?? Cube;
  const brand = entry.brandColor ?? "var(--tint)";

  const handleSubmit = () => {
    const merged = { ...entry.env };
    for (const [k, v] of Object.entries(values)) {
      merged[k] = v.trim();
    }
    onSubmit(merged);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center"
      style={{ background: "rgba(0,0,0,0.5)" }}
      onClick={onCancel}
    >
      <div
        className="w-[420px] rounded-[var(--radius-lg)] p-6"
        style={{ background: "var(--bg-card)", border: "0.5px solid var(--separator)" }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-5 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div
              className="flex h-9 w-9 items-center justify-center rounded-[9px]"
              style={{ background: `color-mix(in srgb, ${brand} 12%, transparent)` }}
            >
              <IconComp size={ICON_SIZE.md} style={{ color: brand }} />
            </div>
            <div>
              <h3 className="text-[14px] font-semibold" style={{ color: "var(--fill-primary)" }}>
                {entry.name}
              </h3>
              <p className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>
                {t("explore.env_config_subtitle")}
              </p>
            </div>
          </div>
          <button
            onClick={onCancel}
            style={{ cursor: "pointer", background: "none", border: "none", color: "var(--fill-tertiary)" }}
          >
            <X size={ICON_SIZE.md} />
          </button>
        </div>

        {entry.installHint && (
          <div
            className="mb-4 flex items-start gap-2 rounded-md px-3 py-2.5 text-[11px] leading-relaxed"
            style={{
              background: "color-mix(in srgb, var(--yellow, #D69E2E) 6%, transparent)",
              border: "0.5px solid color-mix(in srgb, var(--yellow, #D69E2E) 20%, transparent)",
              color: "var(--fill-secondary)",
            }}
          >
            <Key size={ICON_SIZE.sm} className="mt-0.5 shrink-0" style={{ color: "var(--yellow, #D69E2E)" }} />
            <span>{entry.installHint}</span>
          </div>
        )}

        <div className="flex flex-col gap-3">
          {emptyKeys.map((key) => (
            <div key={key}>
              <label
                className="mb-1 block text-[11px] font-medium font-mono"
                style={{ color: "var(--fill-quaternary)" }}
              >
                {key}
              </label>
              <input
                type={key.toLowerCase().includes("token") || key.toLowerCase().includes("secret") || key.toLowerCase().includes("key") ? "password" : "text"}
                value={values[key] ?? ""}
                onChange={(e) => setValues((prev) => ({ ...prev, [key]: e.target.value }))}
                placeholder={t("explore.env_placeholder")}
                className="w-full rounded-md px-3 py-2 text-[12px] font-mono outline-none transition-all"
                style={{
                  background: "var(--bg-primary)",
                  border: "1px solid var(--separator)",
                  color: "var(--fill-primary)",
                }}
                onFocus={(e) => { e.currentTarget.style.borderColor = "var(--tint)"; }}
                onBlur={(e) => { e.currentTarget.style.borderColor = "var(--separator)"; }}
                autoFocus={emptyKeys.indexOf(key) === 0}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && allFilled) handleSubmit();
                }}
              />
            </div>
          ))}
        </div>

        <div className="mt-5 flex items-center justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-md px-3 py-1.5 text-[12px] font-medium transition-colors hover:bg-[var(--bg-hover)]"
            style={{ cursor: "pointer", background: "none", border: "0.5px solid var(--separator)", color: "var(--fill-secondary)" }}
          >
            {t("cancel")}
          </button>
          <button
            onClick={handleSubmit}
            disabled={!allFilled}
            className="rounded-md px-4 py-1.5 text-[12px] font-semibold transition-colors hover:opacity-90 disabled:opacity-40"
            style={{ cursor: allFilled ? "pointer" : "not-allowed", background: "var(--tint)", border: "none", color: "#fff" }}
          >
            {t("explore.install_with_config")}
          </button>
        </div>
      </div>
    </div>
  );
}
