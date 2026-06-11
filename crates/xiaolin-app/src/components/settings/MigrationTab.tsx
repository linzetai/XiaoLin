import { useState } from "react";
import { useTranslation } from "react-i18next";
import { DownloadSimple, UploadSimple, ArrowCounterClockwise, Warning, CheckCircle, XCircle } from "@phosphor-icons/react";
import * as transport from "../../lib/transport";
import { ICON_SIZE } from "../../lib/ui-tokens";

export function MigrationTab() {
  const { t } = useTranslation("settings");
  const [exportStatus, setExportStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [importStatus, setImportStatus] = useState<"idle" | "loading" | "success" | "error">("idle");
  const [progress, setProgress] = useState(0);
  const [options, setOptions] = useState({
    includeSessions: true,
    includeSkills: true,
    includeAgentWorkspaces: false,
  });

  const handleExport = async () => {
    if (!transport.isTauri) {
      alert(t("exportDesktopOnly"));
      return;
    }

    setExportStatus("loading");
    setProgress(0);

    try {
      // 模拟进度更新
      const progressInterval = setInterval(() => {
        setProgress(prev => Math.min(prev + 10, 90));
      }, 200);

      const data = await transport.exportData({
        includeSessions: options.includeSessions,
        includeSkills: options.includeSkills,
        includeAgentWorkspaces: options.includeAgentWorkspaces,
      });

      clearInterval(progressInterval);
      setProgress(95);

      // 保存文件
      const fileName = `xiaolin-backup-${new Date().toISOString().split('T')[0]}.fcdata`;
      const { save } = await import("@tauri-apps/plugin-dialog");
      const filePath = await save({
        filters: [{
          name: "XiaoLin Data File",
          extensions: ["fcdata"]
        }],
        defaultPath: fileName
      });
      if (filePath) {
        const { writeFile } = await import("@tauri-apps/plugin-fs");
        await writeFile(filePath, data);
        setProgress(100);
        setExportStatus("success");
        setTimeout(() => setExportStatus("idle"), 3000);
      } else {
        setExportStatus("idle");
      }
    } catch (error) {
      setExportStatus("error");
      console.error("导出失败:", error);
      setTimeout(() => setExportStatus("idle"), 3000);
    }
  };

  const handleImport = async () => {
    if (!transport.isTauri) {
      alert(t("importDesktopOnly"));
      return;
    }

    setImportStatus("loading");
    setProgress(0);

    try {
      // 模拟进度更新
      const progressInterval = setInterval(() => {
        setProgress(prev => Math.min(prev + 5, 50));
      }, 300);

      // 选择文件
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        filters: [{
          name: "XiaoLin Data Files",
          extensions: ["fcdata", "json"]
        }],
        multiple: false
      });

      if (selected) {
        const { readFile } = await import("@tauri-apps/plugin-fs");
        const fileContents = await readFile(selected as string);
        clearInterval(progressInterval);
        setProgress(70);

        // {t("importData")}
        await transport.importData(new Uint8Array(fileContents), {
          merge: false,
          overwriteConfig: true,
          overwriteAgents: true,
          overwriteSessions: true,
          overwriteSkills: true
        });

        setProgress(100);
        setImportStatus("success");
        setTimeout(() => setImportStatus("idle"), 3000);
      } else {
        setImportStatus("idle");
      }
    } catch (error) {
      setImportStatus("error");
      console.error("导入失败:", error);
      setTimeout(() => setImportStatus("idle"), 3000);
    }
  };

  const getStatusIcon = (status: string) => {
    switch (status) {
      case "success": return <CheckCircle size={ICON_SIZE.md} style={{ color: "var(--green)" }} />;
      case "error": return <XCircle size={ICON_SIZE.md} style={{ color: "var(--red)" }} />;
      case "loading": return <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-[var(--fill-primary)]" />;
      default: return null;
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-[16px] font-semibold mb-1" style={{ color: "var(--fill-primary)" }}>
          {t("migrationTitle")}
        </h3>
        <p className="text-[13px] text-[var(--fill-secondary)]">
          {t("migrationDesc")}
        </p>
      </div>

      {/* 导出部分 */}
      <div
        className="rounded-[var(--radius-md)] p-4"
        style={{ background: "var(--bg-base)", border: "1px solid var(--separator)" }}
      >
        <div className="flex items-center gap-2 mb-3">
          <DownloadSimple size={ICON_SIZE.md} style={{ color: "var(--blue)" }} />
          <h4 className="font-medium" style={{ color: "var(--fill-primary)" }}>
            {t("exportData")}
          </h4>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("includeSessions")}
            </span>
            <label className="switch">
              <input
                type="checkbox"
                checked={options.includeSessions}
                onChange={(e) => setOptions({...options, includeSessions: e.target.checked})}
                className="switch-checkbox"
              />
              <span className="switch-slider" />
            </label>
          </div>

          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("includeSkills")}
            </span>
            <label className="switch">
              <input
                type="checkbox"
                checked={options.includeSkills}
                onChange={(e) => setOptions({...options, includeSkills: e.target.checked})}
                className="switch-checkbox"
              />
              <span className="switch-slider" />
            </label>
          </div>

          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("includeWorkspaces")}
            </span>
            <label className="switch">
              <input
                type="checkbox"
                checked={options.includeAgentWorkspaces}
                onChange={(e) => setOptions({...options, includeAgentWorkspaces: e.target.checked})}
                className="switch-checkbox"
              />
              <span className="switch-slider" />
            </label>
          </div>

          <div className="pt-2">
            <button
              onClick={handleExport}
              disabled={exportStatus === "loading"}
              className="w-full flex items-center justify-center gap-2 py-2.5 rounded-[var(--radius-sm)] transition-colors disabled:opacity-50"
              style={{
                background: "var(--blue)",
                color: "white"
              }}
            >
              {getStatusIcon(exportStatus)}
              {exportStatus === "loading" ? t("exporting", { progress }) : t("exportData")}
            </button>
          </div>
        </div>
      </div>

      {/* 导入部分 */}
      <div
        className="rounded-[var(--radius-md)] p-4"
        style={{ background: "var(--bg-base)", border: "1px solid var(--separator)" }}
      >
        <div className="flex items-center gap-2 mb-3">
          <UploadSimple size={ICON_SIZE.md} style={{ color: "var(--green)" }} />
          <h4 className="font-medium" style={{ color: "var(--fill-primary)" }}>
            {t("importData")}
          </h4>
        </div>

        <div className="space-y-3">
          <div className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
            {t("importDesc")}
          </div>

          <div className="pt-2">
            <button
              onClick={handleImport}
              disabled={importStatus === "loading"}
              className="w-full flex items-center justify-center gap-2 py-2.5 rounded-[var(--radius-sm)] transition-colors disabled:opacity-50"
              style={{
                background: "var(--green)",
                color: "white"
              }}
            >
              {getStatusIcon(importStatus)}
              {importStatus === "loading" ? t("importing", { progress }) : t("selectFileImport")}
            </button>
          </div>

          <div className="flex items-start gap-2 pt-2">
            <Warning  className="mt-0.5 flex-shrink-0" style={{ color: "var(--yellow)" }} />
            <p className="text-[12px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
              {t("importWarning")}
            </p>
          </div>
        </div>
      </div>

      {/* 合并导入选项（高级） */}
      <details
        className="rounded-[var(--radius-md)] p-4"
        style={{ background: "var(--bg-base)", border: "1px solid var(--separator)" }}
      >
        <summary className="cursor-pointer font-medium flex items-center gap-2" style={{ color: "var(--fill-primary)" }}>
          <ArrowCounterClockwise size={ICON_SIZE.md} />
          <span>{t("advancedOptions")}</span>
        </summary>
        <div className="pt-4 space-y-3">
          <div className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            {t("importBehavior")}
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              {t("mergeNotOverwrite")}
            </span>
            <label className="switch">
              <input
                type="checkbox"
                checked={false} // 默认为 false，表示覆盖
                onChange={() => {}}
                className="switch-checkbox"
                disabled
              />
              <span className="switch-slider" />
            </label>
          </div>
        </div>
      </details>
    </div>
  );
}