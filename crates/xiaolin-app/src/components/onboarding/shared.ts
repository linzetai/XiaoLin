import type React from "react";

export const inputCls =
  "w-full rounded-[8px] px-3 py-2.5 text-[13px] outline-none transition-all focus:ring-2 focus:ring-[var(--tint)]";

export const inputStyle: React.CSSProperties = {
  background: "var(--bg-base)",
  color: "var(--fill-primary)",
  border: "0.5px solid var(--separator-opaque)",
};

export const labelCls =
  "mb-1.5 block text-[11px] font-semibold tracking-wide uppercase";

export const labelStyle: React.CSSProperties = { color: "var(--fill-tertiary)" };
