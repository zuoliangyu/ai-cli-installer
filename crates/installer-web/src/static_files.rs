use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::Embed;

/// Serve static files from the embedded dist/ directory.
/// Falls back to index.html for SPA routing.
#[derive(Embed)]
#[folder = "../../dist"]
struct Asset;

pub async fn static_handler(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');

    let mut response = match Asset::get(path).filter(|_| !path.is_empty()) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data.into_owned(),
            )
                .into_response()
        }
        None => match Asset::get("index.html") {
            Some(content) => {
                Html(String::from_utf8_lossy(&content.data).to_string()).into_response()
            }
            None => (
                StatusCode::NOT_FOUND,
                "Frontend not found. Build with `npm run build` first.",
            )
                .into_response(),
        },
    };

    // The Host header has already been checked against the allow-list.
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    apply_security_headers(response.headers_mut(), host);
    response
}

fn apply_security_headers(headers: &mut HeaderMap, host: Option<&str>) {
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if let Ok(csp) = HeaderValue::from_str(&content_security_policy(host)) {
        headers.insert(HeaderName::from_static("content-security-policy"), csp);
    }
}

/// External same-origin scripts only (`/assets/*.js`, `/theme-init.js`); no
/// inline script. `'unsafe-inline'` styles are needed by Svelte transitions.
/// `ws:`/`wss:` to the current host are listed explicitly because older
/// WebKit doesn't treat them as `'self'`.
fn content_security_policy(host: Option<&str>) -> String {
    let ws = host
        .map(|h| format!(" ws://{h} wss://{h}"))
        .unwrap_or_default();
    format!(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; font-src 'self' data:; connect-src 'self'{ws}; \
         object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_headers_are_set() {
        let mut headers = HeaderMap::new();
        apply_security_headers(&mut headers, Some("127.0.0.1:3210"));
        assert_eq!(headers[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
        let csp = headers["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("script-src 'self';"));
        assert!(csp.contains("ws://127.0.0.1:3210"));
    }
}
