//! Embedded web assets (built by Vite into web/dist) and the security
//! headers applied to every response. No third-party scripts, fonts or
//! analytics are ever served or referenced.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct WebAssets;

const MISSING_ASSETS_PAGE: &str = "<!doctype html><meta charset=\"utf-8\">\
<title>LocalPad</title>\
<body style=\"font-family: system-ui; background:#101312; color:#e8ece9; \
display:grid; place-items:center; height:100vh; margin:0\">\
<div style=\"max-width:34rem; padding:2rem\">\
<h1>Web assets not built</h1>\
<p>This binary was compiled without the web interface. Build it with:</p>\
<pre style=\"background:#1a1e1c; padding:1rem; border-radius:8px\">npm --prefix web install\nnpm --prefix web run build\ncargo build --release</pre>\
</div></body>";

pub fn security_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; font-src 'self'; connect-src 'self' ws: wss:; \
             object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    response
}

fn serve_bytes(path: &str, bytes: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cacheable = path.starts_with("assets/");
    let mut response = (
        [(header::CONTENT_TYPE, mime.as_ref().to_string())],
        bytes,
    )
        .into_response();
    if cacheable {
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    security_headers(response)
}

/// Serve a static file from the embedded bundle.
pub fn asset(path: &str) -> Response {
    match WebAssets::get(path) {
        Some(file) => serve_bytes(path, file.data.into_owned()),
        None => security_headers((StatusCode::NOT_FOUND, "not found").into_response()),
    }
}

/// Serve the SPA entry point (client-side routing takes it from there).
pub fn index() -> Response {
    match WebAssets::get("index.html") {
        Some(file) => serve_bytes("index.html", file.data.into_owned()),
        None => security_headers(
            (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                MISSING_ASSETS_PAGE,
            )
                .into_response(),
        ),
    }
}
