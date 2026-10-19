use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use futures_util::{StreamExt, TryStreamExt};
use tracing::warn;

use crate::{
    config::AllowedOrigins,
    cors::apply_cors,
    headers::{TARGET_HEADER, build_response_headers, build_upstream_headers},
};

#[derive(Clone)]
pub struct AppState {
    pub client: reqwest::Client,
    pub allowed_origins: AllowedOrigins,
}

impl AppState {
    pub fn new(client: reqwest::Client, allowed_origins: AllowedOrigins) -> Self {
        Self {
            client,
            allowed_origins,
        }
    }

    pub fn default_client() -> reqwest::Client {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("failed to build http client")
    }
}

pub async fn health() -> &'static str {
    "ok"
}

fn cors_error(state: &AppState, req_headers: &HeaderMap, status: StatusCode, msg: &str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    apply_cors(&mut headers, &state.allowed_origins, req_headers);
    (status, headers, msg.to_string()).into_response()
}

/// Resolve the upstream URL: `X-Target-URL` plus the incoming path+query
/// (unless the proxy path is `/`), so both usages work:
/// `X-Target-URL: https://api.example.com/items + GET /` and
/// `X-Target-URL: https://api.example.com + GET /items?page=2`.
pub fn resolve_target(target_raw: &str, uri: &Uri) -> Result<reqwest::Url, &'static str> {
    let target_raw = target_raw.trim();
    let mut target_str = target_raw.to_string();
    if let Some(path_q) = uri.path_and_query() {
        let pq = path_q.as_str();
        if pq != "/" && pq != "/*" {
            let base_ends_slash = target_str.ends_with('/');
            if base_ends_slash && pq.starts_with('/') {
                target_str.pop();
                target_str.push_str(pq);
            } else if !base_ends_slash && !pq.starts_with('/') {
                target_str.push('/');
                target_str.push_str(pq);
            } else {
                target_str.push_str(pq);
            }
        }
    }

    let mut url: reqwest::Url = target_str.parse().map_err(|_| "Invalid X-Target-URL")?;
    // Appending the proxy path may have produced a non-http(s) URL;
    // fall back to the raw header value alone in that case.
    if url.scheme() != "http" && url.scheme() != "https" {
        url = target_raw.parse().map_err(|_| "Invalid X-Target-URL")?;
    }
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("X-Target-URL must be http(s)");
    }
    Ok(url)
}

pub async fn proxy_handler(State(state): State<AppState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let req_headers = parts.headers.clone();
    let method = parts.method.clone();

    // Preflight: answer directly, no upstream call.
    if method == Method::OPTIONS {
        let mut headers = HeaderMap::new();
        apply_cors(&mut headers, &state.allowed_origins, &req_headers);
        return (StatusCode::NO_CONTENT, headers, Body::empty()).into_response();
    }

    let target_raw = match req_headers.get(TARGET_HEADER) {
        Some(v) => match v.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => {
                return cors_error(
                    &state,
                    &req_headers,
                    StatusCode::BAD_REQUEST,
                    "Invalid X-Target-URL encoding",
                )
            }
        },
        None => {
            return cors_error(
                &state,
                &req_headers,
                StatusCode::BAD_REQUEST,
                "Missing X-Target-URL header",
            )
        }
    };

    let target_url = match resolve_target(&target_raw, &parts.uri) {
        Ok(u) => u,
        Err(msg) => return cors_error(&state, &req_headers, StatusCode::BAD_REQUEST, msg),
    };

    let upstream_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            return cors_error(
                &state,
                &req_headers,
                StatusCode::BAD_REQUEST,
                "Unsupported method",
            )
        }
    };

    // Forward all eligible request headers (auth, cookies, content-type,
    // multi-value headers, …) — see headers::build_upstream_headers.
    let mut upstream = state
        .client
        .request(upstream_method, target_url)
        .headers(build_upstream_headers(&req_headers));

    // Stream the request body upstream (no buffering/size limit).
    let has_body = !matches!(method, Method::GET | Method::HEAD)
        || req_headers.contains_key(header::CONTENT_LENGTH)
        || req_headers.contains_key(header::TRANSFER_ENCODING);
    if has_body {
        let req_stream = TryStreamExt::map_err(body.into_data_stream(), std::io::Error::other);
        upstream = upstream.body(reqwest::Body::wrap_stream(req_stream));
    }

    let upstream_resp = match upstream.send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("upstream error: {e}");
            return cors_error(
                &state,
                &req_headers,
                StatusCode::BAD_GATEWAY,
                "Upstream fetch failed",
            );
        }
    };

    let status =
        StatusCode::from_u16(upstream_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out_headers = build_response_headers(upstream_resp.headers());
    apply_cors(&mut out_headers, &state.allowed_origins, &req_headers);

    // Stream the response body back (redirects already followed by the client).
    let stream = upstream_resp
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));
    let out_body = Body::from_stream(stream);

    (status, out_headers, out_body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_target_full_url_ignores_root_path() {
        let uri: Uri = "/".parse().unwrap();
        let url = resolve_target("https://api.example.com/items?page=2", &uri).unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/items?page=2");
    }

    #[test]
    fn resolve_target_appends_path() {
        let uri: Uri = "/items?page=2".parse().unwrap();
        let url = resolve_target("https://api.example.com", &uri).unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/items?page=2");
    }

    #[test]
    fn resolve_target_avoids_double_slash() {
        let uri: Uri = "/items".parse().unwrap();
        let url = resolve_target("https://api.example.com/", &uri).unwrap();
        assert_eq!(url.as_str(), "https://api.example.com/items");
    }

    #[test]
    fn resolve_target_rejects_non_http() {
        let uri: Uri = "/".parse().unwrap();
        assert!(resolve_target("ftp://example.com/x", &uri).is_err());
        assert!(resolve_target("not a url", &uri).is_err());
    }
}
