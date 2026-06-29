use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    RateLimited(String),
    #[error("{0}")]
    NotImplemented(String),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub err: String,
    pub code: String,
}

pub type AppResult<T> = Result<T, AppError>;

pub fn redact_sensitive_text(input: &str) -> String {
    redact_query_value(input, "api_key")
}

fn redact_query_value(input: &str, key: &str) -> String {
    let needle = format!("{key}=");
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(index) = rest.find(&needle) {
        out.push_str(&rest[..index]);
        out.push_str(&needle);
        out.push_str("***");
        let value_start = index + needle.len();
        let value = &rest[value_start..];
        let value_end = value
            .char_indices()
            .find_map(|(idx, ch)| {
                matches!(ch, '&' | ' ' | ')' | '"' | '\'' | '\n' | '\r').then_some(idx)
            })
            .unwrap_or(value.len());
        rest = &value[value_end..];
    }
    out.push_str(rest);
    out
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, code) = match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, "bad_request"),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, "not_found"),
            AppError::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            AppError::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            AppError::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, "not_implemented"),
            AppError::Sqlx(_) | AppError::Anyhow(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
            }
        };
        let body = ErrorBody {
            err: redact_sensitive_text(&self.to_string()),
            code: code.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive_text;

    #[test]
    fn redacts_api_key_query_values() {
        let text = "error for url (http://x/emby/System/Info?api_key=secret&Limit=0)";
        assert_eq!(
            redact_sensitive_text(text),
            "error for url (http://x/emby/System/Info?api_key=***&Limit=0)"
        );
    }
}
