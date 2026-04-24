import { ClawIcon } from "../layout/ClawIcon";
import { useGatewayStore } from "../../lib/store";
import { SectionTitle } from "./SettingsShared";

export function AboutTab() {
  const gwInfo = useGatewayStore((s) => s.info);
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