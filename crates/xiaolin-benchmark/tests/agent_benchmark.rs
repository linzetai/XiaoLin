use std::path::Path;
use xiaolin_benchmark::runner::{BenchmarkRunner, ReplayExecutor};
use xiaolin_benchmark::task::BenchmarkTask;

/// Integration test: loads all benchmark tasks, runs them with ReplayExecutor,
/// and produces a report. Since no replay fixtures exist yet for the agent events,
/// tasks will fail graders that check output/tool traces, but the pipeline
/// itself should execute without errors.
#[tokio::test]
async fn benchmark_pipeline_smoke_test() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let tasks_dir = workspace_root.join("benchmarks/tasks");
    let fixtures_dir = workspace_root.join("benchmarks/fixtures");

    let tasks = BenchmarkTask::load_dir(&tasks_dir).expect("Failed to load tasks");
    assert!(!tasks.is_empty(), "No tasks found");

    let executor = ReplayExecutor::new(&fixtures_dir);
    let runner = BenchmarkRunner::new("smoke-test");
    let report = runner.run(&tasks, &executor, &fixtures_dir).await;

    assert_eq!(report.total(), tasks.len());

    report.print_summary();

    let tmp = tempfile::tempdir().unwrap();
    let report_path = tmp.path().join("results.jsonl");
    report.write_jsonl(&report_path).unwrap();
    assert!(report_path.exists());

    let content = std::fs::read_to_string(&report_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), tasks.len());

    for line in &lines {
        let _: serde_json::Value =
            serde_json::from_str(line).expect("Each line should be valid JSON");
    }
}

/// Verify that the ReplayExecutor works correctly with pre-recorded events.
#[tokio::test]
async fn replay_executor_with_fixture() {
    use xiaolin_protocol::event::AgentEvent;
    use xiaolin_protocol::event::TurnSummary;
    use xiaolin_protocol::usage::TokenUsage;
    use xiaolin_protocol::TurnId;

    let tmp = tempfile::tempdir().unwrap();
    let fixture_events_dir = tmp.path().join("read-file-not-shell");
    std::fs::create_dir_all(&fixture_events_dir).unwrap();

    let events: Vec<AgentEvent> = vec![
        AgentEvent::ToolResult {
            turn_id: TurnId::new("t1"),
            tool_name: "read_file".into(),
            call_id: "c1".into(),
            output: "[server]\nport = 8080".into(),
            display_output: None,
            success: true,
            metadata: None,
        },
        AgentEvent::TurnEnd {
            turn_id: TurnId::new("t1"),
            summary: TurnSummary {
                turn_id: TurnId::new("t1"),
                tool_calls_made: 1,
                iterations: 1,
                elapsed_ms: 2000,
                usage: Some(TokenUsage {
                    prompt_tokens: 5000,
                    completion_tokens: 500,
                    total_tokens: 5500,
                    cached_input_tokens: 0,
                }),
                context_tokens: None,
                context_window: None,
            },
            session_id: None,
            final_tool_calls: None,
            reason: Some("completed".into()),
        },
    ];
    let events_json = serde_json::to_string_pretty(&events).unwrap();
    std::fs::write(fixture_events_dir.join("events.json"), &events_json).unwrap();

    let task = BenchmarkTask {
        id: "read-file-not-shell".into(),
        version: 1,
        suite: "tool-routing".into(),
        tier: xiaolin_benchmark::task::Tier::L1,
        tags: vec![],
        prompt: "Read config.toml".into(),
        graders: vec![
            xiaolin_benchmark::task::GraderConfig::ToolTrace {
                must_include: vec!["read_file".into()],
                must_not_include: vec!["shell_exec".into()],
                allowed_shell_patterns: vec![],
            },
            xiaolin_benchmark::task::GraderConfig::TokenBudget {
                max_total_tokens: 20000,
            },
        ],
        metrics: Default::default(),
        environment: Default::default(),
    };

    let executor = ReplayExecutor::new(tmp.path());
    let runner = BenchmarkRunner::new("fixture-test");
    let report = runner.run(&[task], &executor, tmp.path()).await;

    assert_eq!(report.total(), 1);
    assert_eq!(
        report.passed(),
        1,
        "Task should pass; graders: {:?}",
        report.tasks[0].graders
    );
    assert!(report.tasks[0].pass);
}
