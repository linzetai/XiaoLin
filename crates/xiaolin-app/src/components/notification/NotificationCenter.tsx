import { useTranslation } from "react-i18next";
import {
  useState,
  useEffect,
  useCallback,
  useRef,
  type ReactNode,
} from "react";
import { Bell, Check, Checks, Trash, Clock, Warning, Info, Lightning } from "@phosphor-icons/react";
import { ICON_SIZE } from "../../lib/ui-tokens";
import * as transport from "../../lib/transport";
import { listen } from "@tauri-apps/api/event";

const isTauri =
  typeof window !== "undefined" &&
  ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);

function parseUtc(ts: string): Date {
  if (!ts || ts.endsWith("Z") || /[+-]\d{2}:\d{2}$/.test(ts)) return new Date(ts);
  return new Date(ts.replace(" ", "T") + "Z");
}

function relativeTime(iso: string, tr: (key: string, opts?: Record<string, unknown>) => string): string {
  const diff = Date.now() - parseUtc(iso).getTime();
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return tr("justNow");
  const mins = Math.floor(secs / 60);
  if (mins < 60) return tr("minutesAgo", { count: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return tr("hoursAgo", { count: hours });
  const days = Math.floor(hours / 24);
  if (days < 30) return tr("daysAgo", { count: days });
  return parseUtc(iso).toLocaleDateString();
}

function categoryIcon(category?: string): ReactNode {
  switch (category) {
    case "cron":
      return <Clock size={ICON_SIZE.md} />;
    case "agent":
      return <Lightning size={ICON_SIZE.md} />;
    case "error":
      return <Warning size={ICON_SIZE.md} />;
    default:
      return <Info size={ICON_SIZE.md} />;
  }
}

interface Props {
  onDetailOpen?: (notification: transport.AppNotification) => void;
}

export function NotificationCenter({ onDetailOpen }: Props) {
  const { t } = useTranslation("notification");
  const [open, setOpen] = useState(false);
  const [unreadCount, setUnreadCount] = useState(0);
  const [notifications, setNotifications] = useState<transport.AppNotification[]>([]);
  const [loading, setLoading] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  const fetchUnreadCount = useCallback(async () => {
    try {
      const { count } = await transport.notificationUnreadCount();
      setUnreadCount(count);
    } catch {
      /* ignore */
    }
  }, []);

  const fetchNotifications = useCallback(async () => {
    setLoading(true);
    try {
      const { notifications: items, unreadCount: uc } = await transport.notificationList(30);
      setNotifications(items);
      setUnreadCount(uc);
    } catch {
      /* ignore */
    } finally {
      setLoading(false);
    }
  }, []);

  // Initial load + listen for push events
  useEffect(() => {
    fetchUnreadCount();

    const cleanups: Array<() => void> = [];

    if (isTauri) {
      let cancelled = false;
      Promise.resolve().then(() => {
        if (cancelled) return;

        listen<{ unreadCount?: number }>("notification-new", (ev) => {
          const uc = ev.payload?.unreadCount;
          if (typeof uc === "number") setUnreadCount(uc);
          // Refresh list if dropdown is open
          fetchNotifications();
        }).then((u) => { if (cancelled) u(); else cleanups.push(u); });

        listen<{ unreadCount?: number }>("notification-read", (ev) => {
          const uc = ev.payload?.unreadCount;
          if (typeof uc === "number") setUnreadCount(uc);
          fetchNotifications();
        }).then((u) => { if (cancelled) u(); else cleanups.push(u); });
      });

      cleanups.push(() => { cancelled = true; });
    }

    return () => cleanups.forEach((fn) => fn());
  }, [fetchUnreadCount, fetchNotifications]);

  // Close dropdown on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  const handleToggle = useCallback(() => {
    setOpen((prev) => {
      const next = !prev;
      if (next) fetchNotifications();
      return next;
    });
  }, [fetchNotifications]);

  const handleMarkRead = useCallback(
    async (id: string, e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        const { unreadCount: uc } = await transport.notificationMarkRead(id);
        setUnreadCount(uc);
        setNotifications((prev) =>
          prev.map((n) => (n.id === id ? { ...n, isRead: true } : n)),
        );
      } catch {
        /* ignore */
      }
    },
    [],
  );

  const handleMarkAllRead = useCallback(async () => {
    try {
      const { unreadCount: uc } = await transport.notificationMarkAllRead();
      setUnreadCount(uc);
      setNotifications((prev) => prev.map((n) => ({ ...n, isRead: true })));
    } catch {
      /* ignore */
    }
  }, []);

  const handleItemClick = useCallback(
    async (n: transport.AppNotification) => {
      if (!n.isRead) {
        try {
          const { unreadCount: uc } = await transport.notificationMarkRead(n.id);
          setUnreadCount(uc);
          setNotifications((prev) =>
            prev.map((item) => (item.id === n.id ? { ...item, isRead: true } : item)),
          );
        } catch {
          /* ignore */
        }
      }
      if ((n.body || n.detail) && onDetailOpen) {
        onDetailOpen(n);
        setOpen(false);
      }
    },
    [onDetailOpen],
  );

  const handleDelete = useCallback(
    async (id: string, e: React.MouseEvent) => {
      e.stopPropagation();
      try {
        await transport.notificationDelete(id);
        setNotifications((prev) => prev.filter((n) => n.id !== id));
        fetchUnreadCount();
      } catch {
        /* ignore */
      }
    },
    [fetchUnreadCount],
  );

  return (
    <div className="relative flex items-center" ref={dropdownRef}>
      {/* Bell button */}
      <button
        onClick={handleToggle}
        className="relative flex h-7 w-7 items-center justify-center rounded-md transition-all duration-100 hover:bg-[var(--bg-hover)] active:scale-95"
        style={{ color: "var(--fill-quaternary)" }}
        title={t("centerTitle")}
      >
        <Bell />
        {unreadCount > 0 && (
          <span
            key={unreadCount}
            className="absolute flex items-center justify-center rounded-full text-white font-medium"
            style={{
              top: "1px",
              right: "1px",
              minWidth: "14px",
              height: "14px",
              padding: "0 3px",
              fontSize: "9px",
              lineHeight: 1,
              background: "var(--red, #E53E3E)",
              animation: "badge-bounce var(--duration-normal) var(--ease-spring)",
            }}
          >
            {unreadCount > 99 ? "99+" : unreadCount}
          </span>
        )}
      </button>

      {/* Dropdown panel */}
      {open && (
        <div
          className="absolute right-0 z-50 flex flex-col overflow-hidden rounded-lg"
          style={{
            top: "calc(var(--titlebar-h) - 2px)",
            width: "340px",
            maxHeight: "420px",
            background: "var(--bg-primary)",
            border: "0.5px solid var(--separator)",
            boxShadow: "0 8px 30px rgba(0,0,0,0.25)",
            animation: "scale-spring var(--duration-normal) var(--ease-spring)",
            transformOrigin: "top right",
          }}
        >
          {/* Header */}
          <div
            className="flex items-center justify-between px-3 py-2"
            style={{ borderBottom: "0.5px solid var(--separator)" }}
          >
            <span
              className="text-[13px] font-semibold"
              style={{ color: "var(--fill-primary)" }}
            >
              {t("centerTitle")}
            </span>
            {unreadCount > 0 && (
              <button
                onClick={handleMarkAllRead}
                className="flex items-center gap-1 rounded px-2 py-0.5 text-[11px] transition-colors hover:bg-[var(--bg-hover)]"
                style={{ color: "var(--blue)" }}
                title={t("markAllReadTitle")}
              >
                <Checks />
                {t("markAllRead")}
              </button>
            )}
          </div>

          {/* List */}
          <div className="flex-1 overflow-y-auto" style={{ scrollbarGutter: "stable" }}>
            {loading && notifications.length === 0 ? (
              <div
                className="flex items-center justify-center py-10 text-[12px]"
                style={{ color: "var(--fill-quaternary)" }}
              >
                {t("loading")}
              </div>
            ) : notifications.length === 0 ? (
              <div
                className="flex flex-col items-center justify-center py-10 gap-2"
                style={{ color: "var(--fill-quaternary)" }}
              >
                <Bell size={28} weight="light" />
                <span className="text-[12px]">{t("empty")}</span>
              </div>
            ) : (
              notifications.map((n, i) => (
                <div
                  key={n.id}
                  onClick={() => handleItemClick(n)}
                  className="group flex items-start gap-2.5 px-3 py-2.5 transition-colors duration-75 cursor-pointer hover:bg-[var(--bg-hover)]"
                  style={{
                    borderBottom: "0.5px solid var(--separator-light, var(--separator))",
                    opacity: n.isRead ? 0.6 : 1,
                    animation: `fade-slide-up var(--duration-normal) var(--ease-out) ${i * 30}ms backwards`,
                  }}
                >
                  {/* Unread dot */}
                  <div className="flex items-center pt-1" style={{ width: "8px" }}>
                    {!n.isRead && (
                      <span
                        className="inline-block h-[6px] w-[6px] rounded-full"
                        style={{ background: "var(--blue, #3B82F6)" }}
                      />
                    )}
                  </div>

                  {/* Icon */}
                  <div
                    className="flex items-center justify-center rounded-md mt-0.5"
                    style={{
                      width: "26px",
                      height: "26px",
                      flexShrink: 0,
                      background: "var(--bg-secondary)",
                      color: "var(--fill-secondary)",
                    }}
                  >
                    {categoryIcon(n.category)}
                  </div>

                  {/* Content */}
                  <div className="flex-1 min-w-0">
                    <div
                      className="text-[12px] font-medium truncate"
                      style={{ color: "var(--fill-primary)" }}
                    >
                      {n.title}
                    </div>
                    {n.body && (
                      <div
                        className="mt-0.5 text-[11px]"
                        style={{
                          color: "var(--fill-tertiary)",
                          display: "-webkit-box",
                          WebkitLineClamp: 2,
                          WebkitBoxOrient: "vertical" as const,
                          overflow: "hidden",
                        }}
                      >
                        {n.body}
                      </div>
                    )}
                    <div className="mt-1 flex items-center gap-2">
                      <span
                        className="text-[10px]"
                        style={{ color: "var(--fill-quaternary)" }}
                      >
                        {relativeTime(n.createdAt, (k, o) => t(k, o))}
                      </span>
                      {(n.body || n.detail) && (
                        <span
                          className="text-[10px]"
                          style={{ color: "var(--tint)" }}
                        >
                          {t("viewDetails")}
                        </span>
                      )}
                    </div>
                  </div>

                  {/* Actions */}
                  <div className="flex items-center gap-0.5 pt-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                    {!n.isRead && (
                      <button
                        onClick={(e) => handleMarkRead(n.id, e)}
                        className="flex items-center justify-center rounded p-1 hover:bg-[var(--bg-active)]"
                        style={{ color: "var(--fill-tertiary)" }}
                        title={t("markRead")}
                      >
                        <Check />
                      </button>
                    )}
                    <button
                      onClick={(e) => handleDelete(n.id, e)}
                      className="flex items-center justify-center rounded p-1 hover:bg-[var(--bg-active)]"
                      style={{ color: "var(--fill-tertiary)" }}
                      title={t("delete")}
                    >
                      <Trash />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>
      )}
    </div>
  );
}
