import { useState } from "react";
import { Download, Upload, RotateCcw, AlertTriangle, CheckCircle, XCircle } from "lucide-react";
import * as transport from "../../lib/transport";
import { ICON } from "../../lib/ui-tokens";

export function MigrationTab() {
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
      alert("导出功能仅在桌面应用中可用");
      return;
    }

    setExportStatus("loading");
    setProgress(0);

    try {
      // 模拟进度更新
      const progressInterval = setInterval(() => {
        setProgress(prev => Math.min(prev + 10, 90));
      }, 200);

      // 导出数据
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
      alert("导入功能仅在桌面应用中可用");
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

        // 导入数据
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
      case "success": return <CheckCircle {...ICON.md} style={{ color: "var(--green)" }} />;
      case "error": return <XCircle {...ICON.md} style={{ color: "var(--red)" }} />;
      case "loading": return <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-[var(--fill-primary)]" />;
      default: return null;
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-[16px] font-semibold mb-1" style={{ color: "var(--fill-primary)" }}>
          数据迁移
        </h3>
        <p className="text-[13px] text-[var(--fill-secondary)]">
          导出或导入您的配置、代理、技能和会话数据
        </p>
      </div>

      {/* 导出部分 */}
      <div
        className="rounded-[var(--radius-md)] p-4"
        style={{ background: "var(--bg-base)", border: "1px solid var(--separator)" }}
      >
        <div className="flex items-center gap-2 mb-3">
          <Download {...ICON.md} style={{ color: "var(--blue)" }} />
          <h4 className="font-medium" style={{ color: "var(--fill-primary)" }}>
            导出数据
          </h4>
        </div>

        <div className="space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              包括会话历史记录
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
              包括技能
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
              包括代理工作目录
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
              {exportStatus === "loading" ? `导出中... ${progress}%` : "导出数据"}
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
          <Upload {...ICON.md} style={{ color: "var(--green)" }} />
          <h4 className="font-medium" style={{ color: "var(--fill-primary)" }}>
            导入数据
          </h4>
        </div>

        <div className="space-y-3">
          <div className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
            从备份文件导入配置、代理、技能和会话数据
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
              {importStatus === "loading" ? `导入中... ${progress}%` : "选择文件导入"}
            </button>
          </div>

          <div className="flex items-start gap-2 pt-2">
            <AlertTriangle {...ICON.sm} className="mt-0.5 flex-shrink-0" style={{ color: "var(--yellow)" }} />
            <p className="text-[12px] leading-relaxed" style={{ color: "var(--fill-secondary)" }}>
              注意：导入操作将覆盖当前配置。如有重要数据，请先执行导出备份。
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
          <RotateCcw {...ICON.md} />
          <span>高级选项</span>
        </summary>
        <div className="pt-4 space-y-3">
          <div className="text-[13px]" style={{ color: "var(--fill-tertiary)" }}>
            配置导入时的行为
          </div>
          <div className="flex items-center justify-between">
            <span className="text-[13px]" style={{ color: "var(--fill-secondary)" }}>
              合并而非覆盖
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