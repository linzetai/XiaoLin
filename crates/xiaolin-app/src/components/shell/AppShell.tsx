import { type ReactNode, useEffect } from "react";
import { GitBranch, Target } from "lucide-react";
import { AppHeader } from "./AppHeader";
import { AppSidebar } from "./AppSidebar";
import { ContentBlock } from "./ContentBlock";
import { useWorkspaceTabs } from "./workspace-tabs";
import { ReviewTabContent, ReviewTabFooter } from "./ReviewTabContent";
import { GoalTabContent } from "./GoalPanel";
import { useGitStore } from "../../lib/stores";

export function AppShell({ children }: { children: ReactNode }) {
  const registerTab = useWorkspaceTabs((s) => s.registerTab);
  const activeTabId = useWorkspaceTabs((s) => s.activeTabId);
  const gitStatus = useGitStore((s) => s.status);

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
      icon: Target,
      component: GoalTabContent,
      order: 2,
    });
  }, [registerTab]);

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
