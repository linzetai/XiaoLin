## Sidebar Project UI — 设计规格

### 设计理念

**多项目统一管理中枢**——Sidebar 分为两个清晰的逻辑区块：**Projects**（已关联项目的会话）和 **Chats**（未关联项目的独立会话）。用户主动选择"绑定项目"才会将会话提升到 Projects 区块，新建会话默认落入 Chats。

### 美学方向

延续现有 Geist 字体 + 冷灰色系 token 体系，不引入额外字体。Projects 区块通过彩色圆点（project.color）和折叠/展开的微交互带来层次感。整体保持紧凑信息密度（单行 30px 高）。

---

## ADDED Requirements

### Requirement: Sidebar 双区块布局

AppSidebar 中间滚动区 SHALL 划分为两个独立区块：

1. **Projects 区块** — 标题 "Projects"，列出所有非归档的 Project 实体，每个 Project 可折叠展开显示其下属 sessions
2. **Chats 区块** — 标题 "Chats"，列出所有 `projectId == null` 的散落会话

两区块之间由 8px 间距分隔（无分隔线）。

#### Scenario: 区块渲染顺序
- **WHEN** 渲染 Sidebar 中间区域
- **THEN** 先渲染 "Projects" 区块，再渲染 "Chats" 区块
- **AND** Projects 区块内 pinned 项目置顶，其余按 `lastOpenedAt` 降序
- **AND** Chats 区块内按 `createdAt` 降序

#### Scenario: 空态显示
- **WHEN** Projects 区块无任何 project
- **THEN** 不渲染 Projects 区块标题和内容（Sidebar 仅显示 Chats）
- **WHEN** Chats 区块无任何 session
- **THEN** 仍渲染 "Chats" 标题，下方显示空态提示 "还没有独立会话"

---

### Requirement: Project 行渲染

每个 Project 在 Projects 区块中渲染为一个可折叠的分组行。

#### 视觉规格
- 行高 32px，padding `6px 10px`，圆角 `var(--radius-xs)`
- 左侧：8px 实心圆点，颜色为 `project.color`
- 中间：项目名称（13px, `--fill-secondary`, font-weight 500, 单行截断）
- 右侧（hover 显示）：
  - session 计数 badge（11px, `--fill-quaternary`）
  - "+" 按钮（在 project 下创建新会话）
  - 展开/折叠箭头（ChevronRight/ChevronDown, 12px）

#### Scenario: Project 行交互
- **WHEN** 用户点击 Project 行
- **THEN** 切换该 Project 下的 sessions 折叠/展开状态
- **AND** 箭头图标旋转 90° 动画（`var(--duration-fast) var(--ease-out)`）

#### Scenario: Project 下新建会话
- **WHEN** 用户点击 Project 行 hover 出现的 "+" 按钮
- **THEN** 创建一个新会话，自动设置 `workDir = project.rootPath` 和 `projectId = project.id`
- **AND** 会话出现在该 Project 下方的 sessions 列表中

#### Scenario: Project 右键菜单
- **WHEN** 用户右键点击 Project 行
- **THEN** 显示上下文菜单，包含：
  - "重命名" — 行内编辑 project.name
  - "更改颜色" — 展开颜色选择面板（8 色预设 + 自定义 hex）
  - "置顶" / "取消置顶" — 切换 project.pinned
  - "归档" — 设置 project.archived = true，从列表移除
  - "从列表移除" — 删除 project 记录（不影响 sessions）

#### Scenario: Project 不可达状态
- **WHEN** `project.reachable === false`（目录不存在）
- **THEN** 项目名称显示为 `--fill-quaternary` 色 + 删除线样式
- **AND** hover tooltip 显示 "项目目录不可达：{rootPath}"

---

### Requirement: Project 下的 Session 列表

展开 Project 后显示其关联的 sessions。

#### 视觉规格
- 缩进 24px（相对 Project 行）
- 每项：MessageCircle 图标（14px）+ 标题（13px, `--fill-secondary`, 单行截断）+ 时间（11px, `--fill-quaternary`）
- 激活态：`--bg-active` 背景 + font-weight 500 + `--fill-primary` 色
- hover：`--bg-hover` 背景

#### Scenario: Session 项点击
- **WHEN** 用户点击 Project 下的 session 项
- **THEN** 切换到该会话（设置 activeChatId）

