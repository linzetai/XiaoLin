//! Pending eval result registry for WebView JS evaluation callbacks.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dashmap::DashMap;

const EVAL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PENDING_EVALS: usize = 256;

struct PendingEvalEntry {
    created: std::time::Instant,
    tx: std::sync::mpsc::SyncSender<Result<String, String>>,
}

static PENDING_EVALS: OnceLock<Arc<DashMap<String, PendingEvalEntry>>> = OnceLock::new();

fn pending_evals() -> &'static Arc<DashMap<String, PendingEvalEntry>> {
    PENDING_EVALS.get_or_init(|| Arc::new(DashMap::new()))
}

fn evict_oldest_pending_eval(map: &DashMap<String, PendingEvalEntry>) {
    let oldest_key = map
        .iter()
        .min_by_key(|entry| entry.value().created)
        .map(|entry| entry.key().clone());
    if let Some(key) = oldest_key {
        map.remove(&key);
        tracing::warn!(
            eval_id = %key,
            max = MAX_PENDING_EVALS,
            "evicted oldest pending eval at capacity"
        );
    }
}

/// Register a pending eval callback and return its receiver.
pub fn register_eval(id: String) -> Result<std::sync::mpsc::Receiver<Result<String, String>>, String> {
    let map = pending_evals();
    if map.len() >= MAX_PENDING_EVALS {
        evict_oldest_pending_eval(map);
        if map.len() >= MAX_PENDING_EVALS {
            tracing::warn!(
                pending = map.len(),
                max = MAX_PENDING_EVALS,
                "pending eval registry full"
            );
            return Err("too many pending evals".into());
        }
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    map.insert(
        id,
        PendingEvalEntry {
            created: std::time::Instant::now(),
            tx,
        },
    );
    Ok(rx)
}

/// Complete a pending eval with a result or error.
pub fn complete_eval(id: &str, outcome: Result<String, String>) {
    if let Some((_, entry)) = pending_evals().remove(id) {
        let _ = entry.tx.send(outcome);
    }
}

/// Remove a pending eval without delivering a result (e.g. on timeout).
pub fn cancel_eval(id: &str) {
    pending_evals().remove(id);
}

pub fn eval_timeout() -> Duration {
    EVAL_TIMEOUT
}

/// Unwrap JSON.stringify transport: plain strings become inner text; objects stay JSON.
pub fn normalize_eval_result(result_json: &str) -> Result<String, String> {
    match serde_json::from_str::<serde_json::Value>(result_json) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        Ok(v) => serde_json::to_string(&v).map_err(|e| e.to_string()),
        Err(_) => Ok(result_json.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_string_value() {
        assert_eq!(
            normalize_eval_result(r#""hello""#).unwrap(),
            "hello"
        );
    }

    #[test]
    fn normalize_object_value() {
        let out = normalize_eval_result(r#"{"a":1}"#).unwrap();
        assert!(out.contains("\"a\":1"));
    }
}
