## 1. 基础设施

- [x] 1.1 安装 `@phosphor-icons/react` 依赖
- [x] 1.2 建立 Lucide → Phosphor 图标名映射表（覆盖项目中所有使用的图标）
- [x] 1.3 更新 `ui-tokens.ts`：定义 ICON_SIZE scale、ICON_WEIGHT 语义映射、ICON_COLOR 常量
- [x] 1.4 在 App 根组件添加 `IconContext.Provider`（size=14, weight="regular", color="currentColor"）

## 2. 迁移 shell 组件

- [x] 2.1 迁移 `AppSidebar.tsx`（~12 个图标实例）
- [x] 2.2 迁移 `AppHeader.tsx`（window controls + 布局切换图标）
- [x] 2.3 迁移 `AppShell.tsx`
- [x] 2.4 迁移 `TerminalTabContent.tsx`
- [x] 2.5 迁移 `TerminalPanel.tsx`
- [x] 2.6 迁移 `WorkspacePanel.tsx`
- [x] 2.7 迁移 `SearchPanel.tsx`
- [x] 2.8 迁移 `GoalPanel.tsx`
- [x] 2.9 迁移 `ReviewTabContent.tsx`
- [x] 2.10 迁移 `WelcomeView.tsx`

## 3. 迁移 message-stream 组件

- [x] 3.1 迁移 `StepIndicator.tsx`（~20 个工具图标映射）
- [x] 3.2 迁移 `MessageRenderer.tsx`
- [x] 3.3 迁移 `ComposerCore.tsx`
- [x] 3.4 迁移 `MessageStream.tsx`
- [x] 3.5 迁移 `ToolCallCard.tsx` + `ToolCallGroup.tsx`
- [x] 3.6 迁移 `PermissionSelector.tsx`
- [x] 3.7 迁移 `ProjectDropdown.tsx`
- [x] 3.8 迁移 `StickyContextBar.tsx` + `StreamFooter.tsx`
- [x] 3.9 迁移 `ReviewTab.tsx` + `DiffCard.tsx`
- [x] 3.10 迁移 `PlanPanel.tsx` + `PlanApprovalCard.tsx` + `ApprovalCard.tsx`
- [x] 3.11 迁移 `SubAgentCard.tsx` + `SubAgentMonitor.tsx`
- [x] 3.12 迁移 `TodoCard.tsx` + `QueuePanel.tsx` + `QueueIndicator.tsx`
- [x] 3.13 迁移 `FileChangesCard.tsx` + `BriefMessageCard.tsx`
- [x] 3.14 迁移 `MentionInput.tsx` + `UserInput.tsx`
- [x] 3.15 迁移 `StreamEmptyState.tsx` + `MarkdownContent.tsx`
- [x] 3.16 迁移 `StepGroup.tsx`

## 4. 迁移 settings/layout/其他组件

- [x] 4.1 迁移 `SettingsPanel.tsx` + 各 Tab 组件（ModelTab, McpManager, LlmPluginTab, WebSearchTab, SkillsTab, SubAgentsTab, SecurityTab, AboutTab, MigrationTab）
- [x] 4.2 迁移 `SessionList.tsx`
- [x] 4.3 迁移 `ConnectionsPage.tsx`
- [x] 4.4 迁移 `TasksPage.tsx`
- [x] 4.5 迁移 `CostDashboard.tsx`
- [x] 4.6 迁移 `NotificationCenter.tsx` + `NotificationDetailPanel.tsx`
- [x] 4.7 迁移 layout 组件（`TitleBar.tsx`, `UpdateBanner.tsx`, `AppLayout.tsx`）
- [x] 4.8 迁移 onboarding 组件（ModelStep, ModelSelectStep, ProviderSelectStep, ApiKeyConfigStep, DoneStep, FeaturesStep, WelcomeStep, ModelSavedConfirmation, SubStepBreadcrumb）
- [x] 4.9 迁移 `PluginsView.tsx` + `AutomationView.tsx` + `CronScheduleHelper.tsx`
- [x] 4.10 迁移 common 组件（`ImageLightbox.tsx`, `ContextMenu.tsx`, `ComingSoon.tsx`）
- [x] 4.11 迁移 `GoalStatusCard.tsx`
- [x] 4.12 迁移 `App.tsx`

## 5. 清理与验证

- [x] 5.1 移除 `lucide-react` 依赖（pnpm remove lucide-react）
- [x] 5.2 删除 `ui-tokens.ts` 中旧的 ICON/ICON_ACTIVE_STROKE 常量（确认无引用后）
- [x] 5.3 运行 `npx tsc --noEmit` 确认零 TypeScript 错误
- [x] 5.4 启动 dev server 进行视觉回归检查
- [x] 5.5 对比迁移前后关键页面截图（sidebar, chat, settings）
