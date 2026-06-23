import {
  HostMappingConfirmPanel,
  useBrowserNetworkConfirmListener,
} from "./HostMappingConfirmPanel";

/** Fixed overlay for Agent-initiated network change confirmations. */
export function BrowserNetworkConfirmOverlay() {
  const { pendingConfirm, dismissConfirm } = useBrowserNetworkConfirmListener();

  if (!pendingConfirm) return null;

  return (
    <div
      className="pointer-events-none fixed inset-x-0 bottom-0 z-[70] flex justify-center pb-4"
      style={{ paddingLeft: "max(16px, env(safe-area-inset-left))", paddingRight: "max(16px, env(safe-area-inset-right))" }}
    >
      <div className="pointer-events-auto w-full max-w-lg">
        <HostMappingConfirmPanel request={pendingConfirm} onResolved={dismissConfirm} />
      </div>
    </div>
  );
}
