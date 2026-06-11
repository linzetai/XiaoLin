import { type ReactNode, useEffect } from "react";
import { GitBranch, Crosshair, Terminal } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";
import { AppHeader } from "./AppHeader";
import { AppSidebar } from "./AppSidebar";
import { ContentBlock } from "./ContentBlock";
import { useWorkspaceTabs } from "./workspace-tabs";
import { ReviewTabContent, ReviewTabFooter } from "./ReviewTabContent";
import { GoalTabContent } from "./GoalPanel";
import { TerminalTabContent } from "./TerminalTabContent";
import { useGitStore, useTerminalStore } from "../../lib/stores";

export function AppShell({ children }: { children: ReactNode }) {
  const { t } = useTranslation("sidebar");
  const registerTab = useWorkspaceTabs((s) => s.registerTab);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const gitStatus = useGitStore((s) => s.status);
  const terminalSessions = useTerminalStore((s) => s.sessions);

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
      id: "goal",
      label: "Goal",
      icon: Crosshair,
      component: GoalTabContent,
      order: 2,
    });
    registerTab({
      id: "terminal",
      label: t("terminal"),
      icon: Terminal,
      component: TerminalTabContent,
      order: 3,
    });
  }, [registerTab, t]);

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
          paddingRight: 3,
        }}
      >
        <AppSidebar />
        <ContentBlock>{children}</ContentBlock>
      </div>
    </div>
  );
}