#### Scenario: Session 项右键
- **WHEN** 用户右键 session 项
- **THEN** 显示菜单：重命名、移动到其他项目、取消关联项目（移回 Chats）、删除

---

### Requirement: Chats 区块（散落会话）

"Chats" 区块显示所有 `projectId == null` 的会话。

#### 视觉规格
- 标题行："Chats"（11px, `--fill-quaternary`, font-weight 500）
- 列表项：与 Project 下的 session 列表样式完全一致（无缩进）

#### Scenario: 新建会话默认归属
- **WHEN** 用户点击顶部 "New chat" 按钮
- **THEN** 创建无 projectId、无 workDir 的新会话
- **AND** 会话出现在 "Chats" 区块

#### Scenario: 关联项目（从 Chats 提升到 Projects）
- **WHEN** 用户通过 StreamFooter 的 "Work locally" 下拉菜单选择一个项目
- **THEN** session 的 projectId 和 workDir 更新，会话从 Chats 移动到 Projects 区块

---

### Requirement: "Work locally" 下拉菜单（ProjectDropdown）

StreamFooter 输入框下方的 "Work locally" 按钮改造为**下拉菜单触发器**。这是设置 session 关联的主要入口。

#### 触发器视觉规格（保持现有 chip 样式）
- 已关联项目时：显示 `🟢 项目名`（彩色圆点 + 名称缩略）+ 下拉箭头 `▾`
- 未关联时：显示 `Monitor 图标 + "Work locally"` + 下拉箭头 `▾`（当前已有的样式）

#### 下拉面板视觉规格
- 弹出方向：向上弹出（popover anchor = bottom of panel, 面板出现在按钮上方）
- 最大宽度 320px，最大高度 360px（溢出滚动）
- 背景 `--bg-elevated`，圆角 `--radius-sm`，阴影 `--shadow-lg`
- 边框 `0.5px solid var(--separator)`

#### 面板内容结构（从上到下）
1. **搜索框**（sticky top）— placeholder "搜索项目或输入路径..."
2. **已有项目列表**（来自 `useProjectStore.projects`，按 lastOpenedAt 排序）
   - 每项：彩色圆点(8px) + 项目名(13px) + rootPath 缩略(11px, `--fill-quaternary`)
   - 当前已选中的项目有 ✓ 标记
3. **分隔线** (`1px solid var(--separator)`, margin 4px 0)
4. **"浏览文件夹..."** — FolderOpen 图标 + 文字，打开系统目录选择对话框
5. **"不使用项目"** — X 图标 + 文字（仅当当前已关联时显示）

#### Scenario: 打开下拉菜单
- **WHEN** 用户点击 "Work locally" 按钮
- **THEN** 弹出下拉面板，显示已有项目列表和操作选项
- **AND** 搜索框自动 focus

#### Scenario: 选择已有项目
- **WHEN** 用户在下拉菜单中点击一个已有项目
- **THEN** 设置 session 的 `projectId` 和 `workDir`（= project.rootPath）
- **AND** 下拉菜单关闭
- **AND** 按钮文字更新为 `🟢 项目名`
- **AND** session 移动到 Sidebar 的 Projects 区块对应组

#### Scenario: 浏览文件夹
- **WHEN** 用户点击 "浏览文件夹..."
- **THEN** 关闭下拉菜单，打开系统原生目录选择对话框
- **AND** 选择目录后自动 `projects.detect` → `projects.create`（如不存在）
- **AND** 设置 session 的 projectId 和 workDir
- **AND** 按钮文字更新

#### Scenario: 搜索过滤
- **WHEN** 用户在搜索框输入文字
- **THEN** 实时过滤项目列表（按 name 和 rootPath 模糊匹配）
- **AND** 无匹配时显示 "无匹配项目" 提示

#### Scenario: 取消关联
- **WHEN** 当前 session 已有 projectId，用户选择 "不使用项目"
- **THEN** 清除 session 的 projectId（workDir 也清除）
- **AND** session 回到 Sidebar 的 Chats 区块
- **AND** 按钮恢复为 "Work locally"

#### Scenario: 点击外部关闭
- **WHEN** 下拉菜单打开时，用户点击面板外区域或按 Escape
- **THEN** 关闭下拉菜单

---

### Requirement: 颜色选择面板

Project 右键 "更改颜色" 弹出的选色面板。

