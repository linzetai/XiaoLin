use crate::metrics::CollectedResult;
use crate::runner::FileSnapshot;
use crate::task::GraderConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradeResult {
    pub grader_type: String,
    pub pass: bool,
    pub reason: String,
}

pub fn evaluate_graders(
    configs: &[GraderConfig],
    result: &CollectedResult,
    workspace_dir: &Path,
    pre_run_files: &HashMap<String, FileSnapshot>,
) -> Vec<GradeResult> {
    configs
        .iter()
        .map(|config| evaluate_one(config, result, workspace_dir, pre_run_files))
        .collect()
}

pub fn all_passed(grades: &[GradeResult]) -> bool {
    grades.iter().all(|g| g.pass)
}

fn evaluate_one(
    config: &GraderConfig,
    result: &CollectedResult,
    workspace_dir: &Path,
    pre_run_files: &HashMap<String, FileSnapshot>,
) -> GradeResult {
    match config {
        GraderConfig::OutputContains { patterns } => {
            grade_output_contains(patterns, &result.assistant_text)
        }
        GraderConfig::OutputNotContains { patterns } => {
            grade_output_not_contains(patterns, &result.assistant_text)
        }
        GraderConfig::ToolTrace {
            must_include,
            must_not_include,
            allowed_shell_patterns,
        } => grade_tool_trace(must_include, must_not_include, allowed_shell_patterns, &result.tool_names_used),
        GraderConfig::TokenBudget { max_total_tokens } => {
            grade_token_budget(*max_total_tokens, &result.metrics)
        }
        GraderConfig::TurnLimit { max_turns } => {
            grade_turn_limit(*max_turns, &result.metrics)
        }
        GraderConfig::FilesystemCheck {
            must_exist,
            must_not_exist,
            unchanged,
            files,
        } => grade_filesystem(must_exist, must_not_exist, unchanged, files, workspace_dir, pre_run_files),
    }
}

fn grade_output_contains(patterns: &[String], text: &str) -> GradeResult {
    for pattern in patterns {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if !re.is_match(text) {
                    return GradeResult {
                        grader_type: "output_contains".into(),
                        pass: false,
                        reason: format!("Pattern '{pattern}' not found in output"),
                    };
                }
            }
            Err(_) => {
                if !text.contains(pattern.as_str()) {
                    return GradeResult {
                        grader_type: "output_contains".into(),
                        pass: false,
                        reason: format!("Text '{pattern}' not found in output"),
                    };
                }
            }
        }
    }
    GradeResult {
        grader_type: "output_contains".into(),
        pass: true,
        reason: "All patterns matched".into(),
    }
}

fn grade_output_not_contains(patterns: &[String], text: &str) -> GradeResult {
    for pattern in patterns {
        match regex::Regex::new(pattern) {
            Ok(re) => {
                if re.is_match(text) {
                    return GradeResult {
                        grader_type: "output_not_contains".into(),
                        pass: false,
                        reason: format!("Pattern '{pattern}' found in output (should not be)"),
                    };
                }
            }
            Err(_) => {
                if text.contains(pattern.as_str()) {
                    return GradeResult {
                        grader_type: "output_not_contains".into(),
                        pass: false,
                        reason: format!("Text '{pattern}' found in output (should not be)"),
                    };
                }
            }
        }
    }
    GradeResult {
        grader_type: "output_not_contains".into(),
        pass: true,
        reason: "No forbidden patterns found".into(),
    }
}

fn grade_tool_trace(
    must_include: &[String],
    must_not_include: &[String],
    allowed_shell_patterns: &[String],
    used_tools: &[String],
) -> GradeResult {
    for required in must_include {
        if !used_tools.iter().any(|t| t == required) {
            return GradeResult {
                grader_type: "tool_trace".into(),
                pass: false,
                reason: format!("Required tool '{required}' was not used"),
            };
        }
    }
    for forbidden in must_not_include {
        if forbidden == "shell_exec" && !allowed_shell_patterns.is_empty() {
            continue;
        }
        if used_tools.iter().any(|t| t == forbidden) {
            return GradeResult {
                grader_type: "tool_trace".into(),
                pass: false,
                reason: format!("Forbidden tool '{forbidden}' was used"),
            };
        }
    }
    GradeResult {
        grader_type: "tool_trace".into(),
        pass: true,
        reason: "Tool trace matches expectations".into(),
    }
}

