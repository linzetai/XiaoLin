import { type ReactNode, useEffect } from "react";
import { GitBranch } from "lucide-react";
import { AppHeader } from "./AppHeader";
import { AppSidebar } from "./AppSidebar";
import { ContentBlock } from "./ContentBlock";
import { useWorkspaceTabs } from "./workspace-tabs";
import { ReviewTabContent, ReviewTabFooter } from "./ReviewTabContent";
export function AppShell({ children }: { children: ReactNode }) {
  const registerTab = useWorkspaceTabs((s) => s.registerTab);

  useEffect(() => {
    registerTab({
      id: "review",
      label: "Review",
      icon: GitBranch,
      component: ReviewTabContent,
      footerComponent: ReviewTabFooter,
      order: 1,
    });
  }, [registerTab]);

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
