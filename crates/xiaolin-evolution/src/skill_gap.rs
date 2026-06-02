//! Skill gap detection — analyzes trajectories to identify recurring failure
//! patterns and missing skills.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::trajectory::{Trajectory, TrajectoryOutcome};

/// A detected gap in the agent's skill repertoire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillGap {
    /// Descriptive label for the gap (e.g. "docker compose debugging").
    pub label: String,
    /// Task type where the gap was observed.
    pub task_type: String,
    /// Tool sequence that consistently fails.
    pub failing_tool_sequence: Vec<String>,
    /// Number of trajectories exhibiting this gap.
    pub occurrence_count: usize,
    /// Average failure index (how far into the trajectory the failure occurs).
    pub avg_failure_depth: f64,
    /// Recommended action to address the gap.
    pub recommendation: GapRecommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapRecommendation {
    /// A new skill should be created to handle this pattern.
    CreateSkill { suggested_name: String },
    /// An existing skill should be improved.
    ImproveSkill { skill_id: String },
    /// The tool itself may have issues (e.g. wrong parameters).
    ToolIssue { tool_name: String, hint: String },
}

/// Complete gap analysis report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapReport {
    pub agent_id: String,
    pub trajectories_analyzed: usize,
    pub failure_count: usize,
    pub gaps: Vec<SkillGap>,
    pub generated_at: String,
}

/// Minimum number of occurrences before a failure pattern becomes a gap.
const MIN_GAP_OCCURRENCES: usize = 2;

/// Analyze trajectories to detect skill gaps.
pub fn detect_gaps(agent_id: &str, trajectories: &[Trajectory]) -> GapReport {
    let failures: Vec<&Trajectory> = trajectories
        .iter()
        .filter(|t| matches!(t.outcome, TrajectoryOutcome::Failure { .. }))
        .collect();

    let failure_count = failures.len();

    // Group failures by task_type + tool sequence fingerprint.
    let mut clusters: HashMap<String, Vec<&Trajectory>> = HashMap::new();
    for t in &failures {
        let key = cluster_key(t);
        clusters.entry(key).or_default().push(t);
    }

    let mut gaps = Vec::new();
    for (key, group) in &clusters {
        if group.len() < MIN_GAP_OCCURRENCES {
            continue;
        }

        let task_type = group
            .first()
            .and_then(|t| t.task_type.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let failing_tools = extract_failing_tool_sequence(group);
        let avg_depth = group
            .iter()
            .map(|t| failure_depth(t) as f64)
            .sum::<f64>()
            / group.len() as f64;

        let recommendation = suggest_recommendation(&failing_tools, &task_type, key);

        gaps.push(SkillGap {
            label: format!("{} failure in {}", failing_tools.join("→"), task_type),
            task_type,
            failing_tool_sequence: failing_tools,
            occurrence_count: group.len(),
            avg_failure_depth: avg_depth,
            recommendation,
        });
    }

    // Sort by occurrence count (most frequent gaps first)
    gaps.sort_by(|a, b| b.occurrence_count.cmp(&a.occurrence_count));

    GapReport {
        agent_id: agent_id.to_string(),
        trajectories_analyzed: trajectories.len(),
        failure_count,
        gaps,
        generated_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn cluster_key(t: &Trajectory) -> String {
    let task = t.task_type.as_deref().unwrap_or("unknown");
    let tools: Vec<&str> = t
        .steps
        .iter()
        .filter_map(|s| s.tool_name.as_deref())
        .collect();
    format!("{task}:{}", tools.join(","))
}

fn extract_failing_tool_sequence(group: &[&Trajectory]) -> Vec<String> {
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    for t in group {
        for step in &t.steps {
            if let Some(ref name) = step.tool_name {
                if step.success == Some(false) {
                    *tool_counts.entry(name.clone()).or_default() += 1;
                }
            }
        }
    }

    let mut tools: Vec<(String, usize)> = tool_counts.into_iter().collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1));
    tools.into_iter().map(|(name, _)| name).take(5).collect()
}

fn failure_depth(t: &Trajectory) -> usize {
    t.steps
        .iter()
        .position(|s| s.success == Some(false))
        .unwrap_or(t.steps.len())
}

fn suggest_recommendation(
    failing_tools: &[String],
    task_type: &str,
    _cluster_key: &str,
) -> GapRecommendation {
    if failing_tools.len() == 1 {
        GapRecommendation::ToolIssue {
            tool_name: failing_tools[0].clone(),
            hint: format!(
                "Tool '{}' consistently fails in {} tasks — check parameters and error handling",
                failing_tools[0], task_type
            ),
        }
    } else {
        GapRecommendation::CreateSkill {
            suggested_name: format!("{}_handler", task_type.replace(' ', "_")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::TrajectoryStep;

    fn make_trajectory(
        task_type: &str,
        steps: Vec<(&str, bool)>,
        outcome: TrajectoryOutcome,
    ) -> Trajectory {
        let steps = steps
            .into_iter()
            .map(|(name, success)| TrajectoryStep {
                role: "assistant".into(),
                action_type: "tool_call".into(),
                tool_name: Some(name.to_string()),
                summary: format!("called {name}"),
                success: Some(success),
            })
            .collect();

        Trajectory {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: "test-agent".into(),
            session_id: "test-session".into(),
            task_type: Some(task_type.into()),
            steps,
            outcome,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn detects_repeated_failure_pattern() {
        let trajs = vec![
            make_trajectory(
                "code_edit",
                vec![("read_file", true), ("shell", false)],
                TrajectoryOutcome::Failure {
                    reason: "shell error".into(),
                },
            ),
            make_trajectory(
                "code_edit",
                vec![("read_file", true), ("shell", false)],
                TrajectoryOutcome::Failure {
                    reason: "shell error".into(),
                },
            ),
            make_trajectory(
                "code_edit",
                vec![("read_file", true), ("write_file", true)],
                TrajectoryOutcome::Success { user_rating: None },
            ),
        ];

        let report = detect_gaps("test-agent", &trajs);
        assert_eq!(report.failure_count, 2);
        assert!(!report.gaps.is_empty());
        assert_eq!(report.gaps[0].occurrence_count, 2);
    }

    #[test]
    fn no_gaps_when_all_success() {
        let trajs = vec![make_trajectory(
            "research",
            vec![("web_search", true)],
            TrajectoryOutcome::Success { user_rating: None },
        )];

        let report = detect_gaps("test-agent", &trajs);
        assert!(report.gaps.is_empty());
    }
}
