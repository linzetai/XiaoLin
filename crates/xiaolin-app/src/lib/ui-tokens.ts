/**
 * Shared UI design tokens — single source of truth for icon sizes,
 * button containers, border-radius classes, and hover/transition patterns.
 *
 * Every component MUST import from here instead of using magic numbers.
 */

import type { IconWeight } from "@phosphor-icons/react";

export const ICON_SIZE = {
  xs: 12,
  sm: 14,
  md: 16,
  lg: 20,
  xl: 24,
  "2xl": 32,
} as const;

export const ICON_WEIGHT = {
  thin: "thin" as IconWeight,
  light: "light" as IconWeight,
  regular: "regular" as IconWeight,
  bold: "bold" as IconWeight,
  fill: "fill" as IconWeight,
  duotone: "duotone" as IconWeight,
} as const;

export const ICON_COLOR = {
  default: undefined,
  muted: "var(--fill-quaternary)",
  secondary: "var(--fill-secondary)",
  accent: "var(--fill-accent)",
  danger: "var(--red)",
  success: "var(--green)",
} as const;

/** @deprecated Use ICON_SIZE + Phosphor weight instead */
export const ICON = {
  sm: { size: ICON_SIZE.sm } as const,
  md: { size: ICON_SIZE.md } as const,
  lg: { size: ICON_SIZE.lg } as const,
} as const;

/** @deprecated Use ICON_WEIGHT.bold instead */
export const ICON_ACTIVE_STROKE = 2;

export const BTN_ICON = {
  sm: "flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors duration-150 hover:bg-[var(--bg-hover)]",
  lg: "flex h-9 w-9 items-center justify-center rounded-[var(--radius-xs)] transition-colors duration-150 hover:bg-[var(--bg-hover)]",
} as const;

export const BTN_TEXT_SM =
  "flex items-center gap-1 rounded-[var(--radius-xs)] px-2.5 py-1.5 text-[11px] font-medium transition-colors duration-150 hover:bg-[var(--bg-hover)] text-[var(--fill-tertiary)] bg-transparent border-none cursor-pointer" as const;

export const BTN_PRIMARY_SM =
  "flex items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-semibold transition-colors duration-150 bg-[var(--tint)] text-white border-none cursor-pointer hover:opacity-90" as const;
