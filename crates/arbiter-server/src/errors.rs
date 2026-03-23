use arbiter_contracts::{ErrorBody, ErrorResponse};
use axum::http::StatusCode;
use axum::Json;

#[derive(Debug)]
pub(crate) struct ApiFailure {
    status: StatusCode,
    code: String,
    message: String,
    details: Option<serde_json::Value>,
}

impl ApiFailure {
    pub(crate) fn bad_request(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    pub(crate) fn not_found(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    pub(crate) fn conflict(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    pub(crate) fn invalid_transition(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    pub(crate) fn approval_required(code: &str, message: &str) -> Self {
        Self {
            status: StatusCode::LOCKED,
            code: code.to_string(),
            message: message.to_string(),
            details: None,
        }
    }

    pub(crate) fn internal(message: &str) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "internal_error".to_string(),
            message: message.to_string(),
            details: None,
        }
    }
}

pub(crate) type ApiErrorResponse = (StatusCode, Json<ErrorResponse>);

pub(crate) fn into_error(err: ApiFailure) -> ApiErrorResponse {
    (
        err.status,
        Json(ErrorResponse {
            error: ErrorBody {
                code: err.code,
                message: err.message,
                details: err.details,
            },
        }),
    )
}
