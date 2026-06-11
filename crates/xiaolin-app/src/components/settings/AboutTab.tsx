import { useTranslation } from "react-i18next";
import { ArrowsClockwise, DownloadSimple, ArrowCounterClockwise, CheckCircle, WarningCircle } from "@phosphor-icons/react";
import { ClawIcon } from "../layout/ClawIcon";
import { useGatewayStore } from "../../lib/store";
import { SectionTitle } from "./SettingsShared";
import { useAppUpdater } from "../../lib/use-app-updater";
import { ICON_SIZE } from "../../lib/ui-tokens";

export function AboutTab() {
  const { t } = useTranslation("settings");
  const gwInfo = useGatewayStore((s) => s.info);
  const {
    status,
    info,
    progress,
    error,
    checkForUpdate,
    downloadAndInstall,
    restartApp,
  } = useAppUpdater(false);

  return (
    <div className="space-y-6">
      <div className="flex flex-col items-center py-6">
        <div className="mb-4">
          <ClawIcon size={64} />
        </div>
        <h3 className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>{t("appName")}</h3>
        <p className="mt-0.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          {t("version", { version: gwInfo?.version ?? "0.1.0" })}
        </p>
      </div>

      {/* Update section */}
      <div>
        <SectionTitle>{t("softwareUpdate")}</SectionTitle>
        <div
          className="overflow-hidden rounded-[var(--radius-sm)]"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          {status === "available" && info ? (
            <div className="px-4 py-3">
              <div className="flex items-center justify-between">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                    {t("newVersionAvailable", { version: info.version })}
                  </div>
                  {info.body && (
                    <div className="mt-1 text-[12px] leading-relaxed" style={{ color: "var(--fill-tertiary)" }}>
                      {info.body.length > 120 ? `${info.body.slice(0, 120)}...` : info.body}
                    </div>
                  )}
                </div>
                <button
                  onClick={downloadAndInstall}
                  className="ml-3 flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium text-white transition-opacity duration-150 hover:opacity-80"
                  style={{ background: "var(--tint)" }}
                >
                  <DownloadSimple size={ICON_SIZE.md} />
                  {t("downloadUpdate")}
                </button>
              </div>
            </div>
          ) : status === "downloading" ? (
            <div className="px-4 py-3">
              <div className="flex items-center justify-between">
                <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>{t("downloadingUpdate")}</span>
                <span className="text-[12px] font-medium tabular-nums" style={{ color: "var(--fill-tertiary)" }}>
                  {progress}%
                </span>
              </div>
              <div className="mt-2 h-1.5 overflow-hidden rounded-full" style={{ background: "var(--fill-quaternary)" }}>
                <div
                  className="h-full rounded-full transition-[width] duration-300 ease-out"
                  style={{ width: `${progress}%`, background: "var(--tint)" }}
                />
              </div>
            </div>
          ) : status === "ready" ? (
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex items-center gap-2">
                <CheckCircle size={ICON_SIZE.md} style={{ color: "var(--green)" }} />
                <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>{t("updateReady")}</span>
              </div>
              <button
                onClick={restartApp}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium text-white transition-opacity duration-150 hover:opacity-80"
                style={{ background: "var(--green, #34C759)" }}
              >
                <ArrowCounterClockwise size={ICON_SIZE.md} />
                {t("restartNow")}
              </button>
            </div>
          ) : status === "error" ? (
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex min-w-0 items-center gap-2">
                <WarningCircle size={ICON_SIZE.md} className="shrink-0" style={{ color: "var(--red)" }} />
                <span className="truncate text-[13px]" style={{ color: "var(--fill-secondary)" }}>
                  {error ?? t("checkUpdateFailed")}
                </span>
              </div>
              <button
                onClick={checkForUpdate}
                className="ml-3 flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                {t("retry")}
              </button>
            </div>
          ) : status === "up-to-date" ? (
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex items-center gap-2">
                <CheckCircle size={ICON_SIZE.md} style={{ color: "var(--green)" }} />
                <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>{t("upToDate")}</span>
              </div>
              <button
                onClick={checkForUpdate}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                <ArrowsClockwise size={ICON_SIZE.md} />
                {t("checkAgain")}
              </button>
            </div>
          ) : (
            <div className="flex items-center justify-between px-4 py-3">
              <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
                {status === "checking" ? t("checkingUpdate") : t("checkUpdatePrompt")}
              </span>
              <button
                onClick={checkForUpdate}
                disabled={status === "checking"}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80 disabled:cursor-default disabled:opacity-50"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                <ArrowsClockwise size={ICON_SIZE.md} className={status === "checking" ? "animate-spin" : ""} />
                {t("checkUpdate")}
              </button>
            </div>
          )}
        </div>
      </div>

      <div>
        <SectionTitle>{t("infoSection")}</SectionTitle>
        {(() => {
          const rows = [
            { label: t("info_framework"), value: "Tauri 2.0 + React 19" },
            { label: t("info_backend"), value: "Rust (Tokio + Axum)" },
            { label: t("info_protocol"), value: "xiaolin-ws/1 (WebSocket)" },
            { label: t("info_license"), value: "MIT" },
          ];
          return (
            <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
              {rows.map(({ label, value }, idx) => (
                <div key={label} className="flex items-center justify-between px-4 py-2.5" style={idx < rows.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
                  <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>{label}</span>
                  <span className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>{value}</span>
                </div>
              ))}
            </div>
          );
        })()}
      </div>
    </div>
  );
}