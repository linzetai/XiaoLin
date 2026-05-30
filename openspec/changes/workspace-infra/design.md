# Workspace Infrastructure Design

## 核心设计原则

1. **零 init 启动** — 不放任何文件也能工作，放了文件就增强体验
2. **动态发现** — Skills/MCP/Rules 放对位置即被识别，无需 manifest 注册
3. **只写自己的目录** — 读取 `.cursor/`、`.codex/` 但只写 `.fastclaw/`
4. **渐进丰富** — 从零配置到全配置是连续光谱

## 三层配置模型

```
┌─────────────────────────────────────────────────┐
│  System Layer (编译时)                           │
│  内置 tools、默认 prompts、agent 框架            │
│  优先级最低，被上层覆盖                          │
└──────────────────┬──────────────────────────────┘
                   ▲
┌──────────────────┴──────────────────────────────┐
│  User Layer (~/.fastclaw/)                       │
│  config/default.json  skills/  plugins/          │
│  workspace/  credentials/                        │
│  ─────────────────────────────────               │
│  跨工具共享:                                     │
│  ~/.cursor/skills/  (只读)                       │
│  ~/.codex/skills/   (只读)                       │
│  ~/.agents/skills/  (只读，跨工具标准)           │
└──────────────────┬──────────────────────────────┘
                   ▲
┌──────────────────┴──────────────────────────────┐
│  Project Layer (<workspace-root>/.fastclaw/)     │
│  config.json   skills/   mcp.json   rules/      │
│  agents/       hooks.json  permissions.json      │
│  ─────────────────────────────────               │
│  跨工具共享 (项目级):                            │
│  <root>/.cursor/skills/  (只读)                  │
│  <root>/skills/          (兼容旧约定)            │
│  优先级最高                                      │
└─────────────────────────────────────────────────┘
```

## Workspace Root 检测

### 算法

```rust
fn detect_workspace_root(start: &Path) -> PathBuf {
    let mut current = start.canonicalize().unwrap_or(start.to_path_buf());
    loop {
        // 优先级 1: 显式标记
        if current.join(".fastclaw").is_dir() {
            return current;
        }
        // 优先级 2: VCS 根
        if current.join(".git").exists() {
            return current;
        }
        // 优先级 3: 语言标记（需要同时有 src/ 或 lib/ 避免 home 下的误判）
        let lang_markers = ["Cargo.toml", "package.json", "pyproject.toml",
                           "go.mod", "build.gradle", "pom.xml"];
        if lang_markers.iter().any(|m| current.join(m).exists()) {
            return current;
        }
        // 向上一级
        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => return start.to_path_buf(), // 回退到起始目录
        }
    }
}
```

### 使用时机

- Gateway 启动时：确定项目级配置路径
- Session 创建时：`work_dir` 默认值
- CLI 命令：确定上下文

## Session work_dir 数据流修复

### 后端

1. **WS `sessions.set_work_dir`**：新增 handler，调用现有 `SessionStore::update_work_dir`
2. **WS `sessions.list` / `sessions.get`**：响应中包含 `work_dir` 和 `source` 字段
3. **Session 创建**：`work_dir` 优先使用客户端传入值，其次用 workspace root 检测结果
4. **Title 生成后通知**：`generate_smart_title` 成功后发送 `sessions.changed`

### 前端

1. **`syncSessionsForAgent`**：从 WS 响应中正确恢复 `workDir`
2. **`setWorkDir`**：调用 `sessions.set_work_dir` WS 方法
3. **持久化**：`workDir` 通过 backend 持久化，前端不需要 localStorage

### 数据流（修复后）

```
前端 setWorkDir(chatId, path)
    │
    ▼
WS sessions.set_work_dir { session_id, work_dir }
    │
    ▼
后端 SessionStore::update_work_dir(id, path)
    │ → UPDATE sessions SET work_dir = ? WHERE id = ?
    ▼
WS broadcast sessions.changed { sessionId }
    │
    ▼
前端 onSessionChanged(sid)
    │ → getSession(sid) → 更新 workDir + title
```

## Skills 动态发现

### 发现规则

目录下包含 `SKILL.md` 的文件夹 = 一个 skill。不需要任何 manifest。

### 扫描路径（优先级从低到高）

