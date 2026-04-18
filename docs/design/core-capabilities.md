# FastClaw 三大核心能力技术方案

> **日期**: 2026-04-18  
> **状态**: Design  
> **目标**: 将长任务编排、代码智能、Skill 自动形成从当前 2-3/5 提升至 4/5

---

## 一、DAG 进阶编排

### 1.1 当前问题

| 问题 | 影响 |
|------|------|
| 节点/图级无超时 | 长 LLM 调用可无限 hang |
| HumanApproval 是 auto-approve stub | 无法做真正的人工审批门控 |
| Condition 只支持 `"true"/"false"` | 无法表达 JSONPath/表达式条件 |
| DAG 严格无环 | 无法表达 "修改→测试→不通过→再修改" 循环 |
| 失败即全图终止 | 无节点级重试、跳过、补偿策略 |

### 1.2 技术方案

#### 1.2.1 节点超时 + 图超时

```rust
// definition.rs — NodeDef 新增
pub struct NodeDef {
    // ... existing fields
    pub timeout_ms: Option<u64>,     // 节点级超时，默认 None = 无限
    pub retry_policy: RetryPolicy,   // 节点级重试
}

pub struct RetryPolicy {
    pub max_retries: u32,       // 默认 0 = 不重试
    pub backoff_ms: u64,        // 重试间隔基数
    pub backoff_multiplier: f64, // 指数退避倍数
}

// DagExecutor 新增
pub struct DagExecutor {
    // ... existing
    pub graph_timeout_ms: Option<u64>,  // 全图超时
}
```

**执行器改造**：
- `run_one` 包裹 `tokio::time::timeout(node.timeout_ms)`
- 超时后 `NodeState::Failed`，根据 `retry_policy` 决定是否重试
- 全图用 `tokio::select!` 监控 `graph_timeout_ms`

#### 1.2.2 真正的 HITL (Human-in-the-Loop)

```rust
// 新增 approval.rs
#[async_trait]
pub trait ApprovalGate: Send + Sync {
    async fn request_approval(
        &self,
        dag_id: &str,
        node_id: &str,
        context: &serde_json::Value,
    ) -> Result<ApprovalDecision>;
}

pub enum ApprovalDecision {
    Approved(serde_json::Value),  // 可附带审批者输入
    Rejected(String),             // 拒绝原因
    Timeout,                      // 等待超时
}
```

**集成**：
- `DagExecutor` 持有 `Option<Arc<dyn ApprovalGate>>`
- `HumanApproval` 节点执行时调用 gate，若无 gate 则 auto-approve（向后兼容）
- Gateway 实现 `WebhookApprovalGate`：发 HTTP 回调 → 存 pending → 等 API 确认

#### 1.2.3 条件表达式引擎

```rust
// 新增 expression.rs
pub fn evaluate_condition(
    expression: &str,
    context: &serde_json::Value,
) -> Result<String> // 返回分支标签
```

**实现**：
- 内置 mini 表达式引擎（基于 `serde_json` 值操作）
- 支持：`$.upstream_node.output.score > 0.8` → `"pass"` / `"fail"`
- 支持：`$.input.language == "rust"` → `"rust_path"` / `"default"`
- 运算符：`==`, `!=`, `>`, `<`, `>=`, `<=`, `&&`, `||`, `!`
- 使用 JSONPointer（`/`分隔）或点号路径访问嵌套值

#### 1.2.4 可控循环（受限循环子图）

```rust
// definition.rs 新增
pub enum NodeKind {
    // ... existing
    /// 受限循环：最多执行 max_iterations 次，退出条件由 condition 决定
    Loop {
        max_iterations: u32,  // 硬上限，防无限循环
        condition_node: String, // 引用一个 Condition/Reflect 节点作为退出判断
    },
}
```

**实现思路**：
- 不破坏 DAG 无环约束：`Loop` 节点展开为 N 层副本（类似循环展开）
- 或者：executor 内部维护 loop counter，对 loop 子图反复调度直到条件满足或达上限
- 推荐后者，更灵活，`validate_acyclic` 对 Loop 节点的回边特殊处理

#### 1.2.5 节点失败策略

```rust
pub enum FailurePolicy {
    Abort,           // 默认：失败即终止全图
    Skip,            // 标记为 Skipped，继续后续节点
    Retry,           // 按 RetryPolicy 重试
    Fallback(String), // 跳转到指定备用节点
}
```

### 1.3 测试计划

- `dag_node_timeout_aborts_hung_node` — 节点超时后状态为 Failed
- `dag_retry_policy_retries_on_failure` — 失败节点按策略重试
- `dag_condition_jsonpath_expression` — 条件表达式路由到正确分支
- `dag_loop_max_iterations_respected` — 循环到达上限后退出
- `dag_human_approval_blocks_until_confirmed` — HITL 门控阻塞等待

---

## 二、代码智能增强

### 2.1 当前问题

| 问题 | 影响 |
|------|------|
| 仅符号级索引 | 无法理解 "谁调用了这个函数" |
| 无跨文件引用 | 重构时无法追踪影响范围 |
| 沙箱仅子进程+超时 | 不够安全，无资源隔离 |
| 自动修复只改 prompt | 无法实际修改代码文件并验证 |

