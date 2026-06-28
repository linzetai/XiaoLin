use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use xiaolin_agent::AgentRuntime;
use xiaolin_benchmark::live::LiveExecutor;
use xiaolin_benchmark::runner::BenchmarkRunner;
use xiaolin_benchmark::task::BenchmarkTask;
use xiaolin_core::agent_config::{AgentConfig, AgentModelConfig, BehaviorConfig};
use xiaolin_core::config::{load_config, ConfigMode};
use xiaolin_core::tool::ToolRegistry;
use xiaolin_protocol::AgentId;

#[derive(Parser)]
#[command(name = "xiaolin-baseline", about = "Run XiaoLin agent benchmark suite")]
struct Cli {
    /// Directory containing benchmark task YAML files.
    #[arg(long, default_value = "benchmarks/tasks")]
    tasks_dir: PathBuf,

    /// Directory containing workspace fixtures.
    #[arg(long, default_value = "benchmarks/fixtures")]
    fixtures_dir: PathBuf,

    /// Output JSONL report file path.
    #[arg(long, default_value = "benchmarks/baseline.jsonl")]
    output: PathBuf,

    /// Only run tasks matching this suite name.
    #[arg(long)]
    suite: Option<String>,

    /// Only run a specific task by ID.
    #[arg(long)]
    task_id: Option<String>,

    /// LLM provider name (from credentials config).
    #[arg(long)]
    provider: Option<String>,

    /// Model name to use.
    #[arg(long, default_value = "")]
    model: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "xiaolin_benchmark=info,xiaolin_agent=info".into()),
        )
        .init();

    let cli = Cli::parse();

    let cwd = std::env::current_dir()?;
    let abs_output = if cli.output.is_absolute() {
        cli.output.clone()
    } else {
        cwd.join(&cli.output)
    };
    let abs_tasks_dir = if cli.tasks_dir.is_absolute() {
        cli.tasks_dir.clone()
    } else {
        cwd.join(&cli.tasks_dir)
    };
    let abs_fixtures_dir = if cli.fixtures_dir.is_absolute() {
        cli.fixtures_dir.clone()
    } else {
        cwd.join(&cli.fixtures_dir)
    };

    let config = load_config(&ConfigMode::Production)?;

    let mut tasks = BenchmarkTask::load_dir(&abs_tasks_dir)?;
    if tasks.is_empty() {
        anyhow::bail!("No benchmark tasks found in {}", cli.tasks_dir.display());
    }

    if let Some(suite) = &cli.suite {
        tasks.retain(|t| t.suite == *suite);
    }
    if let Some(id) = &cli.task_id {
        tasks.retain(|t| t.id == *id);
    }

    if tasks.is_empty() {
        anyhow::bail!("No tasks match the specified filter");
    }

    tracing::info!(count = tasks.len(), "Loaded benchmark tasks");

    let provider_name = cli.provider.clone().unwrap_or_else(|| {
        config
            .credentials
            .providers
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "openai".to_string())
    });

    tracing::info!(provider = %provider_name, model = %cli.model, "Using LLM provider");

    let model_config = AgentModelConfig {
        provider: provider_name,
        model: cli.model.clone(),
        ..AgentModelConfig::default()
    };

    let provider = xiaolin_agent::create_provider_chain(&model_config, Some(&config.credentials))?;

    let runtime = Arc::new(AgentRuntime::new(Arc::from(provider)));
    runtime.init_self_arc();

    let registry = Arc::new(ToolRegistry::new());
    xiaolin_agent::builtin_tools::register_builtin_tools(&registry);
    xiaolin_agent::builtin_tools::register_recall_tools(&registry);

    let agent_config = AgentConfig {
        agent_id: AgentId::new("benchmark"),
        name: Some("Benchmark Agent".into()),
        description: None,
        model: model_config,
        system_prompt: None,
        tools: vec![],
        behavior: BehaviorConfig::default(),
        mcp_servers: vec![],
        min_tier: None,
        max_tier: None,
        avatar: None,
        channels: HashMap::new(),
    };

    let executor = LiveExecutor::new(runtime, registry, agent_config, &abs_fixtures_dir);

    let runner = BenchmarkRunner::generate();
    let report = runner.run(&tasks, &executor, &abs_fixtures_dir).await;

    if let Some(parent) = abs_output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    report.write_jsonl(&abs_output)?;
    tracing::info!(path = %abs_output.display(), "Wrote JSONL report");

    report.print_summary();

    Ok(())
}
