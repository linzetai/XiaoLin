import { useTranslation } from "react-i18next";
import { memo, useCallback } from "react";
import { DownloadSimple, X, ArrowCounterClockwise } from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";
import { useAppUpdater } from "../../lib/use-app-updater";

export const UpdateBanner = memo(function UpdateBanner() {
  const { t } = useTranslation("common");
  const { status, info, progress, downloadAndInstall, restartApp, dismiss } =
    useAppUpdater(true);

  const handleAction = useCallback(() => {
    if (status === "available") downloadAndInstall();
    else if (status === "ready") restartApp();
  }, [status, downloadAndInstall, restartApp]);

  if (status !== "available" && status !== "downloading" && status !== "ready") {
    return null;
  }

  const label =
    status === "available"
      ? t("updateAvailable", { version: info?.version ?? "" })
      : status === "downloading"
        ? t("updateDownloading", { progress })
        : t("updateReadyRestart");

  const ActionIcon = status === "ready" ? ArrowCounterClockwise : DownloadSimple;
  const actionLabel = status === "ready" ? t("restartNow") : t("downloadUpdate");
  const actionDisabled = status === "downloading";

  return (
    <div
      className="flex items-center justify-between px-4 py-1.5"
      style={{
        background: "var(--tint)",
        color: "#fff",
        fontSize: 12,
        fontWeight: 500,
        zIndex: 100,
      }}
    >
      <span className="truncate">{label}</span>
      <div className="flex shrink-0 items-center gap-2">
        {status === "downloading" && (
          <div
            className="h-1 overflow-hidden rounded-full"
            style={{ width: 80, background: "rgba(255,255,255,0.3)" }}
          >
            <div
              className="h-full rounded-full transition-[width] duration-300"
              style={{ width: `${progress}%`, background: "#fff" }}
            />
          </div>
        )}
        {status !== "downloading" && (
          <button
            onClick={handleAction}
            disabled={actionDisabled}
            className="flex cursor-pointer items-center gap-1 rounded px-2 py-0.5 text-[11px] font-medium transition-opacity duration-150 hover:opacity-80 disabled:cursor-default disabled:opacity-50"
            style={{ background: "rgba(255,255,255,0.2)" }}
          >
            <ActionIcon />
            {actionLabel}
          </button>
        )}
        {status === "available" && (
          <button
            onClick={dismiss}
            className="flex cursor-pointer items-center rounded p-0.5 transition-opacity duration-150 hover:opacity-70"
            style={{ background: "transparent" }}
            aria-label={t("close")}
          >
            <X size={ICON_SIZE.md} />
          </button>
        )}
      </div>
    </div>
  );
});