### 2.2 技术方案

#### 2.2.1 调用图与引用索引

```rust
// index.rs 扩展
pub struct CodeGraph {
    pub files: HashMap<PathBuf, FileEntry>,
    pub call_edges: Vec<CallEdge>,        // 函数调用关系
    pub import_graph: Vec<ImportEdge>,     // 模块导入关系
    pub reference_index: HashMap<String, Vec<Reference>>, // 符号 → 使用位置
}

pub struct CallEdge {
    pub caller: SymbolRef,  // (file, symbol_name)
    pub callee: String,     // 被调用的符号名
    pub line: u32,
}

pub struct Reference {
    pub file: PathBuf,
    pub line: u32,
    pub kind: ReferenceKind, // Definition, Call, Import, TypeRef
}
```

**实现**：
- Tree-sitter 遍历 `call_expression` / `method_invocation` 节点
- 构建 caller→callee 边（同文件内精确，跨文件基于名称匹配）
- 为每个符号维护引用列表
- 提供 `find_callers(symbol)` / `find_callees(symbol)` / `find_references(symbol)` API

#### 2.2.2 真实测试执行 + 结果解析

```rust
// sandbox.rs 扩展
pub struct TestRunner {
    pub working_dir: PathBuf,
    pub timeout_secs: u64,
}

pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub errors: Vec<TestError>,
    pub output: String,
}

pub struct TestError {
    pub test_name: String,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub message: String,
}
```

**实现**：
- 支持 `cargo test`（Rust）、`pytest`（Python）、`npm test`（JS/TS）、`go test`（Go）
- 解析测试输出提取 pass/fail/error 结构化信息
- 超时 + 输出截断保护
- 提供 `run_tests(language, working_dir)` 统一 API

#### 2.2.3 Patch 生成与应用

```rust
// 新增 patch.rs
pub struct CodePatch {
    pub file: PathBuf,
    pub hunks: Vec<PatchHunk>,
}

pub struct PatchHunk {
    pub start_line: u32,
    pub end_line: u32,
    pub old_content: String,
    pub new_content: String,
    pub description: String,
}

pub struct PatchEngine {
    pub working_dir: PathBuf,
}

impl PatchEngine {
    /// 应用 patch，返回修改的文件列表
    pub fn apply(&self, patches: &[CodePatch]) -> Result<Vec<PathBuf>>;
    
    /// 回滚所有 patch（基于备份）
    pub fn rollback(&self) -> Result<()>;
    
    /// 应用 → 运行测试 → 失败则回滚
    pub async fn apply_and_verify(
        &self,
        patches: &[CodePatch],
        test_runner: &TestRunner,
    ) -> Result<PatchVerifyResult>;
}
```

**自迭代引擎改造**：
- `SelfIterEngine::iterate` 扩展为可以输出 `CodePatch`（而非仅 prompt hint）
- 循环：诊断 → 生成 patch → apply → run tests → 失败则 rollback + 诊断 → 生成新 patch
- 最多 N 轮（可配置，默认 5）

### 2.3 测试计划

- `code_graph_finds_callers_of_function` — Rust 代码中找到函数的调用者
- `code_graph_cross_file_references` — 跨文件引用追踪
- `test_runner_parses_cargo_test_output` — cargo test 结果解析
- `test_runner_parses_pytest_output` — pytest 结果解析
- `patch_apply_and_rollback` — patch 应用与回滚
- `patch_apply_verify_rollback_on_test_failure` — 测试失败时自动回滚

---

## 三、Skill 自动形成（对标 Hermes）

### 3.1 当前问题

| 问题 | 影响 |
|------|------|
| 无对话轨迹完整记录 | 无法回溯 "怎么解决的" |
| 无任务类型识别 | 无法知道 "这是什么类型的任务" |
| 进化系统只改全局 prompt | 无法形成特定任务的 skill |
| Skill 只有手动创建 | 无自动发现和提取 |
| 无 skill 生命周期 | 无使用统计、无淘汰机制 |

### 3.2 核心概念

```
对话轨迹 → 任务识别 → 模式聚类 → Skill 提取 → 参数化存储 → 检索复用 → 反馈迭代
```

### 3.3 技术方案

#### 3.3.1 对话轨迹记录

```rust
// 新增 trajectory.rs in fastclaw-evolution
pub struct Trajectory {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub task_type: Option<String>,       // LLM 或规则推断
    pub steps: Vec<TrajectoryStep>,
    pub outcome: TrajectoryOutcome,
    pub created_at: String,
}

pub struct TrajectoryStep {
    pub role: String,           // user/assistant/tool
    pub action_type: String,    // message/tool_call/tool_result
    pub tool_name: Option<String>,
    pub summary: String,        // 精简摘要（非原文）
    pub success: Option<bool>,
}

pub enum TrajectoryOutcome {
    Success { user_rating: Option<f64> },
    Failure { reason: String },
    Abandoned,
    Unknown,
}
```

