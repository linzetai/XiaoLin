import { useTranslation } from "react-i18next";
import { Layout, FolderOpen, Sparkle } from "@phosphor-icons/react";

const CONFIG: Record<string, { icon: typeof Sparkle; descKey: string; labelKey: string }> = {
  studio: { icon: Layout, descKey: "workspaceStudioDesc", labelKey: "workspaceStudio" },
  files: { icon: FolderOpen, descKey: "workspaceFilesDesc", labelKey: "workspaceFiles" },
};

export function ComingSoon({ title }: { title?: string }) {
  const { t } = useTranslation("common");
  const meta = title ? CONFIG[title] : undefined;
  const Icon = meta?.icon ?? Sparkle;

  return (
    <div
      className="relative flex h-full flex-col items-center justify-center gap-4"
      style={{ background: "var(--bg-primary)", animation: "scale-in var(--duration-slow) var(--ease-out)" }}
    >
      <div
        className="relative flex h-14 w-14 items-center justify-center rounded-[var(--radius-lg)]"
        style={{
          background: "var(--tint-bg)",
          color: "var(--tint)",
          boxShadow: "0 0 0 4px var(--tint-subtle)",
          animation: "icon-float 3s ease-in-out infinite",
        }}
      >
        <Icon size={24} />
      </div>
      {title && (
        <h3
          className="relative text-[15px] font-semibold tracking-[-0.01em]"
          style={{ color: "var(--fill-primary)" }}
        >
          {meta ? t(meta.labelKey) : title}
        </h3>
      )}
      <p className="relative max-w-[280px] text-center text-[13px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
        {meta ? t(meta.descKey) : t("comingSoonFallback")}
      </p>
      <span
        className="relative mt-2 inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[11px] font-medium"
        style={{
          background: "var(--tint-subtle)",
          color: "var(--tint)",
          border: "0.5px solid var(--border-subtle)",
        }}
      >
        <Sparkle />
        {t("comingSoon")}
      </span>
    </div>
  );
}
