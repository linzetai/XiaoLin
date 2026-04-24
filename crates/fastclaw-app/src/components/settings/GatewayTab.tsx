import { useState, useEffect } from "react";
import { useGatewayStore } from "../../lib/store";
import * as api from "../../lib/api";
import { SectionTitle } from "./SettingsShared";


export function GatewayTab() {
  const gwInfo = useGatewayStore((s) => s.info);
  const gwMode = useGatewayStore((s) => s.mode);
  const connected = useGatewayStore((s) => s.connected);

  const [gwConfig, setGwConfig] = useState<{ port?: number; host?: string } | null>(null);

  useEffect(() => {
    api.getConfig("gateway").then((data) => {
      const cfg = data as { key?: string; value?: { port?: number; host?: string } } | null;
      setGwConfig((cfg?.value ?? cfg) as { port?: number; host?: string } | null);
    }).catch(() => {});
  }, []);

  const modeLabel = gwMode === "embedded" ? "内嵌网关" : gwMode === "remote" ? "远程网关" : gwMode === "browser" ? "浏览器开发" : "连接中...";

  return (
    <div className="space-y-6">
      <div className="space-y-2">
        <SectionTitle>运行状态</SectionTitle>
        {(() => {
          const rows = [
            { label: "模式", value: modeLabel },
            { label: "状态", value: connected ? "已连接" : "未连接", dot: connected },
            ...(gwInfo ? [
              { label: "端口", value: String(gwInfo.port) },
              { label: "版本", value: gwInfo.version },
              { label: "WebSocket", value: gwInfo.wsUrl },
              { label: "HTTP", value: gwInfo.httpUrl },
            ] : []),
          ];
          return (
            <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
              {rows.map(({ label, value, dot }, idx) => (
                <div key={label} className="flex items-center justify-between gap-3 px-4 py-2.5" style={idx < rows.length - 1 ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
                  <span className="shrink-0 text-[13px]" style={{ color: "var(--fill-secondary)" }}>{label}</span>
                  <div className="flex min-w-0 items-center gap-1.5">
                    {dot !== undefined && (
                      <span className="inline-block h-[6px] w-[6px] shrink-0 rounded-full" style={{ background: dot ? "var(--green)" : "var(--red)" }} />
                    )}
                    <span className="min-w-0 truncate text-[13px] font-medium font-mono" style={{ color: "var(--fill-primary)" }} title={value}>{value}</span>
                  </div>
                </div>
              ))}
            </div>
          );
        })()}
      </div>
      {gwConfig && (
        <div>
          <SectionTitle>配置</SectionTitle>
          <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="px-4 py-2.5" style={gwConfig.host ? { borderBottom: "0.5px solid var(--separator)" } : undefined}>
              <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>配置端口</span>
              <div className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{gwConfig.port ?? "默认"}</div>
            </div>
            {gwConfig.host && (
              <div className="px-4 py-2.5">
                <span className="text-[11px]" style={{ color: "var(--fill-tertiary)" }}>绑定地址</span>
                <div className="text-[13px] font-mono" style={{ color: "var(--fill-primary)" }}>{gwConfig.host}</div>
              </div>
            )}
          </div>
          <p className="mt-2 text-[11px]" style={{ color: "var(--fill-quaternary)" }}>
            修改网关配置需编辑 ~/.fastclaw/config/default.json 并重启
          </p>
        </div>
      )}
    </div>
  );
}