use std::path::Path;
use xiaolin_benchmark::task::BenchmarkTask;

#[test]
fn all_benchmark_tasks_parse() {
    let tasks_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("benchmarks/tasks");

    if !tasks_dir.exists() {
        panic!(
            "benchmarks/tasks directory not found at {}",
            tasks_dir.display()
        );
    }

    let tasks = BenchmarkTask::load_dir(&tasks_dir).expect("Failed to load task directory");
    assert!(
        !tasks.is_empty(),
        "No benchmark tasks found in {}",
        tasks_dir.display()
    );

    let actual_ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    assert!(
        actual_ids.contains(&"read-file-not-shell"),
        "Should contain read-file-not-shell task"
    );
    assert!(
        actual_ids.contains(&"analyze-architecture"),
        "Should contain analyze-architecture task"
    );
    assert!(
        actual_ids.contains(&"add-delete-feature"),
        "Should contain add-delete-feature task"
    );
    println!("Parsed {} tasks: {:?}", tasks.len(), actual_ids);

    for task in &tasks {
        assert!(!task.id.is_empty(), "Task has empty id");
        assert!(!task.prompt.is_empty(), "Task {} has empty prompt", task.id);
        assert!(!task.suite.is_empty(), "Task {} has empty suite", task.id);
    }
}
