//! Pending eval result registry for WebView JS evaluation callbacks.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dashmap::DashMap;

const EVAL_TIMEOUT: Duration = Duration::from_secs(5);

static PENDING_EVALS: OnceLock<Arc<DashMap<String, std::sync::mpsc::SyncSender<Result<String, String>>>>> =
    OnceLock::new();

fn pending_evals() -> &'static Arc<DashMap<String, std::sync::mpsc::SyncSender<Result<String, String>>>> {
    PENDING_EVALS.get_or_init(|| Arc::new(DashMap::new()))
}

/// Register a pending eval callback and return its receiver.
pub fn register_eval(id: String) -> std::sync::mpsc::Receiver<Result<String, String>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    pending_evals().insert(id, tx);
    rx
}

/// Complete a pending eval with a result or error.
pub fn complete_eval(id: &str, outcome: Result<String, String>) {
    if let Some((_, tx)) = pending_evals().remove(id) {
        let _ = tx.send(outcome);
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
