import {
  HostMappingConfirmPanel,
  useBrowserNetworkConfirmListener,
} from "./HostMappingConfirmPanel";

/** Fixed overlay for Agent-initiated network change confirmations. */
export function BrowserNetworkConfirmOverlay() {
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
            待确认 {pendingCount} 项（当前第 1 项）
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