#### 预设色板（8 色）
```
#2563EB  (蓝)
#7C3AED  (紫)
#EC4899  (粉)
#EF4444  (红)
#F97316  (橙)
#EAB308  (黄)
#22C55E  (绿)
#06B6D4  (青)
```

#### 视觉规格
- 小型浮层，2×4 网格排列
- 每个色块 24×24，圆角 50%，hover 放大 1.1x
- 当前色块有 2px 白色内环 + check mark overlay

---

## 技术实现方案

### 数据流

```
useProjectStore.projects  ──┐
                             ├─→  AppSidebar  ──→  Projects 区块
useChatMetaStore.chats    ──┘                  ──→  Chats 区块
```

### 分组逻辑（useMemo in AppSidebar）

```typescript
const { projectGroups, looseChats } = useMemo(() => {
  const projectGroups: Array<{
    project: ProjectSummary;
    sessions: ChatMeta[];
  }> = [];
  const looseChats: ChatMeta[] = [];

  const projectMap = useProjectStore.getState().projects;

  for (const chat of chatList) {
    if (chat.projectId && projectMap[chat.projectId]) {
      let group = projectGroups.find(g => g.project.id === chat.projectId);
      if (!group) {
        group = { project: projectMap[chat.projectId], sessions: [] };
        projectGroups.push(group);
      }
      group.sessions.push(chat);
    } else {
      looseChats.push(chat);
    }
  }

  // Sort: pinned first, then by lastOpenedAt
  projectGroups.sort((a, b) => {
    if (a.project.pinned !== b.project.pinned) return b.project.pinned ? 1 : -1;
    return new Date(b.project.lastOpenedAt).getTime() - new Date(a.project.lastOpenedAt).getTime();
  });

  // Sort sessions within each group by createdAt desc
  for (const g of projectGroups) {
    g.sessions.sort((a, b) => {
      const ta = a.createdAt instanceof Date ? a.createdAt.getTime() : 0;
      const tb = b.createdAt instanceof Date ? b.createdAt.getTime() : 0;
      return tb - ta;
    });
  }

  // Sort loose chats by createdAt desc
  looseChats.sort((a, b) => {
    const ta = a.createdAt instanceof Date ? a.createdAt.getTime() : 0;
    const tb = b.createdAt instanceof Date ? b.createdAt.getTime() : 0;
    return tb - ta;
  });

  return { projectGroups, looseChats };
}, [chatList, projects]);
```

### 组件拆分

| 组件 | 职责 |
|------|------|
| `AppSidebar` | 顶层容器，编排 Projects 区块和 Chats 区块 |
| `ProjectGroup` | 单个 Project 折叠组：行 + 子 session 列表 |
| `SessionItem` | 单个 session 行（复用于 ProjectGroup 内和 Chats 区块） |
| `ProjectDropdown` | StreamFooter 中的项目选择下拉菜单（替代原 handleOpenWorkDir） |
| `ColorPicker` | 8 色预设选色面板 |
| `ProjectContextMenu` | Project 右键菜单 |
| `SessionContextMenu` | Session 右键菜单（扩展现有） |

### 折叠状态管理

在 `useUIStore` 中新增：

```typescript
collapsedProjects: Record<string, boolean>;
toggleProjectCollapsed: (projectId: string) => void;
```

默认所有项目展开（collapsed = false）。

---

## 交互流程图

### 会话从 Chats 提升到 Projects（通过 StreamFooter 下拉菜单）

```
用户点击 StreamFooter "Work locally" 按钮
  → 弹出 ProjectDropdown 面板（向上弹出）
    → 选择已有项目
      → setSessionWorkDir(sessionId, project.rootPath)
        → 后端自动 find_or_create_project + 绑定 project_id
          → session 从 Chats 移动到 Projects 对应组
            → broadcast sessions.changed + projects.changed

    → 浏览文件夹
      → 系统目录选择对话框
        → projects.detect(path)
          → projects.create(rootPath) (如不存在)
            → setSessionWorkDir(sessionId, rootPath)

    → 不使用项目
      → setSessionWorkDir(sessionId, null)
        → 清除 project_id
          → session 回到 Chats 区块
```

### Project 下新建会话

```
用户 hover Project 行 → 显示 "+" 按钮
  → 点击 "+"
    → newChat(project.rootPath)
      → session 创建（带 workDir + projectId）
        → session 出现在该 Project 下
```
