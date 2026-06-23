import { useTranslation } from "react-i18next";
import {
  HostMappingConfirmPanel,
  useBrowserNetworkConfirmListener,
} from "./HostMappingConfirmPanel";

/** Fixed overlay for Agent-initiated network change confirmations. */
export function BrowserNetworkConfirmOverlay() {
  const { t } = useTranslation("browser");
  const { pendingConfirm, pendingCount, dismissConfirm } = useBrowserNetworkConfirmListener();

  if (!pendingConfirm) return null;

  return (
    <div
      className="pointer-events-none fixed inset-x-0 bottom-0 z-[70] flex justify-center pb-4"
      style={{ paddingLeft: "max(16px, env(safe-area-inset-left))", paddingRight: "max(16px, env(safe-area-inset-right))" }}
    >
      <div className="pointer-events-auto w-full max-w-lg">
        {pendingCount > 1 && (
          <div
            className="mb-2 text-center text-[11px]"
            style={{ color: "var(--fill-tertiary)" }}
          >
            {t("pendingConfirms", { total: pendingCount })}
          </div>
        )}
        <HostMappingConfirmPanel
          key={pendingConfirm.requestId}
          request={pendingConfirm}
          onResolved={dismissConfirm}
        />
      </div>
    </div>
  );
}
