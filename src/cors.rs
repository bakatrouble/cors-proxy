use axum::http::{HeaderMap, HeaderValue, header};

use crate::config::AllowedOrigins;

const ALLOW_METHODS: &str = "GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS";
const ALLOW_HEADERS_DEFAULT: &str = "Content-Type, Authorization, X-Target-URL, X-Requested-With";

/// Apply the proxy's own CORS headers to an outgoing header map.
///
/// For `Any`, emits `Access-Control-Allow-Origin: *` (no credentials).
/// For an explicit list, echoes the request `Origin` only when allow-listed
/// (otherwise leaves the map untouched), plus credentials + `Vary: Origin`.
pub fn apply_cors(
    headers: &mut HeaderMap,
    allowed: &AllowedOrigins,
    request_headers: &HeaderMap,
) {
    let Some(origin) = allowed.allow_origin_value(request_headers) else {
        return;
    };
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static(ALLOW_METHODS),
    );

    // Reflect the browser's requested headers when present, else a sane default.
    match request_headers
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| HeaderValue::from_str(s).ok())
    {
        Some(reflected) => {
            headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, reflected);
        }
        None => {
            headers.insert(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                HeaderValue::from_static(ALLOW_HEADERS_DEFAULT),
            );
        }
    }

    if allowed.is_any() {
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("86400"),
        );
    } else {
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
            HeaderValue::from_static("true"),
        );
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AllowedOrigins;

    fn origin_headers(origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_str(origin).unwrap());
        h
    }

    #[test]
    fn wildcard_sets_star_and_no_credentials() {
        let mut out = HeaderMap::new();
        apply_cors(&mut out, &AllowedOrigins::Any, &origin_headers("https://x.com"));
        assert_eq!(out.get("access-control-allow-origin").unwrap(), "*");
        assert!(out.get("access-control-allow-credentials").is_none());
        assert_eq!(out.get("access-control-expose-headers").unwrap(), "*");
    }

    #[test]
    fn list_denies_unlisted_origin() {
        let allowed = AllowedOrigins::List(vec!["https://a.com".to_string()]);
        let mut out = HeaderMap::new();
        apply_cors(&mut out, &allowed, &origin_headers("https://evil.com"));
        assert!(out.get("access-control-allow-origin").is_none());
    }

    #[test]
    fn list_echoes_and_sets_credentials() {
        let allowed = AllowedOrigins::List(vec!["https://a.com".to_string()]);
        let mut out = HeaderMap::new();
        apply_cors(&mut out, &allowed, &origin_headers("https://a.com"));
        assert_eq!(
            out.get("access-control-allow-origin").unwrap(),
            "https://a.com"
        );
        assert_eq!(out.get("access-control-allow-credentials").unwrap(), "true");
        assert_eq!(out.get("vary").unwrap(), "Origin");
    }

    #[test]
    fn reflects_requested_headers() {
        let mut req = origin_headers("https://a.com");
        req.insert(
            "access-control-request-headers",
            HeaderValue::from_static("X-Custom-1, X-Custom-2"),
        );
        let mut out = HeaderMap::new();
        apply_cors(&mut out, &AllowedOrigins::Any, &req);
        assert_eq!(
            out.get("access-control-allow-headers").unwrap(),
            "X-Custom-1, X-Custom-2"
        );
    }
}
