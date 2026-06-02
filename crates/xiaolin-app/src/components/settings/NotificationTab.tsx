import { SectionTitle, SettingRow, Toggle } from "./SettingsShared";

export function NotificationTab({
  notifications,
  setNotifications,
  sounds,
  setSounds,
  autoScroll,
  setAutoScroll,
  loaded,
  persist,
}: {
  notifications: boolean;
  setNotifications: (v: boolean) => void;
  sounds: boolean;
  setSounds: (v: boolean) => void;
  autoScroll: boolean;
  setAutoScroll: (v: boolean) => void;
  loaded: boolean;
  persist: (key: string, value: boolean) => void;
}) {
  return (
    <div>
      <SectionTitle>行为</SectionTitle>
      <div className="overflow-hidden rounded-[var(--radius-sm)]" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
        <SettingRow label="桌面通知" description="Agent 完成回复时推送通知">
          <Toggle
            enabled={notifications}
            onChange={() => {
              setNotifications(!notifications);
              if (loaded) persist("notifications", !notifications);
            }}
          />
        </SettingRow>
        <SettingRow label="提示音" description="收到消息时播放提示音">
          <Toggle
            enabled={sounds}
            onChange={() => {
              setSounds(!sounds);
              if (loaded) persist("sounds", !sounds);
            }}
          />
        </SettingRow>
        <SettingRow label="自动滚动" description="新消息时自动滚动到底部" isLast>
          <Toggle
            enabled={autoScroll}
            onChange={() => {
              setAutoScroll(!autoScroll);
              if (loaded) persist("autoScroll", !autoScroll);
            }}
          />
        </SettingRow>
      </div>
    </div>
  );
}
