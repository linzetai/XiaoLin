/**
 * Shared UI design tokens — single source of truth for icon sizes,
 * button containers, border-radius classes, and hover/transition patterns.
 *
 * Every component MUST import from here instead of using magic numbers.
 */

export const ICON = {
  sm: { size: 14, strokeWidth: 1.5 } as const,
  md: { size: 16, strokeWidth: 1.5 } as const,
  lg: { size: 20, strokeWidth: 1.5 } as const,
} as const;

export const ICON_ACTIVE_STROKE = 2;

export const BTN_ICON = {
  sm: "flex h-7 w-7 items-center justify-center rounded-[var(--radius-xs)] transition-colors duration-150 hover:bg-[var(--bg-hover)]",
  lg: "flex h-9 w-9 items-center justify-center rounded-[var(--radius-xs)] transition-colors duration-150 hover:bg-[var(--bg-hover)]",
} as const;
