import { CaretDown, CaretUp, Clock } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

interface QueueIndicatorProps {
  count: number;
  expanded: boolean;
  onToggle: () => void;
}

export function QueueIndicator({ count, expanded, onToggle }: QueueIndicatorProps) {
  const { t } = useTranslation("chat");
  if (count === 0) return null;

  return (
    <button
      onClick={onToggle}
      className="flex w-full items-center gap-1.5 rounded-lg px-3 py-1.5 text-[12px] transition-colors duration-100"
      style={{
        background: "var(--tint)",
        color: "#fff",
        opacity: 0.85,
        cursor: "pointer",
      }}
    >
      <Clock />
      <span className="flex-1 text-left font-medium">
        {t("queue_pendingCount", { count })}
      </span>
      {expanded ? <CaretUp /> : <CaretDown />}
    </button>
  );
}
