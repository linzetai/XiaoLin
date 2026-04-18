//! Pattern-based skill extraction from recorded trajectories.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::trajectory::{Trajectory, TrajectoryOutcome};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    Candidate,
    Active,
    Retired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillParam {
    pub name: String,
    pub param_type: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedSkill {
    pub id: String,
    pub name: String,
    pub task_pattern: String,
    pub strategy_template: String,
    pub parameters: Vec<SkillParam>,
    pub source_trajectory_ids: Vec<String>,
    pub success_rate: f64,
    pub usage_count: i64,
    pub status: SkillStatus,
    pub created_at: String,
    /// Monotonic skill version within the `parent_id` lineage (starts at 1).
    #[serde(default = "default_skill_version")]
    pub version: u32,
    /// Previous version id when this row supersedes an older extracted skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

fn default_skill_version() -> u32 {
    1
}

/// Structured LLM output merged into rule-based [`ExtractedSkill`] rows after clustering.
///
/// The host callback returns this shape; identifiers and telemetry from the rule-based pass
/// (for example [`ExtractedSkill::id`] and [`ExtractedSkill::source_trajectory_ids`]) are preserved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LlmExtractedPattern {
    /// Human-readable skill name (may replace the auto-generated rule-based name).
    pub name: String,
    /// Short description of when the skill applies (replaces or refines the task key).
    pub task_pattern: String,
    /// Step-by-step strategy text merged into the stored skill.
    pub strategy_template: String,
    /// Declared parameters for the strategy template.
    pub parameters: Vec<SkillParam>,
}

/// Host-supplied async hook for LLM-assisted skill extraction ([`SkillExtractor::extract_skills_with_llm`]).
///
/// Implementations typically call an LLM with `trajectories_summary` and parse the response into
/// [`LlmExtractedPattern`]. On error, the extractor keeps the rule-based fields for that cluster.
#[async_trait]
pub trait LlmExtractionCallback: Send + Sync {
    /// Given a compact textual summary of clustered trajectories, return a refined pattern.
    async fn extract_pattern(&self, trajectories_summary: &str) -> Result<LlmExtractedPattern>;
}

/// Clusters successful trajectories by task type and tool-sequence similarity, then emits candidate skills.
pub struct SkillExtractor {
    /// Minimum trajectories in a cluster before emitting a skill.
    pub min_occurrences: u32,
    /// LCS-based sequence similarity threshold in `[0, 1]` for grouping trajectories.
    pub similarity_threshold: f64,
}

impl Default for SkillExtractor {
    fn default() -> Self {
        Self {
            min_occurrences: 3,
            similarity_threshold: 0.6,
        }
    }
}

impl SkillExtractor {
    pub fn extract_skills(&self, trajectories: &[Trajectory]) -> Vec<ExtractedSkill> {
        let successes: Vec<&Trajectory> = trajectories
            .iter()
            .filter(|t| matches!(t.outcome, TrajectoryOutcome::Success { .. }))
            .collect();

        let mut by_task: HashMap<String, Vec<&Trajectory>> = HashMap::new();
        for t in successes {
            let key = t
                .task_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            by_task.entry(key).or_default().push(t);
        }

        let mut skills = Vec::new();
        for (task_pattern, group) in by_task {
            skills.extend(self.extract_from_task_group(&task_pattern, &group));
        }
        skills
    }

    fn extract_from_task_group(
        &self,
        task_pattern: &str,
        trajs: &[&Trajectory],
    ) -> Vec<ExtractedSkill> {
        let min_n = (self.min_occurrences as usize).max(1);
        if trajs.len() < min_n {
            return Vec::new();
        }

        let mut used: HashSet<String> = HashSet::new();
        let mut out = Vec::new();

        for seed in trajs {
            if used.contains(&seed.id) {
                continue;
            }
            let seed_seq = tool_sequence(seed);
            let mut cluster: Vec<&Trajectory> = Vec::new();
            for t in trajs {
                if used.contains(&t.id) {
                    continue;
                }
                let seq = tool_sequence(t);
                if sequence_similarity(&seed_seq, &seq) >= self.similarity_threshold {
                    cluster.push(t);
                }
            }

            if cluster.len() < min_n {
                continue;
            }

            for t in &cluster {
                used.insert(t.id.clone());
            }

            let pattern = canonical_pattern(&cluster, &seed_seq);
            let strategy_template = build_strategy_template(&pattern);
            let parameters = infer_parameters(&cluster, &pattern);
            let source_trajectory_ids: Vec<String> =
                cluster.iter().map(|t| t.id.clone()).collect();
            let success_rate = cluster_success_rate(&cluster);
            let name = format!(
                "auto_{}_{}",
                task_pattern.replace(' ', "_"),
                &uuid::Uuid::new_v4().to_string()[..8]
            );

            out.push(ExtractedSkill {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                task_pattern: task_pattern.to_string(),
                strategy_template,
                parameters,
                source_trajectory_ids,
                success_rate,
                usage_count: 0,
                status: SkillStatus::Candidate,
                created_at: chrono::Utc::now().to_rfc3339(),
                version: 1,
                parent_id: None,
            });
        }

        out
    }

    /// Rule-based extraction followed by optional LLM refinement **per extracted cluster**.
    ///
    /// Runs [`Self::extract_skills`] first, then for each resulting skill loads the corresponding
    /// trajectories (via [`ExtractedSkill::source_trajectory_ids`]), builds a text summary, and
    /// calls `llm.extract_pattern`. On success, merges name, task pattern, strategy, and parameters
    /// from [`LlmExtractedPattern`]; on failure, logs a warning and keeps the rule-based output.
    pub async fn extract_skills_with_llm(
        &self,
        trajectories: &[Trajectory],
        llm: &dyn LlmExtractionCallback,
    ) -> Result<Vec<ExtractedSkill>> {
        let mut base = self.extract_skills(trajectories);
        for sk in &mut base {
            let cluster: Vec<&Trajectory> = trajectories
                .iter()
                .filter(|t| sk.source_trajectory_ids.contains(&t.id))
                .collect();
            let summary = summarize_trajectory_cluster(&sk.task_pattern, &cluster);
            match llm.extract_pattern(&summary).await {
                Ok(p) => {
                    sk.name = p.name;
                    sk.task_pattern = p.task_pattern;
                    sk.strategy_template = p.strategy_template;
                    sk.parameters = p.parameters;
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        skill_id = %sk.id,
                        "LLM skill pattern extraction failed; keeping rule-based fields"
                    );
                }
            }
        }
        Ok(base)
    }
}

