use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fastclaw_core::tool::{Tool, ToolKind, ToolParameterSchema, ToolRegistry, ToolResult};

struct FakeTool {
    name_str: &'static str,
    desc: &'static str,
    hint: &'static str,
    params: Vec<(&'static str, &'static str)>,
}

#[async_trait]
impl Tool for FakeTool {
    fn kind(&self) -> ToolKind {
        ToolKind::Execute
    }
    fn name(&self) -> &str {
        self.name_str
    }
    fn description(&self) -> &str {
        self.desc
    }
    fn parameters_schema(&self) -> ToolParameterSchema {
        let mut props = HashMap::new();
        for (name, desc) in &self.params {
            props.insert(
                name.to_string(),
                serde_json::json!({ "type": "string", "description": desc }),
            );
        }
        ToolParameterSchema {
            schema_type: "object".into(),
            properties: props,
            required: self.params.iter().map(|(n, _)| n.to_string()).collect(),
        }
    }
    fn search_hint(&self) -> &str {
        self.hint
    }
    async fn execute(&self, _: &str) -> ToolResult {
        ToolResult::ok("ok")
    }
}

fn realistic_tools() -> Vec<FakeTool> {
    vec![
        FakeTool {
            name_str: "read_file",
            desc: "Read the contents of a file at the given path. Returns the file content as a string.",
            hint: "fs read cat",
            params: vec![
                ("path", "Absolute path to the file to read"),
                ("offset", "Optional line offset to start reading from"),
                ("limit", "Optional number of lines to read"),
            ],
        },
        FakeTool {
            name_str: "write_file",
            desc: "Write content to a file at the given path. Creates the file if it doesn't exist, overwrites if it does.",
            hint: "fs write create",
            params: vec![
                ("path", "Absolute path to the file to write"),
                ("content", "The content to write to the file"),
            ],
        },
        FakeTool {
            name_str: "edit_file",
            desc: "Edit a file by replacing an exact string match. The old_string must uniquely identify the location to edit.",
            hint: "fs edit replace modify",
            params: vec![
                ("path", "Absolute path to the file to edit"),
                ("old_string", "The exact string to search for and replace"),
                ("new_string", "The string to replace old_string with"),
            ],
        },
        FakeTool {
            name_str: "shell_exec",
            desc: "Execute a shell command and return its stdout, stderr, and exit code. Commands run in a sandboxed environment.",
            hint: "bash terminal command run",
            params: vec![
                ("command", "The shell command to execute"),
                ("working_dir", "Optional working directory for the command"),
                ("timeout_secs", "Optional timeout in seconds"),
            ],
        },
        FakeTool {
            name_str: "web_search",
            desc: "Search the web using a search engine. Returns a list of results with titles, URLs, and snippets.",
            hint: "google search query internet",
            params: vec![
                ("query", "The search query string"),
                ("num_results", "Number of results to return, default 5"),
            ],
        },
        FakeTool {
            name_str: "web_fetch",
            desc: "Fetch the content of a URL and return it as text. Handles HTML, JSON, and plain text responses.",
            hint: "http download curl url",
            params: vec![
                ("url", "The URL to fetch"),
                ("headers", "Optional custom headers as JSON object"),
            ],
        },
        FakeTool {
            name_str: "grep_search",
            desc: "Search file contents using regular expressions. Returns matching lines with file paths and line numbers.",
            hint: "ripgrep rg regex find",
            params: vec![
                ("pattern", "Regular expression pattern to search for"),
                ("path", "Directory or file to search in"),
                ("include", "Optional glob pattern to filter files"),
            ],
        },
        FakeTool {
            name_str: "list_directory",
            desc: "List files and directories at the given path. Returns names, types, and sizes.",
            hint: "ls dir folder browse",
            params: vec![
                ("path", "Directory path to list"),
                ("recursive", "Whether to list recursively"),
            ],
        },
        FakeTool {
            name_str: "code_analysis",
            desc: "Analyze source code structure using tree-sitter. Returns AST nodes, function signatures, and class hierarchies.",
            hint: "ast parse syntax treesitter",
            params: vec![
                ("path", "Path to the source file to analyze"),
                ("language", "Programming language of the file"),
                ("query", "Optional tree-sitter query pattern"),
            ],
        },
        FakeTool {
            name_str: "test_runner",
            desc: "Execute test suites for various languages. Supports cargo test, pytest, npm test, and go test.",
            hint: "unit test integration testing",
            params: vec![
                ("framework", "Test framework: cargo, pytest, npm, go"),
                ("path", "Path to test file or directory"),
                ("filter", "Optional test name filter"),
            ],
        },
        FakeTool {
            name_str: "git_operations",
            desc: "Perform git operations: status, diff, log, commit, branch management, and conflict resolution.",
            hint: "version control vcs commit diff",
            params: vec![
                ("operation", "Git operation: status, diff, log, commit, branch, checkout"),
                ("args", "Additional arguments for the operation"),
            ],
        },
        FakeTool {
            name_str: "docker_manage",
            desc: "Manage Docker containers: build images, run/stop containers, view logs, and manage volumes.",
            hint: "container image devops deploy",
            params: vec![
                ("action", "Docker action: build, run, stop, logs, ps"),
                ("target", "Image name or container ID"),
                ("options", "Additional docker options as JSON"),
            ],
        },
    ]
}

