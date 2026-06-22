import { type ReactNode, useEffect, useRef } from "react";
import { GitBranch, Crosshair, Terminal, Robot, FileText, FolderOpen } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { AppHeader } from "./AppHeader";
import { AppSidebar } from "./AppSidebar";
import { ContentBlock } from "./ContentBlock";
import { useWorkspaceTabs } from "./workspace-tabs";
import { ReviewTabContent, ReviewTabFooter } from "./ReviewTabContent";
import { GoalTabContent } from "./GoalPanel";
import { TerminalTabContent } from "./TerminalTabContent";
import { SubAgentsTabContent } from "./CoordinatorPanel";
import { PlanTabContent } from "../message-stream/PlanPanel";
import { FilesTabContent } from "./FilesTabContent";
import { useGitStore, useTerminalStore, useActiveSubAgentRuns } from "../../lib/stores";
import { useChatMetaStore } from "../../lib/stores/chat-meta-store";
import { useFileViewerStore } from "../../lib/stores/file-viewer-store";

export function AppShell({ children }: { children: ReactNode }) {
  const { t } = useTranslation("sidebar");
  const registerTab = useWorkspaceTabs((s) => s.registerTab);
  const unregisterTab = useWorkspaceTabs((s) => s.unregisterTab);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const gitStatus = useGitStore((s) => s.status);
  const terminalSessions = useTerminalStore((s) => s.sessions);
  const subAgentRuns = useActiveSubAgentRuns();
  const activeChatId = useChatMetaStore((s) => s.activeChatId);
  const activeChat = useChatMetaStore((s) => s.chats[s.activeChatId]);
  const prevChatRef = useRef<string | null>(null);

  useEffect(() => {
    if (activeChatId !== prevChatRef.current) {
      useWorkspaceTabs.getState().switchSession(activeChatId, prevChatRef.current ?? undefined);
      useFileViewerStore.getState().switchSession(activeChatId, prevChatRef.current);
      prevChatRef.current = activeChatId;
    }
  }, [activeChatId]);

  useEffect(() => {
    registerTab({
      id: "review",
      label: "Review",
      icon: GitBranch,
      component: ReviewTabContent,
      footerComponent: ReviewTabFooter,
      order: 1,
    });
    registerTab({
      id: "files",
      label: "Files",
      icon: FolderOpen,
      component: FilesTabContent,
      order: 2,
    });
    registerTab({
      id: "goal",
      label: "Goal",
      icon: Crosshair,
      component: GoalTabContent,
      order: 3,
    });
    registerTab({
      id: "terminal",
      label: t("terminal"),
      icon: Terminal,
      component: TerminalTabContent,
      order: 4,
    });
  }, [registerTab, t]);

  useEffect(() => {
    const hasSubAgents = Object.keys(subAgentRuns).length > 0;
    if (hasSubAgents) {
      registerTab({
        id: "subagents",
        label: "SubAgents",
        icon: Robot,
        component: SubAgentsTabContent,
        order: 5,
      });
      const { activeTabId: currentTab, panelOpen } = useWorkspaceTabs.getState();
      if (currentTab !== "subagents" && !panelOpen) {
        useWorkspaceTabs.getState().setActiveTab("subagents");
      }
    } else {
      unregisterTab("subagents");
    }
  }, [subAgentRuns, registerTab, unregisterTab]);

  // Plan tab — show when plan mode is active or plan file exists
  const isPlanMode = activeChat?.executionMode === "plan";
  const hasPlanFile = activeChat?.planFileExists === true;
  useEffect(() => {
    if (isPlanMode || hasPlanFile) {
      registerTab({
        id: "plan",
        label: "Plan",
        icon: FileText,
        component: PlanTabContent,
        order: 0,
      });
      const { activeTabId: currentTab, panelOpen, planClosedByUser } = useWorkspaceTabs.getState();
      if (currentTab !== "plan" && !panelOpen && !planClosedByUser) {
        useWorkspaceTabs.getState().setActiveTab("plan");
      }
    } else {
      unregisterTab("plan");
    }
  }, [isPlanMode, hasPlanFile, registerTab, unregisterTab]);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { path?: string; line?: number; workDir?: string } | undefined;
      if (!detail?.path) return;

      const chatId = useChatMetaStore.getState().activeChatId;
      const chat = useChatMetaStore.getState().chats[chatId];
      const resolvedWorkDir = detail.workDir ?? chat?.workDir ?? "";
      if (!resolvedWorkDir) return;

      void useFileViewerStore.getState().openFile(detail.path, resolvedWorkDir, detail.line);
      useWorkspaceTabs.getState().setActiveTab("files");
    };

    window.addEventListener("xiaolin:open-file", handler);
    return () => window.removeEventListener("xiaolin:open-file", handler);
  }, []);

  useEffect(() => {
    if (!gitStatus?.isGitRepo) return;
    const changeCount = (gitStatus.staged?.length ?? 0) + (gitStatus.unstaged?.length ?? 0) + (gitStatus.untracked?.length ?? 0);
    const tabs = useWorkspaceTabs.getState().tabs;
    const reviewTab = tabs.find((t) => t.id === "review");
    if (!reviewTab) return;
    const shouldBadge = activeTabId !== "review" && changeCount > 0;
    if (reviewTab.badge !== (shouldBadge ? changeCount : undefined)) {
      useWorkspaceTabs.setState((s) => ({
        tabs: s.tabs.map((t) => t.id === "review" ? { ...t, badge: shouldBadge ? changeCount : undefined } : t),
      }));
    }
  }, [gitStatus, activeTabId]);

  useEffect(() => {
    const runningCount = Object.values(terminalSessions).filter((s) => s.status === "running").length;
    const shouldBadge = activeTabId !== "terminal" && runningCount > 0;
    useWorkspaceTabs.setState((s) => ({
      tabs: s.tabs.map((t) => t.id === "terminal" ? { ...t, badge: shouldBadge ? true : undefined } : t),
    }));
  }, [terminalSessions, activeTabId]);

  return (
    <div
      className="app-shell-new"
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        minHeight: 0,
        overflow: "hidden",
      }}
    >
      <AppHeader />
      <div
        className="shell-body"
        style={{
          display: "flex",
          flexDirection: "row",
          flex: 1,
          minHeight: 0,
          background: "var(--bg-shell)",
        }}
      >
        <AppSidebar />
        <ContentBlock>{children}</ContentBlock>
      </div>
    </div>
  );
}
