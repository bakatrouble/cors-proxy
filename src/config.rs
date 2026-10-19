use std::env;

use axum::http::{HeaderMap, HeaderValue};

/// How to decide the value of `Access-Control-Allow-Origin`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowedOrigins {
    Any,
    List(Vec<String>),
}

impl AllowedOrigins {
    pub fn from_env() -> Self {
        match env::var("ALLOWED_ORIGINS") {
            Ok(raw) => Self::parse(&raw),
            Err(_) => Self::Any,
        }
    }

    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim();
        if raw.is_empty() || raw == "*" {
            return Self::Any;
        }
        let list: Vec<String> = raw
            .split(',')
            .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if list.is_empty() || list.iter().any(|s| s == "*") {
            Self::Any
        } else {
            Self::List(list)
        }
    }

    pub fn is_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    /// Returns the value to use for `Access-Control-Allow-Origin`, if any.
    /// For an explicit list, only returns a value when the request `Origin`
    /// is allow-listed.
    pub fn allow_origin_value(&self, request_headers: &HeaderMap) -> Option<HeaderValue> {
        match self {
            Self::Any => Some(HeaderValue::from_static("*")),
            Self::List(list) => {
                let origin = request_headers
                    .get(axum::http::header::ORIGIN)
                    .and_then(|v| v.to_str().ok())?;
                if list.iter().any(|o| o == origin) {
                    HeaderValue::from_str(origin).ok()
                } else {
                    None
                }
            }
        }
    }
}

/// Runtime settings, sourced from the environment.
#[derive(Clone, Debug)]
pub struct Settings {
    pub port: u16,
    pub allowed_origins: AllowedOrigins,
}

impl Settings {
    pub fn from_env() -> Self {
        let port: u16 = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);
        Self {
            port,
            allowed_origins: AllowedOrigins::from_env(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults_to_any() {
        assert_eq!(AllowedOrigins::parse("*"), AllowedOrigins::Any);
        assert_eq!(AllowedOrigins::parse(""), AllowedOrigins::Any);
        assert_eq!(AllowedOrigins::parse("  "), AllowedOrigins::Any);
        assert_eq!(
            AllowedOrigins::parse("https://a.com, *"),
            AllowedOrigins::Any
        );
    }

    #[test]
    fn parse_list_trims_and_filters() {
        assert_eq!(
            AllowedOrigins::parse("https://a.com, https://b.com "),
            AllowedOrigins::List(vec![
                "https://a.com".to_string(),
                "https://b.com".to_string(),
            ])
        );
    }

    fn headers_with_origin(origin: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("origin", HeaderValue::from_str(origin).unwrap());
        h
    }

    #[test]
    fn any_allows_everything() {
        let h = headers_with_origin("https://evil.com");
        assert_eq!(
            AllowedOrigins::Any.allow_origin_value(&h),
            Some(HeaderValue::from_static("*"))
        );
    }

    #[test]
    fn list_echoes_only_allowlisted() {
        let cfg = AllowedOrigins::List(vec!["https://a.com".to_string()]);
        assert!(cfg
            .allow_origin_value(&headers_with_origin("https://a.com"))
            .is_some());
        assert!(cfg
            .allow_origin_value(&headers_with_origin("https://evil.com"))
            .is_none());
        assert!(cfg.allow_origin_value(&HeaderMap::new()).is_none());
    }
}
