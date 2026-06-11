use serde::{Deserialize, Serialize};

#[cfg(feature = "ts")]
use ts_rs::TS;

/// Filters applied to FTS search queries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchFilters {
    #[serde(default, alias = "workDir", skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    #[serde(default, alias = "dateFrom", skip_serializing_if = "Option::is_none")]
    pub date_from: Option<String>,
    #[serde(default, alias = "dateTo", skip_serializing_if = "Option::is_none")]
    pub date_to: Option<String>,
}

/// A single FTS search hit with session metadata and highlighted snippet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchResult {
    #[serde(alias = "sessionId")]
    pub session_id: String,
    #[serde(alias = "turnId")]
    pub turn_id: String,
    pub role: String,
    #[serde(default, alias = "messageId", skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(alias = "sessionTitle")]
    pub session_title: String,
    #[serde(default, alias = "workDir", skip_serializing_if = "Option::is_none")]
    pub work_dir: Option<String>,
    pub snippet: String,
    pub timestamp: String,
    pub rank: f64,
}

/// Request payload for `search.query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchQueryRequest {
    pub q: String,
    #[serde(default)]
    pub filters: SearchFilters,
    #[serde(default)]
    pub page: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Response payload for `search.query`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchQueryResponse {
    pub results: Vec<SearchResult>,
    #[serde(alias = "totalEstimate")]
    pub total_estimate: u64,
    pub page: i64,
}

/// Response payload for `search.index_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SearchIndexStatusResponse {
    #[serde(alias = "indexedCount")]
    pub indexed_count: u64,
    #[serde(alias = "totalCount")]
    pub total_count: u64,
    #[serde(alias = "isIndexing")]
    pub is_indexing: bool,
}
