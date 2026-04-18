//! Request extractors with project-wide error mapping.

use axum::extract::{FromRequest, Json, Request};
use serde::de::DeserializeOwned;

use crate::error::AppError;

/// JSON body extractor: parse failures become [`AppError::BadRequest`] (HTTP 400) with the standard error JSON shape.
pub struct AppJson<T>(pub T);

#[async_trait::async_trait]
impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let Json(inner) = Json::<T>::from_request(req, state)
            .await
            .map_err(AppError::from)?;
        Ok(AppJson(inner))
    }
}
