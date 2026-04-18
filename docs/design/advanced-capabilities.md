# Harness Engineer — 高级能力模块详细设计

> **版本**: v1.0  
> **日期**: 2026-04-16  
> **状态**: Draft  
> **作者**: FastClaw Team  
> **关联**: 本文档为 `harness-engineer-technical-design.md` 第 10 节的附录

---

## 1. 快速创建 Agent（fastclaw-agent-factory）

### 1.1 核心结构

```rust
pub struct AgentFactory {
    template_registry: TemplateRegistry,
    intent_extractor: IntentExtractor,
    llm_provider: Arc<dyn LlmProvider>,
    config_writer: ConfigWriter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBlueprint {
    pub id: String,
    pub display_name: String,
    pub system_prompt: String,
    pub model_tier: ModelTier,
    pub tools: Vec<String>,
    pub dangerous_tools: Vec<String>,
    pub memory_policy: MemoryPolicy,
    pub channel_bindings: Vec<ChannelBinding>,
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct TemplateRegistry {
    templates: HashMap<String, AgentTemplate>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub category: TemplateCategory,
    pub description: String,
    pub parameters: Vec<TemplateParam>,
    pub base_config: AgentBlueprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TemplateCategory {
    CodeReview,
    Documentation,
    DevOps,
    CustomerSupport,
    DataAnalysis,
    Security,
    Testing,
    General,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateParam {
    pub name: String,
    pub param_type: ParamType,
    pub required: bool,
    pub default: Option<Value>,
    pub description: String,
}
```

### 1.2 对话式创建流程

```rust
impl AgentFactory {
    /// 从自然语言描述创建 Agent（最多 5 轮对话）
    pub async fn create_from_conversation(
        &self,
        initial_description: &str,
    ) -> Result<ConversationResult> {
        let mut context = ConversationContext::new();
        let mut round = 0;

        // 第 1 轮：意图提取
        let intent = self.intent_extractor.extract(initial_description).await?;
        context.set_intent(intent.clone());

        // 匹配最佳模板
        let candidates = self.template_registry.match_templates(&intent);

        if candidates.is_empty() {
            // 无匹配模板，从空白开始
            context.set_mode(CreateMode::FreeForm);
        } else {
            context.set_mode(CreateMode::TemplateBased(candidates[0].id.clone()));
        }

        loop {
            round += 1;
            if round > 5 {
                return Err(FactoryError::MaxRoundsExceeded.into());
            }

            let missing = context.find_missing_fields();
            if missing.is_empty() {
                break;
            }

            // 生成追问
            let question = self.generate_question(&context, &missing).await?;
            let answer = context.wait_for_user_input(question).await?;
            context.apply_answer(&missing, &answer)?;
        }

        // 生成 Blueprint
        let blueprint = context.build_blueprint()?;

        // 生成 JSON 配置
        let json_content = self.config_writer.render_json(&blueprint)?;

        Ok(ConversationResult { blueprint, json_content })
    }
}
```

### 1.3 意图提取器

```rust
pub struct IntentExtractor {
    keyword_rules: Vec<KeywordRule>,
}

#[derive(Debug)]
pub struct ExtractedIntent {
    pub role: String,
    pub tools_needed: Vec<String>,
    pub permission_level: PermissionLevel,
    pub communication_style: CommunicationStyle,
    pub domain: String,
    pub confidence: f64,
}

impl IntentExtractor {
    /// 纯规则提取（无 LLM 调用），< 1ms
    pub fn extract_sync(&self, description: &str) -> ExtractedIntent {
        let lower = description.to_lowercase();
        let mut intent = ExtractedIntent::default();

        for rule in &self.keyword_rules {
            if rule.matches(&lower) {
                intent.merge(&rule.intent_fragment);
            }
        }

        intent
    }

    /// LLM 辅助提取（高精度，用于模糊描述）
    pub async fn extract(&self, description: &str) -> Result<ExtractedIntent> {
        let rule_result = self.extract_sync(description);
        if rule_result.confidence > 0.8 {
            return Ok(rule_result);
        }
        // 低置信度时回退到 LLM 辅助
        todo!("LLM-assisted intent extraction")
    }
}
```

---

## 2. Skill 可视化编排 — Harness Studio（fastclaw-studio）

