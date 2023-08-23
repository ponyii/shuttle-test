use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::{MutexGuard, PoisonError};

use crate::Shard;

type PoisonedShard<'a> = PoisonError<MutexGuard<'a, Shard>>;

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

impl<'a> From<PoisonedShard<'a>> for AppError {
    fn from(_: PoisonedShard<'a>) -> Self {
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
    PoisonedShard,
    StaleShard,
}

impl<'a> From<PoisonedShard<'a>> for HealthProblem {
    fn from(_: PoisonedShard<'a>) -> Self {
        tracing::error!("A poisoned shard found during health check");
        Self::PoisonedShard
    }
}
