use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug)]
pub struct WebError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub field: Option<&'static str>,
}

impl WebError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            field: None,
        }
    }

    pub fn field(mut self, field: &'static str) -> Self {
        self.field = Some(field);
        self
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    field: Option<&'static str>,
    request_id: String,
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                    field: self.field,
                    request_id: request_id(),
                },
            }),
        )
            .into_response()
    }
}

fn request_id() -> String {
    let mut value = [0_u8; 8];
    if getrandom::fill(&mut value).is_err() {
        return "req_unavailable".to_owned();
    }
    let mut request_id = String::with_capacity(20);
    request_id.push_str("req_");
    for byte in value {
        use std::fmt::Write as _;
        let _ = write!(request_id, "{byte:02x}");
    }
    request_id
}
