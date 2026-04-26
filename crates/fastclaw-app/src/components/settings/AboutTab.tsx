import { RefreshCw, Download, RotateCcw, CheckCircle, AlertCircle } from "lucide-react";
import { ClawIcon } from "../layout/ClawIcon";
import { useGatewayStore } from "../../lib/store";
import { SectionTitle } from "./SettingsShared";
import { useAppUpdater } from "../../lib/use-app-updater";

export function AboutTab() {
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
        <h3 className="text-[16px] font-semibold" style={{ color: "var(--fill-primary)" }}>FastClaw</h3>
        <p className="mt-0.5 text-[12px]" style={{ color: "var(--fill-tertiary)" }}>
          版本 {gwInfo?.version ?? "0.1.0"}
        </p>
      </div>

      {/* Update section */}
      <div>
        <SectionTitle>软件更新</SectionTitle>
        <div
          className="overflow-hidden rounded-[var(--radius-sm)]"
          style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}
        >
          {status === "available" && info ? (
            <div className="px-4 py-3">
              <div className="flex items-center justify-between">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium" style={{ color: "var(--fill-primary)" }}>
                    新版本 {info.version} 可用
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
                  <Download size={13} />
                  下载更新
                </button>
              </div>
            </div>
          ) : status === "downloading" ? (
            <div className="px-4 py-3">
              <div className="flex items-center justify-between">
                <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>正在下载更新...</span>
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
                <CheckCircle size={14} style={{ color: "var(--green)" }} />
                <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>更新已下载，重启后生效</span>
              </div>
              <button
                onClick={restartApp}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium text-white transition-opacity duration-150 hover:opacity-80"
                style={{ background: "var(--green, #34C759)" }}
              >
                <RotateCcw size={13} />
                立即重启
              </button>
            </div>
          ) : status === "error" ? (
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex min-w-0 items-center gap-2">
                <AlertCircle size={14} className="shrink-0" style={{ color: "var(--red)" }} />
                <span className="truncate text-[13px]" style={{ color: "var(--fill-secondary)" }}>
                  {error ?? "检查更新失败"}
                </span>
              </div>
              <button
                onClick={checkForUpdate}
                className="ml-3 flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                重试
              </button>
            </div>
          ) : status === "up-to-date" ? (
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex items-center gap-2">
                <CheckCircle size={14} style={{ color: "var(--green)" }} />
                <span className="text-[13px]" style={{ color: "var(--fill-primary)" }}>已是最新版本</span>
              </div>
              <button
                onClick={checkForUpdate}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                <RefreshCw size={12} />
                再次检查
              </button>
            </div>
          ) : (
            <div className="flex items-center justify-between px-4 py-3">
              <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
                {status === "checking" ? "正在检查更新..." : "检查是否有新版本可用"}
              </span>
              <button
                onClick={checkForUpdate}
                disabled={status === "checking"}
                className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-xs)] px-3 py-1.5 text-[12px] font-medium transition-colors duration-150 hover:opacity-80 disabled:cursor-default disabled:opacity-50"
                style={{ background: "var(--fill-quaternary)", color: "var(--fill-primary)" }}
              >
                <RefreshCw size={12} className={status === "checking" ? "animate-spin" : ""} />
                检查更新
              </button>
            </div>
          )}
        </div>
      </div>

      <div>
        <SectionTitle>信息</SectionTitle>
        {(() => {
          const rows = [
            { label: "框架", value: "Tauri 2.0 + React 19" },
            { label: "后端", value: "Rust (Tokio + Axum)" },
            { label: "协议", value: "fastclaw-ws/1 (WebSocket)" },
            { label: "许可证", value: "MIT" },
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