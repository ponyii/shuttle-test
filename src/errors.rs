use axum::http::StatusCode;

#[derive(Debug)]
pub enum AppError {
    RequestError(reqwest::Error),
    JsonParsingError(serde_json::Error),
    UnexpectedStatusCode(StatusCode),
    InvalidData(String),
    // TODO: PoisonError
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self::RequestError(value)
    }
}
