//! Request guards for the Web shell: access-token auth, `Host` allow-list
//! (DNS-rebinding defense) and `Origin` checks for state-changing requests.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::ServerState;

/// 32 random bytes, hex-encoded.
pub(crate) fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("系统随机数源不可用");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time comparison so the token can't be guessed byte by byte.
pub(crate) fn token_matches(expected: &str, provided: Option<&str>) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), provided.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) async fn require_auth(
    State(state): State<ServerState>,
    request: Request,
    next: Next,
) -> Response {
    let provided = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if token_matches(&state.access_token, provided) {
        next.run(request).await
    } else {
        unauthorized()
    }
}

pub(crate) fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "缺少或无效的访问令牌").into_response()
}

/// Lower-cased `Host` values the server answers to (see
/// `Config::allowed_hosts`).
#[derive(Debug)]
pub(crate) struct HostPolicy {
    allowed: Vec<String>,
}

impl HostPolicy {
    pub fn new(allowed: Vec<String>) -> Self {
        Self {
            allowed: allowed
                .into_iter()
                .map(|h| h.to_ascii_lowercase())
                .collect(),
        }
    }

    pub fn host_allowed(&self, host: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        self.allowed.contains(&host)
    }

    /// `Origin` is `scheme://host[:port]` with no path; the authority must be
    /// one of the allowed hosts. `null` and non-http(s) origins are rejected.
    pub fn origin_allowed(&self, origin: &str) -> bool {
        let origin = origin.trim();
        let authority = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"));
        match authority {
            Some(authority) if !authority.is_empty() && !authority.contains('/') => {
                self.host_allowed(authority)
            }
            _ => false,
        }
    }

    pub fn check(
        &self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        uri_host: Option<&str>,
    ) -> Result<(), &'static str> {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .or(uri_host);
        if !host.is_some_and(|h| self.host_allowed(h)) {
            return Err("Host 不被允许；如需通过其他地址访问，请使用 --allowed-host 添加");
        }

        let needs_origin_check = !matches!(*method, Method::GET | Method::HEAD)
            || path == "/ws/progress"
            || headers.contains_key(header::UPGRADE);
        if needs_origin_check {
            if let Some(origin) = headers.get(header::ORIGIN) {
                let ok = origin.to_str().is_ok_and(|o| self.origin_allowed(o));
                if !ok {
                    return Err("跨源请求被拒绝");
                }
            }
        }
        Ok(())
    }
}

pub(crate) async fn guard_host_and_origin(
    State(policy): State<Arc<HostPolicy>>,
    request: Request,
    next: Next,
) -> Response {
    let uri_host = request.uri().authority().map(|a| a.as_str().to_owned());
    match policy.check(
        request.method(),
        request.uri().path(),
        request.headers(),
        uri_host.as_deref(),
    ) {
        Ok(()) => next.run(request).await,
        Err(message) => (StatusCode::FORBIDDEN, message).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn policy() -> HostPolicy {
        HostPolicy::new(vec![
            "127.0.0.1:3210".into(),
            "localhost:3210".into(),
            "[::1]:3210".into(),
            "Example.com".into(),
        ])
    }

    fn headers(pairs: &[(header::HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn token_requires_exact_match() {
        assert!(token_matches("secret", Some("secret")));
        assert!(!token_matches("secret", None));
        assert!(!token_matches("secret", Some("wrong!")));
        assert!(!token_matches("secret", Some("secret2")));
        assert!(!token_matches("secret", Some("")));
    }

    #[test]
    fn generated_tokens_are_random_hex() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn host_header_must_be_allow_listed() {
        let p = policy();
        assert!(p.host_allowed("127.0.0.1:3210"));
        assert!(p.host_allowed("LOCALHOST:3210"));
        assert!(p.host_allowed("[::1]:3210"));
        assert!(p.host_allowed("example.com"));
        assert!(!p.host_allowed("127.0.0.1:9999"));
        assert!(!p.host_allowed("evil.com:3210"));
        assert!(!p.host_allowed("localhost"));
    }

    #[test]
    fn origin_must_match_allowed_host() {
        let p = policy();
        assert!(p.origin_allowed("http://127.0.0.1:3210"));
        assert!(p.origin_allowed("http://localhost:3210"));
        assert!(p.origin_allowed("https://example.com"));
        assert!(!p.origin_allowed("http://evil.com:3210"));
        assert!(!p.origin_allowed("null"));
        assert!(!p.origin_allowed("file://"));
        assert!(!p.origin_allowed("http://127.0.0.1:3210/path"));
    }

    #[test]
    fn check_combines_host_and_origin_rules() {
        let p = policy();
        let ok_host = (header::HOST, "127.0.0.1:3210");

        assert!(p
            .check(
                &Method::GET,
                "/api/tools",
                &headers(std::slice::from_ref(&ok_host)),
                None
            )
            .is_ok());
        assert!(p
            .check(
                &Method::GET,
                "/",
                &headers(&[(header::HOST, "rebind.evil.com:3210")]),
                None
            )
            .is_err());
        assert!(p.check(&Method::GET, "/", &HeaderMap::new(), None).is_err());
        assert!(p
            .check(&Method::GET, "/", &HeaderMap::new(), Some("localhost:3210"))
            .is_ok());

        // Cross-origin GET is allowed (read-only, still token-protected)...
        let cross = (header::ORIGIN, "http://evil.com");
        assert!(p
            .check(
                &Method::GET,
                "/api/tools",
                &headers(&[ok_host.clone(), cross.clone()]),
                None
            )
            .is_ok());
        // ...but not POST or the WebSocket upgrade.
        assert!(p
            .check(
                &Method::POST,
                "/api/tools/install",
                &headers(&[ok_host.clone(), cross.clone()]),
                None
            )
            .is_err());
        assert!(p
            .check(
                &Method::GET,
                "/ws/progress",
                &headers(&[ok_host.clone(), cross]),
                None
            )
            .is_err());

        let same = (header::ORIGIN, "http://127.0.0.1:3210");
        assert!(p
            .check(
                &Method::POST,
                "/api/tools/install",
                &headers(&[ok_host.clone(), same]),
                None
            )
            .is_ok());
        // Non-browser clients send no Origin.
        assert!(p
            .check(&Method::POST, "/api/path/add", &headers(&[ok_host]), None)
            .is_ok());
    }
}
