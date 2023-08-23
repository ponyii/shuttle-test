use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::{MutexGuard, PoisonError};

use crate::Shard;

#[derive(Debug)]
pub enum AppError {
    RequestError(reqwest::Error),
    JsonParsingError(serde_json::Error),
    UnexpectedStatusCode(StatusCode),
    InvalidData(String),
    PoisonedShard,
    NoData,
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestError(value)
    }
}

impl<'a> From<PoisonError<MutexGuard<'a, Shard>>> for AppError {
    fn from(_: PoisonError<MutexGuard<'a, Shard>>) -> Self {
        Self::PoisonedShard
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("This code should have never been reached: {:?}", self);
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

pub enum HealthProblem {
    UnexpectedState,
    // TODO PoisonedShard,
    // TODO StaleShards,
}

impl IntoResponse for HealthProblem {
    fn into_response(self) -> axum::response::Response {
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
