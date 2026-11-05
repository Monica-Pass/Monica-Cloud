mod admin;
mod core;
mod identity;
mod web;

use std::{sync::Arc, time::Instant};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{Extensions, HeaderMap, HeaderName, HeaderValue, StatusCode, Version, header},
};
use tower_http::{
    compression::{
        CompressionLayer, CompressionLevel,
        predicate::{Predicate, SizeAbove},
    },
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};

use crate::{CONFIG, db::Db, error::Error, ratelimit::RateLimiter, storage::Storage};

pub use self::core::purge_expired_sends;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub storage: Storage,
    pub login_limiter: Arc<RateLimiter>,
    pub started_at: Instant,
}

/// Compresses JSON responses. A vault's sync is mostly base64 ciphertext and JSON keys, which
/// halve in size even at the fastest level (1.8 MB → 0.9 MB for 3000 items, a few ms of CPU).
/// Attachment and Send downloads are already-encrypted bytes that would not shrink, so only
/// JSON qualifies; the WebUI's static files come precompressed from the build.
fn json_compression() -> CompressionLayer<impl Predicate> {
    let json = |_: StatusCode, _: Version, headers: &HeaderMap, _: &Extensions| {
        headers.get(header::CONTENT_TYPE).is_some_and(|value| value.as_bytes().starts_with(b"application/json"))
    };
    CompressionLayer::new().quality(CompressionLevel::Precise(1)).compress_when(SizeAbove::new(1024).and(json))
}

pub fn router(state: AppState) -> Router {
    // Token and account endpoints stay uncompressed; their responses are small and carry secrets.
    let vault = Router::new().merge(core::routes()).merge(admin::routes()).layer(json_compression());
    let api = Router::new()
        .merge(identity::routes())
        .merge(vault)
        .fallback(|| async { Error::NotFound("unknown API endpoint".into()) })
        // JSON bodies only; blob uploads stream through `Body` and enforce their own limits.
        .layer(DefaultBodyLimit::max(CONFIG.json_limit_mb * 1024 * 1024));

    Router::new()
        .nest("/api/v1", api)
        .merge(web::routes())
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("same-origin")))
        .layer(SetResponseHeaderLayer::if_not_present(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