fn estimate_definitions_tokens(defs: &[fastclaw_core::tool::ToolDefinition]) -> usize {
    let json = serde_json::to_string(defs).unwrap();
    json.len() / 4 + 4
}

#[test]
fn deferred_mode_saves_at_least_30_percent_tool_tokens() {
    let tools = realistic_tools();
    let total_tool_count = tools.len();

    let all_eager_registry = Arc::new(ToolRegistry::new());
    for tool in realistic_tools() {
        all_eager_registry.register(Arc::new(tool));
    }
    let all_defs = all_eager_registry.eager_definitions();
    assert_eq!(all_defs.len(), total_tool_count);
    let all_tokens = estimate_definitions_tokens(&all_defs);

    let deferred_registry = Arc::new(ToolRegistry::new());
    let eager_names = ["read_file", "write_file", "edit_file", "shell_exec"];
    for tool in realistic_tools() {
        if eager_names.contains(&tool.name_str) {
            deferred_registry.register(Arc::new(tool));
        } else {
            deferred_registry.register_deferred(Arc::new(tool));
        }
    }
    let eager_defs = deferred_registry.eager_definitions();
    assert_eq!(eager_defs.len(), eager_names.len());
    let deferred_tokens = estimate_definitions_tokens(&eager_defs);

    let savings = 1.0 - (deferred_tokens as f64 / all_tokens as f64);
    assert!(
        savings >= 0.30,
        "Deferred mode should save >= 30% tool description tokens, \
         got {:.1}% (all={all_tokens}, eager_only={deferred_tokens})",
        savings * 100.0
    );

    let deferred_count = deferred_registry.deferred_count();
    assert_eq!(
        deferred_count,
        total_tool_count - eager_names.len(),
        "Deferred count mismatch"
    );
}

#[tokio::test]
async fn tool_search_finds_deferred_tools() {
    let registry = Arc::new(ToolRegistry::new());
    let eager_names = ["read_file", "write_file", "edit_file", "shell_exec"];
    for tool in realistic_tools() {
        if eager_names.contains(&tool.name_str) {
            registry.register(Arc::new(tool));
        } else {
            registry.register_deferred(Arc::new(tool));
        }
    }

    let search_tool = fastclaw_agent::builtin_tools::ToolSearchTool::new(registry.clone());

    let result = search_tool
        .execute(r#"{"query": "docker container"}"#)
        .await;
    assert!(result.success);
    let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(
        v["match_count"].as_u64().unwrap() >= 1,
        "Should find docker_manage via keyword search"
    );

    let result = search_tool
        .execute(r#"{"query": "git version control"}"#)
        .await;
    assert!(result.success);
    let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(
        v["match_count"].as_u64().unwrap() >= 1,
        "Should find git_operations via search hint"
    );
}

#[tokio::test]
async fn activate_deferred_tool_makes_it_eager() {
    let registry = Arc::new(ToolRegistry::new());
    let eager_names = ["read_file", "write_file", "edit_file", "shell_exec"];
    for tool in realistic_tools() {
        if eager_names.contains(&tool.name_str) {
            registry.register(Arc::new(tool));
        } else {
            registry.register_deferred(Arc::new(tool));
        }
    }

    let initial_eager = registry.eager_definitions().len();
    let initial_deferred = registry.deferred_count();

    let search_tool = fastclaw_agent::builtin_tools::ToolSearchTool::new(registry.clone());

    let result = search_tool
        .execute(r#"{"query": "select:web_search"}"#)
        .await;
    assert!(result.success);
    assert!(result.output.contains("activated"));

    assert_eq!(
        registry.eager_definitions().len(),
        initial_eager + 1,
        "Eager definitions should increase by 1"
    );
    assert_eq!(
        registry.deferred_count(),
        initial_deferred - 1,
        "Deferred count should decrease by 1"
    );

    let has_web_search = registry
        .eager_definitions()
        .iter()
        .any(|d| d.function.name == "web_search");
    assert!(
        has_web_search,
        "web_search should now appear in eager definitions"
    );
}

#[tokio::test]
async fn full_workflow_eager_search_activate_use() {
    let registry = Arc::new(ToolRegistry::new());
    for tool in realistic_tools() {
        if tool.name_str == "read_file" || tool.name_str == "shell_exec" {
            registry.register(Arc::new(tool));
        } else {
            registry.register_deferred(Arc::new(tool));
        }
    }

    let eager_before = registry.eager_definitions().len();
    assert_eq!(eager_before, 2, "Only 2 eager tools initially");

    let search_tool = fastclaw_agent::builtin_tools::ToolSearchTool::new(registry.clone());

    let result = search_tool.execute(r#"{"query": "test runner"}"#).await;
    assert!(result.success);
    let v: serde_json::Value = serde_json::from_str(&result.output).unwrap();
    assert!(v["match_count"].as_u64().unwrap() >= 1);

    let result = search_tool
        .execute(r#"{"query": "select:test_runner"}"#)
        .await;
    assert!(result.success);

    assert_eq!(registry.eager_definitions().len(), eager_before + 1);

    let exec_result = registry.execute_named("test_runner", "{}").await;
    assert!(
        exec_result.is_ok(),
        "Should be able to execute activated tool"
    );
}
