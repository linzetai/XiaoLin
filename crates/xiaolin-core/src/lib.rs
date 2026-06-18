pub mod agent_config;
pub mod agent_markdown;
pub mod bus;
pub mod channel;
pub mod channel_plugin;
pub mod complexity;
pub mod config;
pub mod config_access;
pub mod error;
pub mod hardening;
pub mod llm_plugin;
pub mod path;

pub use complexity::ComplexityTier;
pub use error::{XiaoLinError, XiaoLinResult};
pub mod hub;
pub mod paths;
pub mod project;
pub mod project_mcp_approval;
pub mod routing;

pub use routing::Router;
pub mod migration;
pub mod rules;
pub mod skill;
pub mod skill_embedding;
pub mod skill_usage;
pub mod tool;
pub mod tool_runtime;
pub mod types;
pub mod history_compat;
pub mod typed_turn_data;
pub mod workspace;

/// Re-export all protocol types so downstream crates can keep using
/// `xiaolin_core::protocol::*` without adding a direct protocol dependency.
pub use xiaolin_protocol as protocol;