fn grade_token_budget(max_total: u64, metrics: &crate::metrics::RunMetrics) -> GradeResult {
    let actual = metrics
        .token_usage
        .as_ref()
        .map_or(0, |u| u.total_tokens as u64);
    if actual > max_total {
        GradeResult {
            grader_type: "token_budget".into(),
            pass: false,
            reason: format!("Token usage {actual} exceeds budget {max_total}"),
        }
    } else {
        GradeResult {
            grader_type: "token_budget".into(),
            pass: true,
            reason: format!("Token usage {actual} within budget {max_total}"),
        }
    }
}

fn grade_turn_limit(max_turns: u32, metrics: &crate::metrics::RunMetrics) -> GradeResult {
    if metrics.iterations > max_turns {
        let turns = metrics.iterations;
        GradeResult {
            grader_type: "turn_limit".into(),
            pass: false,
            reason: format!("Turns {turns} exceeds limit {max_turns}"),
        }
    } else {
        let turns = metrics.iterations;
        GradeResult {
            grader_type: "turn_limit".into(),
            pass: true,
            reason: format!("Turns {turns} within limit {max_turns}"),
        }
    }
}

fn grade_filesystem(
    must_exist: &[String],
    must_not_exist: &[String],
    unchanged: &[String],
    files: &[crate::task::FileCheck],
    workspace_dir: &Path,
    pre_run_files: &HashMap<String, FileSnapshot>,
) -> GradeResult {
    for path in must_exist {
        if !workspace_dir.join(path).exists() {
            return GradeResult {
                grader_type: "filesystem_check".into(),
                pass: false,
                reason: format!("Expected file '{path}' does not exist"),
            };
        }
    }
    for path in must_not_exist {
        if workspace_dir.join(path).exists() {
            return GradeResult {
                grader_type: "filesystem_check".into(),
                pass: false,
                reason: format!("File '{path}' should not exist but does"),
            };
        }
    }
    for path in unchanged {
        let file_path = workspace_dir.join(path);
        if !file_path.exists() {
            return GradeResult {
                grader_type: "filesystem_check".into(),
                pass: false,
                reason: format!("Expected unchanged file '{path}' does not exist"),
            };
        }
        let baseline = match pre_run_files.get(path) {
            Some(snapshot) => snapshot,
            None => {
                return GradeResult {
                    grader_type: "filesystem_check".into(),
                    pass: false,
                    reason: format!("No pre-run snapshot for unchanged file '{path}'"),
                };
            }
        };
        let meta = match std::fs::metadata(&file_path) {
            Ok(m) => m,
            Err(e) => {
                return GradeResult {
                    grader_type: "filesystem_check".into(),
                    pass: false,
                    reason: format!("Cannot read metadata for '{path}': {e}"),
                };
            }
        };
        let modified_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if meta.len() != baseline.size || modified_secs != baseline.modified_secs {
            return GradeResult {
                grader_type: "filesystem_check".into(),
                pass: false,
                reason: format!(
                    "File '{path}' changed (size {} -> {}, mtime {} -> {})",
                    baseline.size,
                    meta.len(),
                    baseline.modified_secs,
                    modified_secs
                ),
            };
        }
    }
    for check in files {
        let file_path = workspace_dir.join(&check.path);
        if !file_path.exists() {
            return GradeResult {
                grader_type: "filesystem_check".into(),
                pass: false,
                reason: format!("File '{}' does not exist", check.path),
            };
        }
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(e) => {
                return GradeResult {
                    grader_type: "filesystem_check".into(),
                    pass: false,
                    reason: format!("Cannot read '{}': {e}", check.path),
                };
            }
        };
        for pattern in &check.contains {
            if !content.contains(pattern.as_str()) {
                return GradeResult {
                    grader_type: "filesystem_check".into(),
                    pass: false,
                    reason: format!(
                        "File '{}' does not contain '{pattern}'",
                        check.path
                    ),
                };
            }
        }
        for pattern in &check.not_contains {
            if content.contains(pattern.as_str()) {
                return GradeResult {
                    grader_type: "filesystem_check".into(),
                    pass: false,
                    reason: format!(
                        "File '{}' contains forbidden '{pattern}'",
                        check.path
                    ),
                };
            }
        }
    }
    GradeResult {
        grader_type: "filesystem_check".into(),
        pass: true,
        reason: "Filesystem checks passed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{CollectedResult, RunMetrics};
    use xiaolin_protocol::usage::TokenUsage;

    fn mock_result(
        text: &str,
        tools: &[&str],
        total_tokens: u32,
        iterations: u32,
    ) -> CollectedResult {
        CollectedResult {
            metrics: RunMetrics {
                iterations,
                token_usage: Some(TokenUsage {
                    prompt_tokens: total_tokens / 2,
                    completion_tokens: total_tokens / 2,
                    total_tokens,
                    cached_input_tokens: 0,
                }),
                ..Default::default()
            },
            assistant_text: text.into(),
            tool_names_used: tools.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn output_contains_pass() {
        let result = mock_result("The port is 8080", &[], 100, 1);
        let grade = grade_output_contains(&["8080".into()], &result.assistant_text);
        assert!(grade.pass);
    }

    #[test]
    fn output_contains_fail() {
        let result = mock_result("The port is 3000", &[], 100, 1);
        let grade = grade_output_contains(&["8080".into()], &result.assistant_text);
        assert!(!grade.pass);
    }

    #[test]
    fn tool_trace_pass() {
        let result = mock_result("ok", &["read_file", "edit_file"], 100, 1);
        let grade = grade_tool_trace(
            &["read_file".into()],
            &["shell_exec".into()],
            &[],
            &result.tool_names_used,
        );
        assert!(grade.pass);
    }

    #[test]
    fn tool_trace_fail_missing() {
        let result = mock_result("ok", &["shell_exec"], 100, 1);
        let grade = grade_tool_trace(
            &["read_file".into()],
            &[],
            &[],
            &result.tool_names_used,
        );
        assert!(!grade.pass);
        assert!(grade.reason.contains("read_file"));
    }

    #[test]
    fn tool_trace_fail_forbidden() {
        let result = mock_result("ok", &["read_file", "shell_exec"], 100, 1);
        let grade = grade_tool_trace(
            &[],
            &["shell_exec".into()],
            &[],
            &result.tool_names_used,
        );
        assert!(!grade.pass);
        assert!(grade.reason.contains("shell_exec"));
    }

    #[test]
    fn tool_trace_allowed_shell_patterns_bypass() {
        let result = mock_result("ok", &["read_file", "shell_exec"], 100, 1);
        let grade = grade_tool_trace(
            &[],
            &["shell_exec".into()],
            &["cargo".into()],
            &result.tool_names_used,
        );
        assert!(grade.pass, "shell_exec should be allowed when allowed_shell_patterns is non-empty");
    }

    #[test]
    fn token_budget_pass() {
        let result = mock_result("ok", &[], 5000, 1);
        let grade = grade_token_budget(10000, &result.metrics);
        assert!(grade.pass);
    }

    #[test]
    fn token_budget_fail() {
        let result = mock_result("ok", &[], 15000, 1);
        let grade = grade_token_budget(10000, &result.metrics);
        assert!(!grade.pass);
    }

    #[test]
    fn turn_limit_pass() {
        let result = mock_result("ok", &[], 100, 2);
        let grade = grade_turn_limit(3, &result.metrics);
        assert!(grade.pass);
    }

    #[test]
    fn turn_limit_fail() {
        let result = mock_result("ok", &[], 100, 5);
        let grade = grade_turn_limit(3, &result.metrics);
        assert!(!grade.pass);
    }

    #[test]
    fn multi_grader_all_pass() {
        let result = mock_result("port is 8080", &["read_file"], 5000, 2);
        let configs = vec![
            GraderConfig::OutputContains {
                patterns: vec!["8080".into()],
            },
            GraderConfig::ToolTrace {
                must_include: vec!["read_file".into()],
                must_not_include: vec!["shell_exec".into()],
                allowed_shell_patterns: vec![],
            },
            GraderConfig::TokenBudget {
                max_total_tokens: 10000,
            },
        ];
        let grades = evaluate_graders(&configs, &result, Path::new("/tmp"), &HashMap::new());
        assert!(all_passed(&grades));
    }

    #[test]
    fn multi_grader_one_fails() {
        let result = mock_result("port is 8080", &["shell_exec"], 5000, 2);
        let configs = vec![
            GraderConfig::OutputContains {
                patterns: vec!["8080".into()],
            },
            GraderConfig::ToolTrace {
                must_include: vec![],
                must_not_include: vec!["shell_exec".into()],
                allowed_shell_patterns: vec![],
            },
        ];
        let grades = evaluate_graders(&configs, &result, Path::new("/tmp"), &HashMap::new());
        assert!(!all_passed(&grades));
    }
}
