//! General, tool-free decisions. Input is untrusted and cannot name writes or tools.
use crate::{ApiError, blocking};
use axum::{Json, extract::State};
use std::sync::Arc;
use std::time::Duration;
use writer::Writer;

pub async fn decide(
    State(w): State<Arc<Writer>>,
    Json(input): Json<ai::decisions::Input>,
) -> Result<Json<ai::decisions::Response>, ApiError> {
    blocking(w, move |w| {
        let (result, line) = ai::decisions::call(w.state(), input, Duration::from_secs(30));
        tracing::info!(target: "ai", "{line}");
        result
            .map(Json)
            .map_err(|e| ApiError::bad_request(e.to_string()))
    })
    .await
}
