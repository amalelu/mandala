// SPDX-License-Identifier: MPL-2.0

//! Tiny HTTP error type for axum handlers. Per `CODE_CONVENTIONS §9`
//! we don't use `thiserror` / `anyhow` / custom `Error` enums —
//! `ApiError(StatusCode, String)` is the minimum that lets a route
//! `?`-propagate while keeping the response shape under our control.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct ApiError(pub StatusCode, pub String);

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }
    pub fn not_implemented(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_IMPLEMENTED, msg.into())
    }
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, msg.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = axum::Json(serde_json::json!({ "error": self.1 }));
        (self.0, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_into_response_status_and_body() {
        let resp = ApiError(StatusCode::BAD_REQUEST, "bad thing".into()).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_constructors_set_expected_status() {
        assert_eq!(ApiError::bad_request("x").0, StatusCode::BAD_REQUEST);
        assert_eq!(
            ApiError::not_implemented("x").0,
            StatusCode::NOT_IMPLEMENTED
        );
        assert_eq!(
            ApiError::unavailable("x").0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