fn summarize_trajectory_cluster(task_pattern: &str, cluster: &[&Trajectory]) -> String {
    let mut lines = vec![format!("Task pattern key: {task_pattern}"), String::new()];
    lines.push(format!("Cluster size: {}", cluster.len()));
    for t in cluster {
        lines.push(format!("--- trajectory {} ---", t.id));
        lines.push(format!("agent_id={} session_id={}", t.agent_id, t.session_id));
        let seq = tool_sequence(t);
        lines.push(format!("tool_sequence: {}", seq.join(" -> ")));
        let step_summaries: Vec<String> = t
            .steps
            .iter()
            .filter(|s| s.action_type == "tool_result" && !s.summary.trim().is_empty())
            .map(|s| format!("{:?}: {}", s.tool_name, s.summary))
            .take(12)
            .collect();
        if !step_summaries.is_empty() {
            lines.push("notable results:".into());
            lines.extend(step_summaries);
        }
    }
    lines.join("\n")
}

fn cluster_success_rate(cluster: &[&Trajectory]) -> f64 {
    if cluster.is_empty() {
        return 0.0;
    }
    let ok = cluster
        .iter()
        .filter(|t| matches!(t.outcome, TrajectoryOutcome::Success { .. }))
        .count() as f64;
    ok / cluster.len() as f64
}

/// Ordered tool names from tool_call / tool_result steps.
pub fn tool_sequence(traj: &Trajectory) -> Vec<String> {
    let mut seq = Vec::new();
    for step in &traj.steps {
        if step.action_type == "tool_call" || step.action_type == "tool_result" {
            if let Some(ref n) = step.tool_name {
                if !n.is_empty() {
                    seq.push(n.clone());
                }
            }
        }
    }
    dedupe_consecutive(seq)
}

fn dedupe_consecutive(names: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for n in names {
        if out.last().map(|l: &String| l != &n).unwrap_or(true) {
            out.push(n);
        }
    }
    out
}

fn canonical_pattern(cluster: &[&Trajectory], seed_seq: &[String]) -> Vec<String> {
    if !seed_seq.is_empty() {
        return seed_seq.to_vec();
    }
    cluster
        .iter()
        .map(|t| tool_sequence(t))
        .max_by_key(|s| s.len())
        .unwrap_or_default()
}

fn build_strategy_template(pattern: &[String]) -> String {
    if pattern.is_empty() {
        return "Follow the conversation flow; no dominant tool pattern was found.".to_string();
    }
    pattern
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}. Use `{}` to progress the task.", i + 1, t))
        .collect::<Vec<_>>()
        .join(" ")
}