### 2.1 FlowDSL 定义

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDefinition {
    pub id: String,
    pub name: String,
    pub version: u32,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub variables: HashMap<String, FlowVariable>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: String,
    pub kind: FlowNodeKind,
    pub position: Position,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FlowNodeKind {
    // 触发类
    ManualTrigger,
    CronTrigger { schedule: String },
    WebhookTrigger { path: String },

    // AI 类
    LlmCall { agent_id: String, prompt_template: String },
    EmbeddingCall { model: String },

    // 工具类
    ToolExec { tool_name: String, params: Value },
    HttpRequest { method: String, url: String },
    CodeExec { language: String, code: String },

    // 流程控制
    Condition { expression: String },
    Switch { variable: String, cases: Vec<(String, String)> },
    Loop { max_iterations: u32 },
    Parallel { branches: Vec<String> },
    Join { strategy: MergeStrategy },
    Delay { duration_ms: u64 },

    // 记忆类
    MemoryRecall { query_template: String, top_k: usize },
    MemoryStore { memory_type: MemoryType },

    // Agent 类
    AgentDelegate { agent_id: String },
    HumanApproval { prompt: String, timeout_secs: u64 },

    // 输出类
    MessageOutput { channel: String },
    VariableSet { name: String, expression: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    pub condition: Option<String>,
    pub label: Option<String>,
}
```

### 2.2 FlowDSL → DAG 转换

```rust
pub struct FlowCompiler;

impl FlowCompiler {
    /// FlowDSL → DagGraph（技术方案 2.5 节定义的 DAG 结构）
    pub fn compile(&self, flow: &FlowDefinition) -> Result<DagGraph> {
        self.validate(flow)?;

        let mut dag = DagGraph::new();
        let mut node_map = HashMap::new();

        for flow_node in &flow.nodes {
            let dag_node = self.convert_node(flow_node)?;
            let idx = dag.add_node(dag_node);
            node_map.insert(&flow_node.id, idx);
        }

        for edge in &flow.edges {
            let from = node_map.get(edge.from.as_str())
                .ok_or(FlowError::NodeNotFound(edge.from.clone()))?;
            let to = node_map.get(edge.to.as_str())
                .ok_or(FlowError::NodeNotFound(edge.to.clone()))?;

            dag.add_edge(*from, *to, DagEdge {
                condition: edge.condition.clone(),
            });
        }

        // 验证 DAG（无环、有唯一入口）
        if petgraph::algo::is_cyclic_directed(&dag) {
            return Err(FlowError::CycleDetected.into());
        }

        Ok(dag)
    }

    fn validate(&self, flow: &FlowDefinition) -> Result<()> {
        if flow.nodes.is_empty() {
            return Err(FlowError::EmptyFlow.into());
        }
        // 检查节点 ID 唯一性
        let ids: HashSet<_> = flow.nodes.iter().map(|n| &n.id).collect();
        if ids.len() != flow.nodes.len() {
            return Err(FlowError::DuplicateNodeId.into());
        }
        Ok(())
    }
}
```

### 2.3 Studio WebSocket 调试协议

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StudioMessage {
    /// 客户端 → 服务端：保存 Flow
    SaveFlow { flow: FlowDefinition },
    /// 客户端 → 服务端：开始调试执行
    StartDebug { flow_id: String, input: Value },
    /// 客户端 → 服务端：断点设置
    SetBreakpoint { node_id: String },
    /// 客户端 → 服务端：继续执行（从断点恢复）
    Resume { run_id: String },

    /// 服务端 → 客户端：节点状态变更
    NodeStateChange {
        run_id: String,
        node_id: String,
        state: NodeState,
        output: Option<Value>,
        elapsed_ms: u64,
    },
    /// 服务端 → 客户端：断点命中
    BreakpointHit {
        run_id: String,
        node_id: String,
        context: Value,
    },
    /// 服务端 → 客户端：执行完成
    RunComplete {
        run_id: String,
        result: DagResult,
        total_elapsed_ms: u64,
    },
    /// 服务端 → 客户端：AI 节点建议
    AiSuggestion {
        suggestions: Vec<NodeSuggestion>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeSuggestion {
    pub node_kind: FlowNodeKind,
    pub connect_from: Option<String>,
    pub reason: String,
    pub confidence: f64,
}
```

---

## 3. 多 Agent 协作（fastclaw-collab）

### 3.1 CollabHub 核心

```rust
pub struct CollabHub {
    message_bus: InternalMessageBus,
    registry: AgentCapabilityRegistry,
    thread_manager: ThreadManager,
    signer: MessageSigner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilityCard {
    pub agent_id: AgentId,
    pub skills: Vec<SkillTag>,
    pub max_concurrent: usize,
    pub avg_response_ms: u64,
    pub cost_tier: ModelTier,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillTag {
    CodeGeneration,
    CodeReview,
    TextSummary,
    DataAnalysis,
    Translation,
    Search,
    Debugging,
    Testing,
    Documentation,
    Custom(String),
}

pub struct AgentCapabilityRegistry {
    cards: DashMap<AgentId, AgentCapabilityCard>,
}

impl AgentCapabilityRegistry {
    pub fn find_best_match(
        &self,
        required_skills: &[SkillTag],
        exclude: &[AgentId],
    ) -> Option<AgentId> {
        self.cards
            .iter()
            .filter(|entry| !exclude.contains(entry.key()))
            .filter(|entry| entry.value().status == AgentStatus::Ready)
            .filter(|entry| {
                required_skills.iter().all(|s| entry.value().skills.contains(s))
            })
            .min_by_key(|entry| {
                (entry.value().cost_tier as u8, entry.value().avg_response_ms)
            })
            .map(|entry| entry.key().clone())
    }
}
```

### 3.2 消息总线

```rust
pub struct InternalMessageBus {
    channels: DashMap<AgentId, mpsc::Sender<AgentMessage>>,
    signer: MessageSigner,
    max_ttl: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: String,
    pub thread_id: String,
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
    pub msg_type: AgentMessageType,
    pub ttl: u8,
    pub signature: String,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentMessageType {
    TaskRequest { task: Value },
    TaskResponse { result: Value, success: bool },
    StatusUpdate { status: String },
    ErrorReport { error: String },
}

impl InternalMessageBus {
    pub async fn send(&self, mut msg: AgentMessage) -> Result<()> {
        if msg.ttl == 0 {
            tracing::warn!(msg_id = %msg.id, "TTL exhausted, dropping message");
            return Err(CollabError::TtlExhausted.into());
        }
        msg.ttl -= 1;
        msg.signature = self.signer.sign(&msg);

        let sender = self.channels.get(&msg.to)
            .ok_or(CollabError::AgentNotRegistered(msg.to.clone()))?;

        sender.send(msg).await
            .map_err(|_| CollabError::AgentUnreachable(msg.to.clone()))?;

        Ok(())
    }

    pub fn register(&self, agent_id: AgentId) -> mpsc::Receiver<AgentMessage> {
        let (tx, rx) = mpsc::channel(256);
        self.channels.insert(agent_id, tx);
        rx
    }
}
```

### 3.3 协作模式实现

```rust
/// 主从模式：Orchestrator 分解任务，分配给 Worker
pub struct OrchestratorMode {
    hub: Arc<CollabHub>,
    orchestrator_id: AgentId,
    llm_provider: Arc<dyn LlmProvider>,
}

impl OrchestratorMode {
    pub async fn execute(&self, task: &str) -> Result<CollabResult> {
        // 1. Orchestrator 分解任务
        let subtasks = self.decompose_task(task).await?;

        // 2. 为每个子任务匹配最佳 Worker
        let assignments: Vec<(AgentId, Value)> = subtasks.iter()
            .map(|st| {
                let worker = self.hub.registry.find_best_match(
                    &st.required_skills, &[self.orchestrator_id.clone()],
                ).ok_or(CollabError::NoSuitableWorker)?;
                Ok((worker, st.payload.clone()))
            })
            .collect::<Result<Vec<_>>>()?;

        // 3. 并行分发
        let thread_id = Uuid::new_v4().to_string();
        let handles: Vec<_> = assignments.into_iter()
            .map(|(worker_id, payload)| {
                let hub = self.hub.clone();
                let tid = thread_id.clone();
                let oid = self.orchestrator_id.clone();
                tokio::spawn(async move {
                    hub.send_and_wait(oid, worker_id, payload, &tid).await
                })
            })
            .collect();

        // 4. 收集结果
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await??);
        }

        // 5. Orchestrator 汇总
        let summary = self.summarize_results(&results).await?;
        Ok(CollabResult { thread_id, summary, sub_results: results })
    }
}

/// 流水线模式：A → B → C 顺序传递
pub struct PipelineMode {
    hub: Arc<CollabHub>,
    stages: Vec<AgentId>,
}

impl PipelineMode {
    pub async fn execute(&self, input: Value) -> Result<Value> {
        let thread_id = Uuid::new_v4().to_string();
        let mut current = input;

        for (i, agent_id) in self.stages.iter().enumerate() {
            let prev = if i > 0 { Some(self.stages[i - 1].clone()) } else { None };
            current = self.hub.send_and_wait(
                prev.unwrap_or_else(|| AgentId::from("system")),
                agent_id.clone(),
                current,
                &thread_id,
            ).await?;
        }

        Ok(current)
    }
}

/// 辩证模式：多个 Agent 独立分析 → Arbitrator 综合
pub struct DebateMode {
    hub: Arc<CollabHub>,
    debaters: Vec<AgentId>,
    arbitrator: AgentId,
    max_rounds: u32,
}

impl DebateMode {
    pub async fn execute(&self, question: &str) -> Result<Value> {
        let thread_id = Uuid::new_v4().to_string();

        // 并行获取各方观点
        let opinions: Vec<_> = futures::future::join_all(
            self.debaters.iter().map(|agent_id| {
                self.hub.send_and_wait(
                    self.arbitrator.clone(),
                    agent_id.clone(),
                    json!({ "question": question }),
                    &thread_id,
                )
            })
        ).await.into_iter().collect::<Result<Vec<_>>>()?;

        // Arbitrator 综合判断
        let verdict = self.hub.send_and_wait(
            AgentId::from("system"),
            self.arbitrator.clone(),
            json!({ "question": question, "opinions": opinions }),
            &thread_id,
        ).await?;

        Ok(verdict)
    }
}
```

---

## 4. 模型复杂度路由（fastclaw-model-router）

### 4.1 复杂度评估器

```rust
pub struct ComplexityEstimator {
    keyword_weights: HashMap<String, f64>,
}

#[derive(Debug, Clone)]
pub struct ComplexityScore {
    pub raw_score: f64,
    pub tier: ModelTier,
    pub dimensions: ComplexityDimensions,
    pub urgency_override: Option<ModelTier>,
}

#[derive(Debug, Clone)]
pub struct ComplexityDimensions {
    pub task_type_score: f64,
    pub context_length_score: f64,
    pub tool_count_score: f64,
    pub failure_history_score: f64,
    pub user_intent_score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModelTier {
    Tiny = 0,
    Small = 1,
    Medium = 2,
    Large = 3,
    Frontier = 4,
}

impl ComplexityEstimator {
    /// 纯本地计算，< 1ms，不调用 LLM
    pub fn estimate(&self, request: &ChatRequest, session: &Session) -> ComplexityScore {
        let mut dims = ComplexityDimensions::default();

        // 任务类型评分（关键词匹配）
        dims.task_type_score = self.score_task_type(&request.last_message());

        // 上下文长度评分
        let ctx_tokens = session.total_tokens();
        dims.context_length_score = match ctx_tokens {
            0..=500 => 0.1,
            501..=2000 => 0.3,
            2001..=8000 => 0.5,
            8001..=32000 => 0.7,
            _ => 0.9,
        };

        // 工具数量评分
        let tool_count = request.available_tools.len();
        dims.tool_count_score = (tool_count as f64 / 20.0).min(1.0);

        // 历史失败率
        dims.failure_history_score = session.recent_failure_rate();

        // 用户显式意图（"简单说"、"详细分析"等）
        let urgency = self.detect_user_intent(&request.last_message());

        let raw_score = dims.task_type_score * 0.4
            + dims.context_length_score * 0.2
            + dims.tool_count_score * 0.15
            + dims.failure_history_score * 0.15
            + dims.user_intent_score * 0.1;

        let tier = match raw_score {
            s if s < 0.2 => ModelTier::Tiny,
            s if s < 0.4 => ModelTier::Small,
            s if s < 0.6 => ModelTier::Medium,
            s if s < 0.8 => ModelTier::Large,
            _ => ModelTier::Frontier,
        };

        ComplexityScore {
            raw_score,
            tier,
            dimensions: dims,
            urgency_override: urgency,
        }
    }

    fn score_task_type(&self, message: &str) -> f64 {
        let lower = message.to_lowercase();
        let mut score = 0.3; // 基线

        // 高复杂度关键词
        let complex_keywords = ["设计", "架构", "实现", "重构", "优化", "分布式",
            "并发", "implement", "design", "architect", "refactor"];
        for kw in &complex_keywords {
            if lower.contains(kw) { score += 0.15; }
        }

        // 低复杂度关键词
        let simple_keywords = ["什么是", "解释", "hello", "你好", "简单说",
            "help", "what is", "explain"];
        for kw in &simple_keywords {
            if lower.contains(kw) { score -= 0.1; }
        }

        score.clamp(0.0, 1.0)
    }

    fn detect_user_intent(&self, message: &str) -> Option<ModelTier> {
        let lower = message.to_lowercase();
        if lower.contains("简单") || lower.contains("简短") || lower.contains("brief") {
            return Some(ModelTier::Small);
        }
        if lower.contains("详细") || lower.contains("深入") || lower.contains("comprehensive") {
            return Some(ModelTier::Large);
        }
        None
    }
}
```

### 4.2 预算跟踪器

```rust
pub struct BudgetTracker {
    daily_usage: DashMap<AgentId, f64>,
    last_reset: AtomicU64,
}

impl BudgetTracker {
    pub fn check_budget(
        &self,
        agent_id: &AgentId,
        policy: &BudgetPolicy,
        requested_tier: ModelTier,
    ) -> ModelTier {
        self.maybe_reset_daily();

        let used = self.daily_usage
            .get(agent_id)
            .map(|v| *v)
            .unwrap_or(0.0);

        let remaining = policy.daily_limit_usd - used;
        let utilization = used / policy.daily_limit_usd;

        if utilization > 0.9 {
            return ModelTier::Tiny.max(policy.min_tier);
        }
        if utilization > 0.7 && requested_tier > ModelTier::Small {
            return ModelTier::Small.max(policy.min_tier);
        }

        requested_tier.clamp(policy.min_tier, policy.max_tier)
    }

    pub fn record_usage(&self, agent_id: &AgentId, cost_usd: f64) {
        self.daily_usage
            .entry(agent_id.clone())
            .and_modify(|v| *v += cost_usd)
            .or_insert(cost_usd);
    }
}

#[derive(Debug, Clone)]
pub struct BudgetPolicy {
    pub daily_limit_usd: f64,
    pub min_tier: ModelTier,
    pub max_tier: ModelTier,
}
```

### 4.3 ModelRouter 集成

```rust
pub struct ModelRouter {
    estimator: ComplexityEstimator,
    budget_tracker: BudgetTracker,
    providers: HashMap<ModelTier, Vec<Arc<dyn LlmProvider>>>,
}

impl ModelRouter {
    pub async fn select_provider(
        &self,
        request: &ChatRequest,
        session: &Session,
        agent_config: &AgentConfig,
    ) -> Result<Arc<dyn LlmProvider>> {
        let score = self.estimator.estimate(request, session);

        let effective_tier = if let Some(override_tier) = score.urgency_override {
            override_tier
        } else {
            self.budget_tracker.check_budget(
                &agent_config.id,
                &agent_config.budget_policy,
                score.tier,
            )
        };

        self.providers
            .get(&effective_tier)
            .and_then(|providers| providers.first())
            .cloned()
            .ok_or(RouterError::NoProviderForTier(effective_tier).into())
    }
}
```

---

## 5. Agent 自我迭代（fastclaw-self-iter）

### 5.1 迭代引擎

```rust
pub struct SelfIterationEngine {
    diagnoser: ErrorDiagnoser,
    strategist: RepairStrategist,
    sandbox: SandboxRunner,
    memory: Arc<MemorySystem>,
    max_iterations: u32,
}

impl SelfIterationEngine {
    /// 触发条件检查
    pub fn should_iterate(&self, context: &IterationContext) -> bool {
        context.consecutive_failures >= 2
            || context.user_negative_feedback
            || context.tool_error_detected
            || context.logic_contradiction_detected
    }

    /// 执行自我迭代闭环
    pub async fn iterate(&self, context: IterationContext) -> Result<IterationResult> {
        for round in 0..self.max_iterations {
            // 1. 诊断
            let diagnosis = self.diagnoser.diagnose(&context).await?;

            // 2. 从记忆中检索相似失败案例
            let similar_cases = self.memory.recall(
                &diagnosis.error_description,
                RecallOptions { top_k: 5, memory_type: Some(MemoryType::Episodic) },
            ).await?;

            // 3. 生成候选策略
            let strategies = self.strategist.generate(
                &diagnosis, &similar_cases, 3,
            ).await?;

            // 4. 沙箱并行验证
            let results = self.sandbox.verify_parallel(&strategies, &context).await?;

            // 5. 选择最优策略
            if let Some(best) = results.iter().find(|r| r.success) {
                // 固化到情景记忆
                self.memory.memorize(MemoryEntry {
                    content: format!(
                        "Error: {}\nFix: {}\nStrategy: {}",
                        diagnosis.error_description,
                        best.strategy.description,
                        best.strategy.kind,
                    ),
                    memory_type: MemoryType::Episodic,
                    ..Default::default()
                }).await?;

                return Ok(IterationResult {
                    success: true,
                    rounds: round + 1,
                    applied_strategy: Some(best.strategy.clone()),
                    diagnosis,
                });
            }
        }

        // 升级：交给人工或更强模型
        Ok(IterationResult {
            success: false,
            rounds: self.max_iterations,
            applied_strategy: None,
            diagnosis: self.diagnoser.diagnose(&context).await?,
        })
    }
}
```

### 5.2 沙箱验证器

```rust
pub struct SandboxRunner {
    session_store: Arc<SessionStore>,
}

impl SandboxRunner {
    /// 在隔离 Session 中验证策略，不污染真实数据
    pub async fn verify_parallel(
        &self,
        strategies: &[RepairStrategy],
        context: &IterationContext,
    ) -> Result<Vec<VerifyResult>> {
        let handles: Vec<_> = strategies.iter()
            .map(|strategy| {
                let store = self.session_store.clone();
                let ctx = context.clone();
                let strat = strategy.clone();
                tokio::spawn(async move {
                    // 创建沙箱会话（带 sandbox: 前缀，不会与真实会话冲突）
                    let sandbox_key = format!("sandbox:{}:{}", ctx.session_key, Uuid::new_v4());
                    let result = Self::run_in_sandbox(&store, &sandbox_key, &strat, &ctx).await;
                    // 清理沙箱数据
                    let _ = store.delete_session(&sandbox_key).await;
                    VerifyResult { strategy: strat, success: result.is_ok(), detail: result }
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await?);
        }
        Ok(results)
    }
}
```

---

## 6. 智能上下文管理（fastclaw-context）

### 6.1 六层上下文模型

```rust
pub struct ContextManager {
    user_profile_store: UserProfileStore,
    memory_system: Arc<MemorySystem>,
    compressor: ContextCompressor,
}

impl ContextManager {
    /// 构建 6 层上下文，< 10ms（不含向量检索）
    pub async fn build_context(
        &self,
        agent_config: &AgentConfig,
        session: &Session,
        user_message: &str,
        total_budget: usize,
    ) -> Result<Vec<ChatMessage>> {
        let mut messages = Vec::with_capacity(64);
        let mut remaining_tokens = total_budget;

        // Layer 1: System Prompt（固定，高优先级）
        let system_tokens = count_tokens(&agent_config.system_prompt);
        messages.push(ChatMessage::system(&agent_config.system_prompt));
        remaining_tokens -= system_tokens;

        // Layer 2: 用户画像（跨会话持久化）
        let profile = self.user_profile_store
            .load(&session.peer_id).await?;
        if let Some(profile) = profile {
            let profile_text = profile.to_context_string();
            let profile_tokens = count_tokens(&profile_text);
            if profile_tokens < remaining_tokens / 6 {
                messages.push(ChatMessage::system(profile_text));
                remaining_tokens -= profile_tokens;
            }
        }

        // Layer 3: 会话摘要（长会话压缩后的精华）
        if session.message_count() > 20 {
            let summary = self.compressor.summarize(session).await?;
            let summary_tokens = count_tokens(&summary);
            if summary_tokens < remaining_tokens / 4 {
                messages.push(ChatMessage::system(format!("[会话摘要] {summary}")));
                remaining_tokens -= summary_tokens;
            }
        }

        // Layer 4: 向量记忆召回
        let recalled = self.memory_system.recall(
            user_message,
            RecallOptions { top_k: 5, ..Default::default() },
        ).await?;
        let memory_text = format_memories(&recalled);
        let memory_tokens = count_tokens(&memory_text);
        if memory_tokens < remaining_tokens / 4 && !recalled.is_empty() {
            messages.push(ChatMessage::system(memory_text));
            remaining_tokens -= memory_tokens;
        }

        // Layer 5: 最近对话历史（剩余空间的 80%）
        let history_budget = (remaining_tokens as f64 * 0.8) as usize;
        let recent = session.recent_messages_within_budget(history_budget);
        messages.extend(recent);
        remaining_tokens -= recent.iter().map(|m| m.token_count).sum::<usize>();

        // Layer 6: 当前用户输入
        messages.push(ChatMessage::user(user_message));

        Ok(messages)
    }
}
```

### 6.2 用户画像

```rust
pub struct UserProfileStore {
    store: Arc<SessionStore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub peer_id: String,
    pub tech_stack: Vec<String>,
    pub expertise_level: ExpertiseLevel,
    pub communication_style: CommunicationStyle,
    pub preferred_language: String,
    pub common_tools: Vec<String>,
    pub last_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExpertiseLevel { Beginner, Intermediate, Advanced, Expert }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommunicationStyle { Concise, Detailed, Tutorial, Technical }

impl UserProfile {
    /// 纯规则提取，无 LLM 调用
    pub fn extract_from_messages(messages: &[ChatMessage]) -> Self {
        let mut profile = Self::default();

        for msg in messages.iter().filter(|m| m.role == "user") {
            let content = &msg.content;
            // 技术栈检测
            for lang in ["rust", "python", "typescript", "go", "java", "c++"] {
                if content.to_lowercase().contains(lang) {
                    if !profile.tech_stack.contains(&lang.to_string()) {
                        profile.tech_stack.push(lang.to_string());
                    }
                }
            }

            // 沟通风格推断
            let avg_length = messages.iter()
                .filter(|m| m.role == "user")
                .map(|m| m.content.len())
                .sum::<usize>() / messages.len().max(1);

            profile.communication_style = if avg_length < 50 {
                CommunicationStyle::Concise
            } else if avg_length < 200 {
                CommunicationStyle::Technical
            } else {
                CommunicationStyle::Detailed
            };
        }

        profile
    }
}
```

### 6.3 会话压缩器

```rust
pub struct ContextCompressor {
    summary_provider: Arc<dyn LlmProvider>,
}

impl ContextCompressor {
    /// 滚动压缩：当历史超过阈值时，用小模型压缩
    pub async fn compress_if_needed(
        &self,
        session: &mut Session,
        threshold_tokens: usize,
    ) -> Result<bool> {
        if session.total_tokens() < threshold_tokens {
            return Ok(false);
        }

        // 取前 60% 的消息进行压缩
        let split_point = (session.messages.len() as f64 * 0.6) as usize;
        let to_compress = &session.messages[..split_point];

        let summary = self.summary_provider.chat(ChatRequest {
            messages: vec![
                ChatMessage::system("Summarize the following conversation, preserving key facts, decisions, and context."),
                ChatMessage::user(format_messages_for_summary(to_compress)),
            ],
            ..Default::default()
        }).await?;

        // 替换压缩区间
        let compressed_msg = ChatMessage::system(
            format!("[Compressed history] {}", summary.content)
        );
        session.messages = std::iter::once(compressed_msg)
            .chain(session.messages[split_point..].iter().cloned())
            .collect();

        Ok(true)
    }
}
```

---

## 7. 代码能力增强（fastclaw-code）

### 7.1 代码库索引

```rust
pub struct CodebaseIndex {
    ast_parser: AstParser,
    vector_index: Arc<VectorIndex>,
    call_graph: CallGraph,
    file_index: DashMap<PathBuf, FileMetadata>,
}

pub struct AstParser {
    parsers: HashMap<String, tree_sitter::Parser>,
}

#[derive(Debug)]
pub struct FileMetadata {
    pub path: PathBuf,
    pub language: String,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
    pub hash: u64,
    pub last_indexed: u64,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: (usize, usize),
    pub signature: String,
}

#[derive(Debug, Clone)]
pub enum SymbolKind {
    Function, Method, Class, Struct, Enum, Interface, Module, Variable, Constant,
}

impl CodebaseIndex {
    /// 增量索引：只处理变更的文件，> 1000 文件/分钟
    pub async fn index_directory(&self, root: &Path) -> Result<IndexStats> {
        let mut stats = IndexStats::default();

        let files = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| self.is_supported_file(e.path()));

        for entry in files {
            let path = entry.path().to_path_buf();
            let content = tokio::fs::read_to_string(&path).await?;
            let hash = fxhash::hash64(content.as_bytes());

            // 跳过未变更的文件
            if let Some(existing) = self.file_index.get(&path) {
                if existing.hash == hash {
                    stats.skipped += 1;
                    continue;
                }
            }

            // AST 解析
            let language = self.detect_language(&path);
            let symbols = self.ast_parser.parse(&content, &language)?;

            // 向量索引（按函数/类分 chunk）
            for symbol in &symbols {
                let chunk = &content[symbol.range.0..symbol.range.1];
                self.vector_index.insert(
                    fxhash::hash64(chunk.as_bytes()),
                    chunk,
                ).await?;
            }

            // 调用图更新
            self.call_graph.update(&path, &symbols, &content)?;

            self.file_index.insert(path, FileMetadata {
                path: entry.path().to_path_buf(),
                language,
                symbols,
                imports: vec![],
                hash,
                last_indexed: unix_millis_now(),
            });

            stats.indexed += 1;
        }

        Ok(stats)
    }

    /// 语义搜索
    pub async fn semantic_search(
        &self,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>> {
        let vector_results = self.vector_index.search(query, top_k * 2).await?;

        let mut results: Vec<SearchResult> = vector_results.into_iter()
            .filter_map(|(id, score)| {
                self.find_symbol_by_vector_id(id)
                    .map(|sym| SearchResult { symbol: sym, score })
            })
            .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        results.truncate(top_k);

        Ok(results)
    }
}
```

### 7.2 代码执行沙箱

```rust
pub struct CodeExecutionSandbox {
    temp_root: PathBuf,
    default_timeout: Duration,
    default_memory_limit_mb: usize,
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub code: String,
    pub language: CodeLanguage,
    pub timeout: Duration,
    pub memory_limit_mb: usize,
    pub network_access: bool,
    pub stdin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub success: bool,
    pub elapsed_ms: u64,
    pub memory_peak_mb: usize,
}

#[derive(Debug, Clone)]
pub enum CodeLanguage { Python, Rust, Go, JavaScript, Shell }

impl CodeExecutionSandbox {
    pub async fn execute(&self, req: ExecutionRequest) -> Result<ExecutionResult> {
        let work_dir = self.create_work_dir()?;
        let script_path = self.write_script(&work_dir, &req)?;

        let (cmd, args) = match req.language {
            CodeLanguage::Python => ("python3", vec![script_path.to_str().unwrap().to_string()]),
            CodeLanguage::Rust => {
                self.compile_rust(&work_dir, &script_path).await?;
                (work_dir.join("output").to_str().unwrap().to_string(), vec![])
            }
            CodeLanguage::Go => ("go", vec!["run".into(), script_path.to_str().unwrap().to_string()]),
            CodeLanguage::JavaScript => ("node", vec![script_path.to_str().unwrap().to_string()]),
            CodeLanguage::Shell => ("bash", vec![script_path.to_str().unwrap().to_string()]),
        };

        let start = std::time::Instant::now();

        let output = tokio::time::timeout(req.timeout, async {
            tokio::process::Command::new(&cmd)
                .args(&args)
                .env_clear()
                .env("HOME", &work_dir)
                .env("PATH", "/usr/local/bin:/usr/bin:/bin")
                .current_dir(&work_dir)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .output()
                .await
        }).await;

        // 清理工作目录
        let _ = tokio::fs::remove_dir_all(&work_dir).await;

        match output {
            Ok(Ok(output)) => Ok(ExecutionResult {
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                exit_code: output.status.code().unwrap_or(-1),
                success: output.status.success(),
                elapsed_ms: start.elapsed().as_millis() as u64,
                memory_peak_mb: 0,
            }),
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(ExecutionResult {
                stdout: String::new(),
                stderr: "Execution timed out".into(),
                exit_code: -1,
                success: false,
                elapsed_ms: req.timeout.as_millis() as u64,
                memory_peak_mb: 0,
            }),
        }
    }
}
```

### 7.3 错误自动修复

```rust
pub struct AutoFixEngine {
    sandbox: CodeExecutionSandbox,
    llm_provider: Arc<dyn LlmProvider>,
    max_rounds: u32,
}

impl AutoFixEngine {
    /// 执行 → 分析 → 修复 → 验证循环，最多 5 轮
    pub async fn fix_loop(
        &self,
        code: &str,
        language: CodeLanguage,
        test_command: Option<&str>,
    ) -> Result<AutoFixResult> {
        let mut current_code = code.to_string();
        let mut history = Vec::new();

        for round in 0..self.max_rounds {
            // 执行
            let result = self.sandbox.execute(ExecutionRequest {
                code: current_code.clone(),
                language: language.clone(),
                timeout: Duration::from_secs(30),
                ..Default::default()
            }).await?;

            if result.success {
                return Ok(AutoFixResult {
                    success: true,
                    rounds: round + 1,
                    final_code: current_code,
                    changes: history,
                });
            }

            // 分析错误 + 生成修复
            let fix_response = self.llm_provider.chat(ChatRequest {
                messages: vec![
                    ChatMessage::system("Fix the code error. Return only the corrected code."),
                    ChatMessage::user(format!(
                        "Code:\n```\n{}\n```\n\nError:\n```\n{}\n```",
                        current_code, result.stderr
                    )),
                ],
                ..Default::default()
            }).await?;

            let new_code = extract_code_from_response(&fix_response.content);
            history.push(FixChange {
                round,
                error: result.stderr.clone(),
                diff: generate_diff(&current_code, &new_code),
            });
            current_code = new_code;
        }

        Ok(AutoFixResult {
            success: false,
            rounds: self.max_rounds,
            final_code: current_code,
            changes: history,
        })
    }
}
```
