use crate::grader::GradeResult;
use crate::metrics::RunMetrics;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskReport {
    pub run_id: String,
    pub task_id: String,
    pub suite: String,
    pub pass: bool,
    pub graders: Vec<GradeResult>,
    pub metrics: RunMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub run_id: String,
    pub timestamp: String,
    pub tasks: Vec<TaskReport>,
}

impl RunReport {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            timestamp: chrono::Utc::now().to_rfc3339(),
            tasks: Vec::new(),
        }
    }

    pub fn add_task(&mut self, report: TaskReport) {
        self.tasks.push(report);
    }

    pub fn total(&self) -> usize {
        self.tasks.len()
    }

    pub fn passed(&self) -> usize {
        self.tasks.iter().filter(|t| t.pass).count()
    }

    pub fn failed(&self) -> usize {
        self.total() - self.passed()
    }

    pub fn pass_rate(&self) -> f64 {
        if self.total() == 0 {
            return 0.0;
        }
        self.passed() as f64 / self.total() as f64
    }

    pub fn write_jsonl(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::File::create(path)?;
        for task in &self.tasks {
            let line = serde_json::to_string(task)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    pub fn print_summary(&self) {
        let sep = "=".repeat(60);
        let dash = "-".repeat(60);
        println!("\n{sep}");
        println!("  Benchmark Run: {}", self.run_id);
        println!("  Timestamp: {}", self.timestamp);
        println!("{sep}");
        println!(
            "  Results: {}/{} passed ({:.0}%)",
            self.passed(),
            self.total(),
            self.pass_rate() * 100.0
        );
        println!("{dash}");

        for task in &self.tasks {
            let status = if task.pass { "PASS" } else { "FAIL" };
            let tokens = task
                .metrics
                .token_usage
                .as_ref()
                .map_or(0, |u| u.total_tokens);
            println!(
                "  [{status}] {:<30} turns={:<3} tokens={:<7} {:.1}s",
                task.task_id,
                task.metrics.iterations,
                tokens,
                task.metrics.duration_ms as f64 / 1000.0,
            );

            if !task.pass {
                for g in &task.graders {
                    if !g.pass {
                        println!("         ↳ {}: {}", g.grader_type, g.reason);
                    }
                }
            }
        }

        println!("{dash}");

        let total_tokens: u32 = self
            .tasks
            .iter()
            .filter_map(|t| t.metrics.token_usage.as_ref())
            .map(|u| u.total_tokens)
            .sum();
        let total_prompt_tokens: u32 = self
            .tasks
            .iter()
            .filter_map(|t| t.metrics.token_usage.as_ref())
            .map(|u| u.prompt_tokens)
            .sum();
        let total_completion_tokens: u32 = self
            .tasks
            .iter()
            .filter_map(|t| t.metrics.token_usage.as_ref())
            .map(|u| u.completion_tokens)
            .sum();
        let total_time: u64 = self.tasks.iter().map(|t| t.metrics.duration_ms).sum();
        let total_tools: u32 = self.tasks.iter().map(|t| t.metrics.tool_calls_total).sum();
        let total_tool_fails: u32 = self.tasks.iter().map(|t| t.metrics.tool_calls_failed).sum();
        let avg_tokens_per_iter: f64 = {
            let total_iters: u32 = self.tasks.iter().map(|t| t.metrics.iterations).sum();
            if total_iters > 0 {
                f64::from(total_tokens) / f64::from(total_iters)
            } else {
                0.0
            }
        };

        println!("  Total tokens:      {total_tokens} (prompt: {total_prompt_tokens}, completion: {total_completion_tokens})");
        println!("  Avg tokens/iter:   {avg_tokens_per_iter:.0}");
        println!(
            "  Tool calls:        {total_tools} ({total_tool_fails} failed, {:.0}% error rate)",
            if total_tools > 0 {
                f64::from(total_tool_fails) / f64::from(total_tools) * 100.0
            } else {
                0.0
            }
        );
        println!("  Total time:        {:.1}s", total_time as f64 / 1000.0);
        println!("{sep}");

        self.print_detailed_breakdown();
        println!();
    }

    fn print_detailed_breakdown(&self) {
        let sep = "=".repeat(60);
        let dash = "-".repeat(60);

        println!("\n{sep}");
        println!("  Per-Task Details");
        println!("{sep}");

        for task in &self.tasks {
            let status = if task.pass { "PASS" } else { "FAIL" };
            println!("\n  [{status}] {}", task.task_id);
            println!(
                "  Suite: {}  |  Turns: {}  |  Duration: {:.1}s",
                task.suite,
                task.metrics.iterations,
                task.metrics.duration_ms as f64 / 1000.0
            );

            if let Some(usage) = &task.metrics.token_usage {
                println!(
                    "  Tokens: {} total (prompt: {}, completion: {}, cached: {})",
                    usage.total_tokens,
                    usage.prompt_tokens,
                    usage.completion_tokens,
                    usage.cached_input_tokens
                );
                println!(
                    "  Avg tokens/iter: {:.0}",
                    task.metrics.avg_tokens_per_iteration()
                );
            }

            println!(
                "  Tool calls: {} total ({} ok, {} failed) — error rate: {:.0}%",
                task.metrics.tool_calls_total,
                task.metrics.tool_calls_success,
                task.metrics.tool_calls_failed,
                task.metrics.tool_error_rate() * 100.0
            );

            if !task.metrics.tool_calls_by_name.is_empty() {
                print!("  Tools used: ");
                let mut entries: Vec<_> = task.metrics.tool_calls_by_name.iter().collect();
                entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                for (i, (name, stats)) in entries.iter().enumerate() {
                    if i > 0 {
                        print!(", ");
                    }
                    if stats.failed > 0 {
                        print!("{name} ({}/{}ok)", stats.total, stats.success);
                    } else {
                        print!("{name} ({})", stats.total);
                    }
                }
                println!();
            }

            if !task.metrics.iteration_details.is_empty() {
                println!("  Iteration breakdown:");
                for iter in &task.metrics.iteration_details {
                    let tools_str = if iter.tool_calls.is_empty() {
                        "(no tools)".to_string()
                    } else {
                        iter.tool_calls
                            .iter()
                            .map(|tc| {
                                if tc.success {
                                    tc.tool_name.clone()
                                } else {
                                    format!("{}(FAIL)", tc.tool_name)
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    println!(
                        "    iter {}: cumul_tokens={} ctx={}/{} {}{}",
                        iter.iteration,
                        iter.cumulative_total_tokens,
                        iter.context_used_tokens,
                        iter.context_limit_tokens,
                        tools_str,
                        if iter.context_compressed {
                            " [COMPACT]"
                        } else {
                            ""
                        },
                    );
                }
            }

            if !task.pass {
                println!("  Failed graders:");
                for g in &task.graders {
                    if !g.pass {
                        println!("    - {}: {}", g.grader_type, g.reason);
                    }
                }
            }

            println!("  {dash}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::RunMetrics;

    #[test]
    fn run_report_stats() {
        let mut report = RunReport::new("test-run".into());
        report.add_task(TaskReport {
            run_id: "test-run".into(),
            task_id: "t1".into(),
            suite: "s1".into(),
            pass: true,
            graders: vec![],
            metrics: RunMetrics::default(),
        });
        report.add_task(TaskReport {
            run_id: "test-run".into(),
            task_id: "t2".into(),
            suite: "s1".into(),
            pass: false,
            graders: vec![],
            metrics: RunMetrics::default(),
        });
        assert_eq!(report.total(), 2);
        assert_eq!(report.passed(), 1);
        assert_eq!(report.failed(), 1);
        assert!((report.pass_rate() - 0.5).abs() < f64::EPSILON);
    }
}