fn infer_parameters(cluster: &[&Trajectory], pattern: &[String]) -> Vec<SkillParam> {
    let mut summaries: Vec<String> = Vec::new();
    for t in cluster {
        for step in &t.steps {
            if step.action_type == "tool_result" {
                let s = step.summary.trim();
                if !s.is_empty() && !summaries.contains(&s.to_string()) {
                    summaries.push(s.to_string());
                }
            }
        }
    }

    let mut params = Vec::new();
    if summaries.len() > 1 {
        params.push(SkillParam {
            name: "result_context".into(),
            param_type: "string".into(),
            description: "Varying tool result summaries observed across successful runs."
                .into(),
            default_value: None,
        });
    }

    if pattern.iter().any(|p| p.to_lowercase().contains("search")) {
        params.push(SkillParam {
            name: "query_focus".into(),
            param_type: "string".into(),
            description: "What to search for or prioritize when using search tools.".into(),
            default_value: None,
        });
    }

    params
}

fn lcs_len(a: &[String], b: &[String]) -> usize {
    let n = a.len();
    let m = b.len();
    let mut prev = vec![0usize; m + 1];
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        for j in 1..=m {
            if a[i - 1] == b[j - 1] {
                cur[j] = prev[j - 1] + 1;
            } else {
                cur[j] = cur[j - 1].max(prev[j]);
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

fn sequence_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let lcs = lcs_len(a, b);
    let denom = a.len().max(b.len()).max(1);
    lcs as f64 / denom as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{TrajectoryOutcome, TrajectoryStep};

    fn mk_traj(id: &str, tools: &[&str], task: Option<&str>) -> Trajectory {
        let steps: Vec<TrajectoryStep> = tools
            .iter()
            .flat_map(|tool| {
                vec![
                    TrajectoryStep {
                        role: "assistant".into(),
                        action_type: "tool_call".into(),
                        tool_name: Some((*tool).to_string()),
                        summary: format!("call {tool}"),
                        success: None,
                    },
                    TrajectoryStep {
                        role: "tool".into(),
                        action_type: "tool_result".into(),
                        tool_name: Some((*tool).to_string()),
                        summary: "ok".into(),
                        success: Some(true),
                    },
                ]
            })
            .collect();

        Trajectory {
            id: id.to_string(),
            agent_id: "a".into(),
            session_id: format!("s-{id}"),
            task_type: task.map(String::from),
            steps,
            outcome: TrajectoryOutcome::Success { user_rating: None },
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn skill_extracted_from_repeated_pattern() {
        let ex = SkillExtractor {
            min_occurrences: 3,
            similarity_threshold: 0.6,
        };
        let trajs = vec![
            mk_traj("1", &["read_file", "web_search"], Some("research")),
            mk_traj("2", &["read_file", "web_search"], Some("research")),
            mk_traj("3", &["read_file", "web_search"], Some("research")),
        ];
        let skills = ex.extract_skills(&trajs);
        assert!(
            !skills.is_empty(),
            "expected at least one extracted skill for repeated pattern"
        );
        let s = &skills[0];
        assert!(s.strategy_template.contains("read_file"));
        assert!(s.strategy_template.contains("web_search"));
        assert_eq!(s.source_trajectory_ids.len(), 3);
        assert_eq!(s.version, 1);
        assert!(s.parent_id.is_none());
    }

    struct MockLlm;

    #[async_trait]
    impl LlmExtractionCallback for MockLlm {
        async fn extract_pattern(&self, _summary: &str) -> Result<LlmExtractedPattern> {
            Ok(LlmExtractedPattern {
                name: "LLM Name".into(),
                task_pattern: "research".into(),
                strategy_template: "Refined: use read_file then web_search.".into(),
                parameters: vec![SkillParam {
                    name: "q".into(),
                    param_type: "string".into(),
                    description: "focus".into(),
                    default_value: None,
                }],
            })
        }
    }

    #[tokio::test]
    async fn extract_skills_with_llm_merges_callback_fields() {
        let ex = SkillExtractor {
            min_occurrences: 3,
            similarity_threshold: 0.6,
        };
        let trajs = vec![
            mk_traj("1", &["read_file", "web_search"], Some("research")),
            mk_traj("2", &["read_file", "web_search"], Some("research")),
            mk_traj("3", &["read_file", "web_search"], Some("research")),
        ];
        let llm = MockLlm;
        let skills = ex.extract_skills_with_llm(&trajs, &llm).await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "LLM Name");
        assert!(skills[0].strategy_template.contains("Refined"));
        assert_eq!(skills[0].parameters.len(), 1);
    }
}