```
用户级:
  ~/.agents/skills/              ← 跨工具共享
  ~/.codex/skills/               ← Codex skills (只读)
  ~/.cursor/skills/              ← Cursor skills (只读)
  ~/.fastclaw/skills/            ← FastClaw 用户 skills

项目级:
  <root>/skills/                 ← 通用约定（兼容现有）
  <root>/.cursor/skills/         ← Cursor 项目 skills (只读)
  <root>/.fastclaw/skills/       ← FastClaw 项目 skills（最高优先级）
```

### 合并策略

- 同 ID skill：高优先级覆盖低优先级
- 每个 skill 携带 `SkillSource` 元信息：`{ layer, origin, path }`
  - `origin`: `fastclaw`, `cursor`, `codex`, `shared`
- Agent 的 `list_skills` 工具显示来源信息

## 项目级 MCP 配置

### 文件路径

```
<workspace-root>/.fastclaw/mcp.json
```

### 格式（对齐 Cursor）

```json
{
  "mcpServers": {
    "my-db": {
      "command": "npx",
      "args": ["@modelcontextprotocol/server-postgres", "postgresql://..."],
      "env": { "PG_HOST": "localhost" },
      "enabled": true
    }
  }
}
```

### 合并策略

- 项目级 + 用户级并存
- 同 ID server：项目级优先
- 项目级可用 `"enabled": false` 屏蔽用户级的 server
- 项目级 MCP 只在 workspace root 匹配时加载

## 项目级 Rules

### 文件路径

```
<workspace-root>/.fastclaw/rules/*.md
```

### 格式

Markdown 文件，可选 YAML frontmatter：

```markdown
---
name: coding-standards
alwaysApply: true
globs: ["*.rs", "*.ts"]
---

# 项目编码规范

1. 所有公开 API 必须有文档注释
2. 错误处理使用 anyhow::Result
...
```

### 加载

- 扫描 `.fastclaw/rules/` 下所有 `.md` 文件
- 注入到 system prompt（类似 Cursor 的 workspace rules）
- `alwaysApply: true` → 始终注入
- `globs` → 仅当操作匹配文件时注入

## 按工作区分组会话

### 分组逻辑

```typescript
function groupSessionsByWorkspace(sessions: Chat[]): Map<string, Chat[]> {
    const groups = new Map<string, Chat[]>();
    for (const session of sessions) {
        const key = normalizeWorkDir(session.workDir) || "未关联项目";
        groups.get(key)?.push(session) ?? groups.set(key, [session]);
    }
    return groups;
}

function normalizeWorkDir(dir: string | null): string | null {
    if (!dir) return null;
    // 去除 home 前缀，只显示相对路径
    return dir.replace(homedir, "~");
}
```

### UI 结构

```
侧边栏:
├── 🔍 搜索
├── ➕ 新建对话
├── 📁 ~/workspace/my_tools/FastClaw (3)
│   ├── feat(wechat): 实现微信 channel      12分钟前
│   ├── 探索项目级配置基建方案               刚才
│   └── 修复 CDN 上传问题                    昨天
├── 📁 ~/workspace/other-project (1)
│   └── 代码审查                             3天前
└── 📁 未关联项目 (2)
    ├── 随便聊聊                             上周
    └── 学习 Rust                            2周前
```

## Agent 元能力

### 内置 Skill: fastclaw-config-manager

安装位置：`~/.fastclaw/skills/fastclaw-config-manager/SKILL.md`
首次启动时自动创建。

内容：描述 `.fastclaw/` 完整目录结构、文件格式、操作约定。
让 agent 在用户说"帮我加个 skill"时知道怎么做。

### 内置工具

| 工具 | 功能 |
|------|------|
| `list_project_config` | 列出当前项目的 skills, MCP, rules |
| `add_project_skill` | 创建 `.fastclaw/skills/<name>/SKILL.md` |
| `add_mcp_server` | 在 `.fastclaw/mcp.json` 添加条目 |
| `remove_mcp_server` | 从 `.fastclaw/mcp.json` 移除条目 |

### 上下文注入

Gateway 启动时自动扫描项目配置，将摘要注入 system prompt：

```
当前项目: ~/workspace/my_tools/FastClaw
├── Skills: 5 个 (3 项目级, 2 来自 Cursor)
├── MCP Servers: 1 个 (my-db)
├── Rules: 2 条
└── 配置目录: .fastclaw/ (已创建)
```
