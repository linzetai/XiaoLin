import { create } from "zustand";
import { persist } from "zustand/middleware";

export type ThemeMode = "light" | "dark" | "system";
type ResolvedTheme = "light" | "dark";

export type AccentTheme =
  | "default"
  | "monochrome"
  | "ocean"
  | "sunset"
  | "midnight"
  | "sage"
  | "rose";

export interface AccentPreset {
  id: AccentTheme;
  label: string;
  preview: {
    light: { bg: string; sidebar: string; accent: string; text: string };
    dark:  { bg: string; sidebar: string; accent: string; text: string };
  };
}

export const ACCENT_PRESETS: AccentPreset[] = [
  {
    id: "default", label: "经典",
    preview: {
      light: { bg: "#ffffff", sidebar: "#f5f5f7", accent: "#2563EB", text: "#6e6e73" },
      dark:  { bg: "#000000", sidebar: "#1c1c1e", accent: "#60A5FA", text: "#98989d" },
    },
  },
  {
    id: "monochrome", label: "素雅",
    preview: {
      light: { bg: "#ffffff", sidebar: "#f5f5f7", accent: "#1d1d1f", text: "#6e6e73" },
      dark:  { bg: "#000000", sidebar: "#1c1c1e", accent: "#e5e5ea", text: "#98989d" },
    },
  },
  {
    id: "ocean", label: "海洋",
    preview: {
      light: { bg: "#F0F7FF", sidebar: "#E1EFFE", accent: "#2563EB", text: "#3B6B9E" },
      dark:  { bg: "#0A1628", sidebar: "#0F1F35", accent: "#3B82F6", text: "#7BA3CC" },
    },
  },
  {
    id: "sunset", label: "日落",
    preview: {
      light: { bg: "#FFFBF5", sidebar: "#FFF3E5", accent: "#EA580C", text: "#9A3412" },
      dark:  { bg: "#1A0F05", sidebar: "#261A0E", accent: "#FB923C", text: "#FBBF24" },
    },
  },
  {
    id: "midnight", label: "午夜",
    preview: {
      light: { bg: "#F8FAFC", sidebar: "#F1F5F9", accent: "#3B82F6", text: "#334155" },
      dark:  { bg: "#020617", sidebar: "#0F172A", accent: "#60A5FA", text: "#94A3B8" },
    },
  },
  {
    id: "sage", label: "鼠尾草",
    preview: {
      light: { bg: "#F7FAF8", sidebar: "#EDF5F0", accent: "#16A34A", text: "#2D6B44" },
      dark:  { bg: "#0A1A10", sidebar: "#0F261A", accent: "#4ADE80", text: "#6ECC8E" },
    },
  },
  {
    id: "rose", label: "玫瑰",
    preview: {
      light: { bg: "#FFF7F9", sidebar: "#FEEEF2", accent: "#E11D48", text: "#881337" },
      dark:  { bg: "#1A0A10", sidebar: "#2D1220", accent: "#FB7185", text: "#FB7185" },
    },
  },
];

interface ThemeState {
  mode: ThemeMode;
  resolved: ResolvedTheme;
  accent: AccentTheme;
  setMode: (mode: ThemeMode) => void;
  setAccent: (accent: AccentTheme) => void;
}

function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode !== "system") return mode;
  if (typeof window === "undefined") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

const isMac = typeof navigator !== "undefined"
  && /Mac|iPhone|iPad/.test(
    (navigator as { userAgentData?: { platform?: string } }).userAgentData?.platform
      ?? navigator.platform ?? "",
  );

function applyTheme(theme: ResolvedTheme, accent: AccentTheme) {
  const el = document.documentElement;
  el.setAttribute("data-theme", theme);
  if (!isMac) el.setAttribute("data-opaque-chrome", "");
  if (accent === "default") {
    el.removeAttribute("data-accent");
  } else {
    el.setAttribute("data-accent", accent);
  }
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set, get) => ({
      mode: "light" as ThemeMode,
      resolved: "light" as ResolvedTheme,
      accent: "default" as AccentTheme,
      setMode: (mode) => {
        const resolved = resolveTheme(mode);
        applyTheme(resolved, get().accent);
        set({ mode, resolved });
      },
      setAccent: (accent) => {
        applyTheme(get().resolved, accent);
        set({ accent });
      },
    }),
    {
      name: "xiaolin-theme",
      onRehydrateStorage: () => (state) => {
        if (state) {
          const resolved = resolveTheme(state.mode);
          applyTheme(resolved, state.accent ?? "default");
          state.resolved = resolved;
        }
      },
    },
  ),
);

if (typeof window !== "undefined") {
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", () => {
      const { mode, setMode } = useThemeStore.getState();
      if (mode === "system") setMode("system");
    });

  const { mode, accent } = useThemeStore.getState();
  applyTheme(resolveTheme(mode), accent);
}
