use axum::http::StatusCode;
use axum::response::IntoResponse;

#[derive(Debug)]
pub enum AppError {
    RequestError(reqwest::Error),
    JsonParsingError(serde_json::Error),
    UnexpectedStatusCode(StatusCode),
    InvalidData(String),
    NoData,
    // TODO: PoisonError
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestError(value)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        eprintln!(
            "This code should have never been reached. Error = {:?}",
            self
        );
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}
