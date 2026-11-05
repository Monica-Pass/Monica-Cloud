//! Serves the WebUI build (`WEB_VAULT_FOLDER`) as a single-page app, like
//! vaultwarden serves the Bitwarden web vault.

use axum::{
    Json, Router,
    extract::Request,
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use super::AppState;
use crate::{CONFIG, util::fmt_date};

const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
                   img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; object-src 'none'; \
                   frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

pub fn routes() -> Router<AppState> {
    let router = Router::new().route("/alive", get(alive));
    let folder = &CONFIG.web_vault_folder;
    let index = folder.join("index.html");
    if CONFIG.web_vault_enabled && index.exists() {
        // `npm run build` writes .br/.gz next to each text asset; serve those when the
        // browser accepts them instead of compressing on every request.
        //
        // Hashed build output gets no SPA fallback: a chunk that no longer exists after an
        // upgrade must be a 404, not index.html cached as JavaScript for a year.
        let assets = ServeDir::new(folder.join("assets")).precompressed_br().precompressed_gzip();
        // Unknown paths fall back to index.html so client-side routes work on reload.
        let spa = ServeDir::new(folder)
            .precompressed_br()
            .precompressed_gzip()
            .fallback(ServeFile::new(index).precompressed_br().precompressed_gzip());
        router.nest_service("/assets", assets).fallback_service(spa).layer(middleware::from_fn(web_headers))
    } else {
        router.fallback(missing_web_vault)
    }
}

async fn alive() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "time": fmt_date(&crate::util::now()) }))
}

async fn missing_web_vault() -> impl IntoResponse {
    let folder = CONFIG.web_vault_folder.to_string_lossy().into_owned();
    Html(format!(
        "<!doctype html><meta charset=utf-8><title>Monica Cloud</title>\
         <p>Monica Cloud API is running. The WebUI was not found in <code>{}</code>; \
         build it with <code>npm run build</code> in <code>webui/</code> and point <code>WEB_VAULT_FOLDER</code> at <code>webui/dist</code>.</p>",
        html_escape(&folder)
    ))
}

fn html_escape(raw: &str) -> String {
    raw.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Hashed build assets are immutable; everything else (index.html) must revalidate.
async fn web_headers(request: Request, next: Next) -> Response {
    let asset = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let immutable = asset && response.status().is_success();
    let headers = response.headers_mut();
    let cache = if immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    // The same URL is served compressed or not depending on Accept-Encoding.
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    response
}
