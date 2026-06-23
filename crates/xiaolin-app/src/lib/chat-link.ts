import { useWorkspaceTabs } from "../components/shell/workspace-tabs";
import { useBrowserStore } from "./stores/browser-store";
import { useConfigStore, type ChatLinkTarget } from "./stores/config-store";
import { isTauri } from "./transport";

export function isHttpUrl(url: string): boolean {
  try {
    const protocol = new URL(url).protocol.toLowerCase();
    return protocol === "http:" || protocol === "https:";
  } catch {
    return false;
  }
}

async function openExternal(url: string): Promise<void> {
  if (isTauri) {
    try {
      const { open } = await import("@tauri-apps/plugin-shell");
      await open(url);
      return;
    } catch (e) {
      console.warn("[chat-link] external open failed, falling back to window.open:", e);
    }
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

async function openBuiltin(url: string): Promise<void> {
  if (!isTauri) {
    await openExternal(url);
    return;
  }
  const pageId = await useBrowserStore.getState().openPage(url);
  if (!pageId) {
    console.warn("[chat-link] builtin open failed, falling back to external");
    await openExternal(url);
    return;
  }

  const { layoutMode } = useBrowserStore.getState();
  if (layoutMode === "fullwidth") {
    await useBrowserStore.getState().showActivePage();
    return;
  }

  const focusBrowserPanel = () => {
    const tabs = useWorkspaceTabs.getState().tabs;
    if (tabs.some((t) => t.id === "browser")) {
      useWorkspaceTabs.getState().setActiveTab("browser");
      return true;
    }
    return false;
  };

  if (!focusBrowserPanel()) {
    requestAnimationFrame(() => {
      if (!focusBrowserPanel()) setTimeout(focusBrowserPanel, 0);
    });
  }
}

export function resolveChatLinkTarget(
  defaultTarget: ChatLinkTarget,
  shiftKey: boolean,
): "builtin" | "external" {
  if (shiftKey) {
    return defaultTarget === "builtin" ? "external" : "builtin";
  }
  return defaultTarget;
}

export async function handleChatLinkClick(url: string, shiftKey: boolean): Promise<void> {
  if (!isHttpUrl(url)) return;

  const defaultTarget = useConfigStore.getState().display.chatLinkTarget;
  const target = resolveChatLinkTarget(defaultTarget, shiftKey);

  if (target === "builtin") {
    await openBuiltin(url);
  } else {
    await openExternal(url);
  }
}
