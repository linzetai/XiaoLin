## ADDED Requirements

### Requirement: Token budgets SHALL account for prompt overhead
Benchmark task token budgets SHALL be calibrated based on measured prompt overhead per iteration. The formula SHALL be: `budget = prompt_overhead * expected_turns * safety_margin` where `safety_margin >= 1.2`.

#### Scenario: Simple task budget realistic
- **WHEN** a task requires 4 turns to complete
- **AND** prompt overhead is ~27K tokens per iteration
- **THEN** the token budget SHALL be at least `27000 * 4 * 1.2 = 129,600` tokens

### Requirement: Graders SHALL distinguish capability from efficiency
Tool trace graders SHALL allow shell_exec for build/test operations while forbidding it for file read/write/edit operations. The `forbidden_tools` grader SHALL support context-aware exclusions.

#### Scenario: Shell for cargo build is allowed
- **WHEN** the agent uses `shell_exec` to run `cargo build` or `cargo test`
- **AND** the grader has `forbidden_tools: [shell_exec]` but `allowed_shell_patterns: ["cargo *", "npm *"]`
- **THEN** the grader SHALL pass (shell used for its intended purpose)

#### Scenario: Shell for cat is forbidden
- **WHEN** the agent uses `shell_exec` to run `cat src/main.rs`
- **AND** the grader has `forbidden_tools: [shell_exec]`
- **THEN** the grader SHALL fail (should use read_file)
