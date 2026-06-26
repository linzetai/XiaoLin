use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Matcher for determining which tools/events a hook applies to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookMatcher {
    /// Matches all tools.
    AllTools,
    /// Matches a specific tool by exact name.
    ToolName { name: String },
    /// Matches tools by glob pattern (e.g. "file_*").
    ToolPattern { pattern: String },
}

/// A single hook specification loaded from configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookSpec {
    pub matcher: HookMatcher,
    pub command: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl HookSpec {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

fn default_timeout_secs() -> u64 {
    10
}

/// Hook configuration loaded from `.xiaolin/hooks.json5` or similar.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookConfig {
    #[serde(default)]
    pub pre_tool_use: Vec<HookSpec>,
    #[serde(default)]
    pub post_tool_use: Vec<HookSpec>,
    #[serde(default)]
    pub stop: Vec<HookSpec>,
}

impl HookConfig {
    /// Load hook configuration from a directory.
    /// Looks for `hooks.json5` or `hooks.json` in the given directory.
    pub fn load_from_dir(dir: &Path) -> anyhow::Result<Self> {
        let json5_path = dir.join("hooks.json5");
        let json_path = dir.join("hooks.json");

        let content = if json5_path.exists() {
            std::fs::read_to_string(&json5_path)?
        } else if json_path.exists() {
            std::fs::read_to_string(&json_path)?
        } else {
            return Ok(Self::default());
        };

        let config: HookConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse hook config: {e}"))?;
        Ok(config)
    }

    /// Merge another config into this one. Appends hooks from `other`.
    pub fn merge(&mut self, other: HookConfig) {
        self.pre_tool_use.extend(other.pre_tool_use);
        self.post_tool_use.extend(other.post_tool_use);
        self.stop.extend(other.stop);
    }

    /// Check if any hooks are configured.
    pub fn is_empty(&self) -> bool {
        self.pre_tool_use.is_empty() && self.post_tool_use.is_empty() && self.stop.is_empty()
    }
}

#[cfg(test)]
impl HookMatcher {
    pub fn matches(&self, tool_name: &str) -> bool {
        match self {
            Self::AllTools => true,
            Self::ToolName { name } => name == tool_name,
            Self::ToolPattern { pattern } => super::hook_executor::glob_match(pattern, tool_name),
        }
    }
}

#[cfg(test)]
impl HookConfig {
    /// Total number of hook specs across all event types.
    pub fn total_hooks(&self) -> usize {
        self.pre_tool_use.len() + self.post_tool_use.len() + self.stop.len()
    }

    /// Get all pre-tool-use hooks that match a given tool name.
    pub fn matching_pre_hooks(&self, tool_name: &str) -> Vec<&HookSpec> {
        self.pre_tool_use
            .iter()
            .filter(|h| h.matcher.matches(tool_name))
            .collect()
    }

    /// Get all post-tool-use hooks that match a given tool name.
    pub fn matching_post_hooks(&self, tool_name: &str) -> Vec<&HookSpec> {
        self.post_tool_use
            .iter()
            .filter(|h| h.matcher.matches(tool_name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn sample_config() -> HookConfig {
        HookConfig {
            pre_tool_use: vec![
                HookSpec {
                    matcher: HookMatcher::ToolName {
                        name: "shell_exec".into(),
                    },
                    command: "echo 'pre-shell'".into(),
                    timeout_secs: 5,
                    blocking: true,
                    working_dir: None,
                },
                HookSpec {
                    matcher: HookMatcher::ToolPattern {
                        pattern: "file_*".into(),
                    },
                    command: "echo 'pre-file'".into(),
                    timeout_secs: 10,
                    blocking: false,
                    working_dir: None,
                },
            ],
            post_tool_use: vec![HookSpec {
                matcher: HookMatcher::AllTools,
                command: "echo 'post'".into(),
                timeout_secs: 10,
                blocking: false,
                working_dir: None,
            }],
            stop: vec![],
        }
    }

    #[test]
    fn load_from_dir_reads_json() {
        let dir = tempfile::tempdir().unwrap();
        let config = sample_config();
        let json = serde_json::to_string_pretty(&config).unwrap();

        let mut f = std::fs::File::create(dir.path().join("hooks.json")).unwrap();
        f.write_all(json.as_bytes()).unwrap();

        let loaded = HookConfig::load_from_dir(dir.path()).unwrap();
        assert_eq!(loaded.pre_tool_use.len(), 2);
        assert_eq!(loaded.post_tool_use.len(), 1);
        assert!(loaded.stop.is_empty());
    }

    #[test]
    fn load_from_dir_returns_default_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = HookConfig::load_from_dir(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn merge_appends_hooks() {
        let mut config = sample_config();
        let other = HookConfig {
            pre_tool_use: vec![HookSpec {
                matcher: HookMatcher::AllTools,
                command: "echo extra".into(),
                timeout_secs: 3,
                blocking: true,
                working_dir: None,
            }],
            post_tool_use: vec![],
            stop: vec![HookSpec {
                matcher: HookMatcher::AllTools,
                command: "echo stop".into(),
                timeout_secs: 10,
                blocking: false,
                working_dir: None,
            }],
        };

        config.merge(other);
        assert_eq!(config.pre_tool_use.len(), 3);
        assert_eq!(config.stop.len(), 1);
        assert_eq!(config.total_hooks(), 5);
    }

    #[test]
    fn matching_pre_hooks_filters_correctly() {
        let config = sample_config();

        let matches = config.matching_pre_hooks("shell_exec");
        assert_eq!(matches.len(), 1);
        assert!(matches[0].blocking);

        let matches = config.matching_pre_hooks("file_read");
        assert_eq!(matches.len(), 1);
        assert!(!matches[0].blocking);

        let matches = config.matching_pre_hooks("unknown_tool");
        assert!(matches.is_empty());
    }

    #[test]
    fn hook_matcher_glob_works() {
        let matcher = HookMatcher::ToolPattern {
            pattern: "file_*".into(),
        };
        assert!(matcher.matches("file_read"));
        assert!(matcher.matches("file_write"));
        assert!(!matcher.matches("shell_exec"));

        let matcher = HookMatcher::AllTools;
        assert!(matcher.matches("anything"));
    }
}
