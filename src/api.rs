use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::Json;
use axum::response::IntoResponse;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    JsonExtractorRejection(#[from] JsonRejection),
    #[error("Number of people must be between {min} and {max}, not {amount}")]
    InvalidNumberOfPeople{amount: u8, min: u8, max: u8},

    #[error("An unexpected error occurred.")]
    Unexpected,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            ApiError::JsonExtractorRejection(json_rejection) => {
                (json_rejection.status(), json_rejection.body_text())
            }
            ApiError::InvalidNumberOfPeople { .. } => {
                (StatusCode::BAD_REQUEST, format!("{self}"))
            }
            ApiError::Unexpected => {
                (StatusCode::INTERNAL_SERVER_ERROR, format!("{self}"))
            }
        };

        let payload = json!({ "error": message, });

        (status, Json(payload)).into_response()
    }
}