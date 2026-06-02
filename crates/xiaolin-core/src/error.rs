//! Structured errors for `xiaolin-core`.
//!
//! `XiaoLinError` implements [`std::error::Error`]. For call sites that still use
//! [`anyhow::Result`], the `?` operator works because [`anyhow::Error`] implements
//! [`From`] for any `E: std::error::Error + Send + Sync + 'static`, which includes
//! [`XiaoLinError`] (backward compatible; no extra `From` impl required).

use std::time::Duration;

use thiserror::Error;

/// Primary error type for `xiaolin-core` public APIs.
#[derive(Debug, Error)]
pub enum XiaoLinError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("agent error: {0}")]
    Agent(String),

    #[error("plugin error: {0}")]
    Plugin(String),

    #[error("memory error: {0}")]
    Memory(String),

    #[error("routing error: {0}")]
    Routing(String),

    #[error("LLM provider error: {0}")]
    LlmProvider(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("JSON5 error: {0}")]
    Json5(String),

    #[error("tool not found: {0}")]
    ToolNotFound(String),

    #[error("tool execution failed: {0}")]
    ToolExecution(String),

    #[error("message bus: agent not registered: {0}")]
    BusAgentNotFound(String),

    #[error("message bus: agent mailbox closed")]
    BusMailboxClosed,

    #[error("message bus: reply channel closed")]
    BusReplyClosed,

    #[error("message bus: request timed out after {0:?}")]
    BusRequestTimeout(Duration),

    #[error("message bus: HMAC signature missing or invalid")]
    BusInvalidSignature,

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl XiaoLinError {
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    pub fn json5(err: impl std::fmt::Display) -> Self {
        Self::Json5(err.to_string())
    }

    pub fn tool_execution(msg: impl Into<String>) -> Self {
        Self::ToolExecution(msg.into())
    }
}

/// Convenient [`Result`] alias for core APIs.
pub type XiaoLinResult<T> = std::result::Result<T, XiaoLinError>;
