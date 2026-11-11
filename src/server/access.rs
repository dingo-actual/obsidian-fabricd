//! Access logging that cannot leak request content.
//!
//! One structured record per request: method, path, status, duration. Never a
//! header, never a query string (a token must not reach a log), never a body. The
//! `Authorization` value is not even read here.

use std::time::Instant;

use axum::extract::Request;
use axum::http::Uri;
use axum::middleware::Next;
use axum::response::Response;

/// Logs one completed request.
pub async fn log(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = log_path(request.uri()).to_owned();
    let started = Instant::now();

    let response = next.run(request).await;

    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        target: "obsidian_fabricd::access",
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        duration_ms,
        "request"
    );

    response
}

/// The path alone. The query string is dropped so a smuggled token cannot be
/// logged; nothing else about the request is recorded.
fn log_path(uri: &Uri) -> &str {
    uri.path()
}

#[cfg(test)]
mod tests {
    use axum::http::Uri;

    use super::log_path;

    #[test]
    fn the_query_string_never_reaches_the_log() {
        let uri: Uri = "http://127.0.0.1:47322/v1/events?session_id=abc&token=secret"
            .parse()
            .expect("a valid URI");

        assert_eq!(log_path(&uri), "/v1/events");
    }

    #[test]
    fn a_plain_path_is_recorded_unchanged() {
        let uri: Uri = "/v1/health".parse().expect("a valid URI");

        assert_eq!(log_path(&uri), "/v1/health");
    }
}
