//! HTTP-layer errors with stable JSON bodies and correct status codes.

use axum::{
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Unauthorized(String),
    NotFound(String),
    RateLimited,
    Internal(anyhow::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::BadRequest(m) | AppError::Unauthorized(m) | AppError::NotFound(m) => {
                write!(f, "{m}")
            }
            AppError::RateLimited => write!(f, "rate limited"),
            AppError::Internal(e) => write!(f, "{e}"),
        }
    }
}

impl AppError {
    pub fn error_type(&self) -> &'static str {
        match self {
            AppError::BadRequest(_) => "bad_request",
            AppError::Unauthorized(_) => "unauthorized",
            AppError::NotFound(_) => "not_found",
            AppError::RateLimited => "rate_limited",
            AppError::Internal(_) => "server_error",
        }
    }

    fn message(&self) -> String {
        match self {
            AppError::BadRequest(m) | AppError::Unauthorized(m) | AppError::NotFound(m) => {
                m.clone()
            }
            AppError::RateLimited => "rate limited".to_string(),
            AppError::Internal(e) => e.to_string(),
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// JSON payload for WebSocket `error` frames (includes HTTP-like `status`).
    pub fn to_ws_error_value(&self) -> serde_json::Value {
        json!({
            "status": self.status().as_u16(),
            "message": self.message(),
            "type": self.error_type(),
        })
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        let err_type = self.error_type();
        let message = self.message();
        match &self {
            AppError::NotFound(m) => tracing::warn!(%m, "not found"),
            AppError::BadRequest(m) => tracing::warn!(%m, "bad request"),
            AppError::Unauthorized(m) => tracing::warn!(%m, "unauthorized"),
            AppError::RateLimited => tracing::warn!("rate limited"),
            AppError::Internal(e) => tracing::error!(error = %e, "request failed"),
        }
        let body = json!({
            "error": {
                "message": message,
                "type": err_type,
            }
        });
        (status, Json(body)).into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Internal(err)
    }
}

impl From<JsonRejection> for AppError {
    fn from(rejection: JsonRejection) -> Self {
        AppError::BadRequest(rejection.body_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    async fn response_json(err: AppError) -> serde_json::Value {
        let res = err.into_response();
        let body = res
            .into_body()
            .collect()
            .await
            .expect("body collect")
            .to_bytes();
        serde_json::from_slice(&body).expect("json body")
    }

    #[tokio::test]
    async fn app_error_status_codes() {
        let cases: Vec<(AppError, u16)> = vec![
            (AppError::BadRequest("bad".into()), 400),
            (AppError::Unauthorized("no".into()), 401),
            (AppError::NotFound("missing".into()), 404),
            (AppError::RateLimited, 429),
            (AppError::Internal(anyhow::anyhow!("fail")), 500),
        ];
        for (err, expected) in cases {
            let status = err.into_response().status();
            assert_eq!(status.as_u16(), expected);
        }
    }

    #[tokio::test]
    async fn app_error_json_shape() {
        let body = response_json(AppError::BadRequest("invalid input".into())).await;
        let err = body
            .get("error")
            .and_then(|v| v.as_object())
            .expect("error object");
        assert!(err.get("message").is_some());
        assert!(err.get("type").is_some());
        assert_eq!(
            err.get("message").and_then(|v| v.as_str()),
            Some("invalid input")
        );
        assert_eq!(
            err.get("type").and_then(|v| v.as_str()),
            Some("bad_request")
        );
    }

    #[test]
    fn app_error_ws_error_value_contains_type() {
        let cases: Vec<(AppError, &'static str, u16)> = vec![
            (AppError::BadRequest("x".into()), "bad_request", 400),
            (AppError::Unauthorized("x".into()), "unauthorized", 401),
            (AppError::NotFound("x".into()), "not_found", 404),
            (AppError::RateLimited, "rate_limited", 429),
            (
                AppError::Internal(anyhow::anyhow!("e")),
                "server_error",
                500,
            ),
        ];
        for (err, ty, status) in cases {
            let v = err.to_ws_error_value();
            assert_eq!(v.get("type").and_then(|x| x.as_str()), Some(ty));
            assert_eq!(
                v.get("status").and_then(|x| x.as_u64()),
                Some(u64::from(status))
            );
            assert!(v.get("message").is_some());
        }
    }
}
