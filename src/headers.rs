//! Header forwarding helpers.
//!
//! Request headers (method-agnostic: method, auth, cookies, content-type,
//! multi-value headers, …) are forwarded to the upstream, except for the
//! proxy's own control header and hop-by-hop headers. Response headers are
//! forwarded back except hop-by-hop, `content-length` (recomputed for the
//! streamed body) and upstream CORS headers (the proxy sets its own).

use axum::http::{HeaderMap, HeaderName, HeaderValue};

// Hop-by-hop headers that must not be forwarded (RFC 9110 §7.6.1).
const HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

/// Proxy control header carrying the upstream URL.
pub const TARGET_HEADER: &str = "x-target-url";

pub fn is_hop_header(name: &str) -> bool {
    HOP_HEADERS.iter().any(|h| h.eq_ignore_ascii_case(name))
}

/// Upstream CORS headers must never leak through — the proxy sets its own
/// based on `ALLOWED_ORIGINS`.
pub fn is_upstream_cors_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("access-control-allow-origin")
        || name.eq_ignore_ascii_case("access-control-allow-methods")
        || name.eq_ignore_ascii_case("access-control-allow-headers")
        || name.eq_ignore_ascii_case("access-control-allow-credentials")
        || name.eq_ignore_ascii_case("access-control-expose-headers")
        || name.eq_ignore_ascii_case("access-control-max-age")
}

fn parse_upstream_name(name: &str) -> Option<reqwest::header::HeaderName> {
    name.parse().ok()
}

fn convert_value(value: &reqwest::header::HeaderValue) -> Option<HeaderValue> {
    HeaderValue::from_bytes(value.as_bytes()).ok()
}

fn convert_value_back(value: &HeaderValue) -> Option<reqwest::header::HeaderValue> {
    reqwest::header::HeaderValue::from_bytes(value.as_bytes()).ok()
}

/// Build the upstream request headers from the incoming client headers.
///
/// Preserves multi-value headers (via `append`) and raw bytes; drops the
/// `X-Target-URL` control header and hop-by-hop headers.
pub fn build_upstream_headers(
    incoming: &HeaderMap,
) -> reqwest::header::HeaderMap {
    let mut out = reqwest::header::HeaderMap::new();
    for (name, value) in incoming.iter() {
        let n = name.as_str();
        if n.eq_ignore_ascii_case(TARGET_HEADER) || is_hop_header(n) {
            continue;
        }
        let Some(req_name) = parse_upstream_name(n) else {
            continue;
        };
        if let Some(req_value) = convert_value_back(value) {
            out.append(req_name, req_value);
        }
    }
    out
}

/// Build the downstream response headers from the upstream response headers.
///
/// Drops hop-by-hop, `content-length`, and upstream CORS headers; preserves
/// multi-value headers and raw bytes.
pub fn build_response_headers(
    upstream: &reqwest::header::HeaderMap,
) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (name, value) in upstream.iter() {
        let n = name.as_str();
        if is_hop_header(n) || is_upstream_cors_header(n) {
            continue;
        }
        let Ok(ax_name) = n.parse::<HeaderName>() else {
            continue;
        };
        if let Some(ax_value) = convert_value(value) {
            out.append(ax_name, ax_value);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hv(s: &str) -> HeaderValue {
        HeaderValue::from_str(s).unwrap()
    }

    fn rhv(s: &str) -> reqwest::header::HeaderValue {
        reqwest::header::HeaderValue::from_str(s).unwrap()
    }

    #[test]
    fn upstream_forwards_all_except_control_and_hop() {
        let mut incoming = HeaderMap::new();
        incoming.insert("x-target-url", hv("https://example.com"));
        incoming.insert("authorization", hv("Bearer token"));
        incoming.insert("x-custom", hv("1"));
        incoming.append("x-custom", hv("2"));
        incoming.insert("host", hv("proxy"));
        incoming.insert("content-length", hv("5"));
        incoming.insert("connection", hv("keep-alive"));

        let out = build_upstream_headers(&incoming);
        assert!(out.get("x-target-url").is_none());
        assert!(out.get("host").is_none());
        assert!(out.get("content-length").is_none());
        assert!(out.get("connection").is_none());
        assert_eq!(out.get("authorization").unwrap(), "Bearer token");
        let all: Vec<_> = out
            .get_all("x-custom")
            .iter()
            .map(|v| v.to_str().unwrap().to_string())
            .collect();
        assert_eq!(all, vec!["1".to_string(), "2".to_string()]);
    }

    #[test]
    fn response_strips_hop_and_upstream_cors() {
        let mut upstream = reqwest::header::HeaderMap::new();
        upstream.insert("content-type", rhv("application/json"));
        upstream.insert("x-custom", rhv("yes"));
        upstream.insert("transfer-encoding", rhv("chunked"));
        upstream.insert("content-length", rhv("10"));
        upstream.insert("access-control-allow-origin", rhv("*"));
        upstream.insert("access-control-allow-credentials", rhv("true"));

        let out = build_response_headers(&upstream);
        assert_eq!(out.get("content-type").unwrap(), "application/json");
        assert_eq!(out.get("x-custom").unwrap(), "yes");
        assert!(out.get("transfer-encoding").is_none());
        assert!(out.get("content-length").is_none());
        assert!(out.get("access-control-allow-origin").is_none());
        assert!(out.get("access-control-allow-credentials").is_none());
    }
}