**采集时机**：
- Session 结束（TTL 过期 / 用户关闭）时，从消息历史提取轨迹
- 使用规则（关键词）+ 可选 LLM 推断 `task_type`
- 存入 SQLite `trajectories` 表

#### 3.3.2 模式聚类与 Skill 提取

```rust
// 新增 skill_extractor.rs in fastclaw-evolution
pub struct SkillExtractor {
    pub min_occurrences: u32,     // 至少出现 N 次才提取
    pub similarity_threshold: f64, // 相似度阈值
}

pub struct ExtractedSkill {
    pub name: String,
    pub task_pattern: String,       // 任务类型描述
    pub strategy_template: String,  // 参数化策略模板
    pub parameters: Vec<SkillParam>,
    pub source_trajectories: Vec<String>, // 来源轨迹 ID
    pub success_rate: f64,
    pub usage_count: u32,
}

pub struct SkillParam {
    pub name: String,
    pub param_type: String,   // string/number/enum
    pub description: String,
    pub default_value: Option<String>,
}
```

**提取流程**：
1. 收集同 `task_type` 的成功轨迹
2. 对轨迹的 step 序列做模式匹配（工具调用序列相似度）
3. 当某模式出现 ≥ `min_occurrences` 次 → 触发提取
4. 使用 LLM（通过 `DistillationCallback`）将具体步骤泛化为参数化模板
5. 生成 `ExtractedSkill` 存入 `skills` 表

#### 3.3.3 Skill 存储与生命周期

```rust
// 扩展 skill.rs 或新增 skill_store.rs
pub struct SkillStore {
    pool: SqlitePool,
}

impl SkillStore {
    pub async fn save_skill(&self, skill: &ExtractedSkill) -> Result<()>;
    pub async fn find_similar_skills(&self, task_description: &str, limit: usize) -> Result<Vec<ExtractedSkill>>;
    pub async fn record_usage(&self, skill_id: &str, success: bool) -> Result<()>;
    pub async fn get_top_skills(&self, agent_id: &str, limit: usize) -> Result<Vec<ExtractedSkill>>;
    pub async fn retire_skill(&self, skill_id: &str) -> Result<()>; // 成功率过低时淘汰
}
```

**生命周期**：
- **发现**：dreaming pipeline 中检测重复模式
- **提取**：`SkillExtractor::extract` 从轨迹生成 skill
- **审批**：默认为 `candidate` 状态，成功率 > 阈值后自动提升为 `active`
- **使用**：Agent 收到任务时，先查 `find_similar_skills`，匹配则注入 skill 到 prompt
- **淘汰**：usage_count > 10 且 success_rate < 30% 时标记为 `retired`

#### 3.3.4 Skill 检索与注入

```rust
// 扩展 runtime.rs
impl AgentRuntime {
    async fn inject_relevant_skills(
        &self,
        messages: &mut Vec<ChatMessage>,
        task_description: &str,
    ) {
        if let Some(skill_store) = &self.skill_store {
            let skills = skill_store.find_similar_skills(task_description, 3).await;
            if !skills.is_empty() {
                let skill_prompt = format_skills_for_prompt(&skills);
                // 在 system prompt 后注入 skill 指引
                inject_after_system(messages, &skill_prompt);
            }
        }
    }
}
```

**匹配策略**：
- 优先：`task_type` 完全匹配
- 其次：task_description 关键词交集
- 可选：embedding 语义相似度（复用 memory 的向量能力）

### 3.4 测试计划

- `trajectory_recorded_on_session_end` — 会话结束时轨迹写入
- `skill_extracted_from_repeated_pattern` — 重复模式触发 skill 提取
- `skill_store_find_similar` — 按任务描述检索匹配 skill
- `skill_lifecycle_candidate_to_active` — 成功率达标后自动激活
- `skill_lifecycle_retire_low_success` — 低成功率自动淘汰
- `skill_injected_into_agent_prompt` — 匹配的 skill 注入到 prompt

---

## 四、实施优先级

| 优先级 | 模块 | 预估工作量 | 说明 |
|--------|------|-----------|------|
| **P0** | DAG 节点超时 + 重试策略 | 小 | executor 改造，高 ROI |
| **P0** | 条件表达式引擎 | 中 | mini 解析器 |
| **P0** | 对话轨迹记录 | 中 | 数据采集基础设施 |
| **P1** | 调用图 + 引用索引 | 中 | Tree-sitter 遍历扩展 |
| **P1** | 真实测试执行 + 解析 | 中 | 多语言 runner |
| **P1** | Skill 提取 + 存储 | 大 | 核心 Hermes 能力 |
| **P1** | HITL 审批门控 | 中 | webhook + pending 队列 |
| **P2** | Patch 生成与应用 | 大 | 自动修复闭环 |
| **P2** | 可控循环子图 | 中 | executor 循环支持 |
| **P2** | Skill 检索注入 | 中 | runtime 集成 |

---

## 五、兼容性保证

所有改动遵循：
1. **向后兼容**：新字段使用 `#[serde(default)]`，旧配置不受影响
2. **渐进式**：新能力通过 feature flag 或可选配置启用
3. **无破坏性 API 变更**：扩展现有 trait，不修改已有方法签名
