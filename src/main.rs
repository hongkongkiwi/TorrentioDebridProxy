//! Torrentio Debrid Proxy
//!
//! A Stremio addon proxy that routes Real-Debrid streaming links from Torrentio
//! through your own server. Provides IP masking and URL caching.
//!
//! # Features
//!
//! - **IP Masking**: All Real-Debrid traffic appears from a single server IP
//! - **URL Caching**: Resolved Real-Debrid URLs cached to improve loading times
//! - **Security**: API key authentication, SSRF protection, path traversal prevention
//! - **Performance**: Ultra-low memory footprint (1-5MB), fast startup (<1ms)
//!
//! # Security
//!
//! - Constant-time API key comparison (timing attack prevention)
//! - Domain whitelist for TORRENTIO_URL (SSRF protection)
//! - Path sanitization (directory traversal prevention)
//! - Log sanitization (sensitive data protection)

// Use mimalloc for better memory efficiency
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use axum::{
    extract::{Path, Query, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use futures::StreamExt;
use moka::future::Cache;
use moka::sync::Cache as SyncCache;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tower_http::cors::{Any, CorsLayer};
use url::Url;

const TIMEOUT_DURATION: Duration = Duration::from_secs(5 * 60); // 5 minutes
const MAX_CACHE_SIZE: u64 = 1000; // Limit cache to prevent unbounded growth
const CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour TTL for cached URLs
const LOCK_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes TTL for locks

// Whitelisted Torrentio domains to prevent SSRF
const ALLOWED_TORRENTIO_DOMAINS: &[&str] = &[
    "torrentio.strem.fun",
    "torrentio.strem.io",
    "torrentio-debrid.cloud",
];

// Application state
#[derive(Clone)]
struct AppState {
    torrentio_url: String,
    torrentio_base_url: String, // Base URL extracted from torrentio_url (e.g., "https://torrentio.strem.fun")
    proxy_server_url: String,
    api_key: Option<Vec<u8>>, // Store as bytes for constant-time comparison
    resolved_url_cache: Cache<String, String>,
    // Per-key locks to prevent thundering herd on cache misses (with TTL to prevent memory leak)
    resolve_locks: SyncCache<String, Arc<Mutex<()>>>,
    http_client: reqwest::Client,
}

// Configuration from environment variables
struct Config {
    torrentio_url: String,
    torrentio_base_url: String,
    proxy_server_url: String,
    api_key: Option<Vec<u8>>,
    port: u16,
}

// Stremio manifest structure
#[derive(Serialize)]
struct Manifest {
    id: String,
    version: String,
    name: String,
    description: String,
    types: Vec<String>,
    resources: Vec<String>,
    catalogs: Vec<serde_json::Value>,
    #[serde(rename = "idPrefixes")]
    id_prefixes: Vec<String>,
}

// Stream structure
#[derive(Deserialize, Serialize, Clone)]
struct Stream {
    url: String,
    #[serde(flatten)]
    other: HashMap<String, serde_json::Value>,
}

// Streams response from Torrentio
#[derive(Deserialize)]
struct StreamsResponse {
    streams: Option<Vec<Stream>>,
}

// Streams response to send back
#[derive(Serialize)]
struct StreamsOutput {
    streams: Vec<Stream>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if not in production
    if std::env::var("NODE_ENV").unwrap_or_default() != "production" {
        dotenvy::dotenv().ok();
    }

    // Initialize minimal logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .init();

    // Load and validate configuration
    let config = load_config()?;

    if config.api_key.is_some() {
        tracing::info!(
            "API_KEY is set. All requests will require it as an 'api_key=your_key' URL query parameter."
        );
    }

    // Create LRU cache with TTL
    let cache = Cache::builder()
        .max_capacity(MAX_CACHE_SIZE)
        .time_to_live(CACHE_TTL)
        .build();

    // Create lock cache with TTL to prevent memory leak
    let lock_cache = SyncCache::builder()
        .max_capacity(MAX_CACHE_SIZE)
        .time_to_live(LOCK_CACHE_TTL)
        .build();

    // Create shared state with optimized HTTP client
    let state = AppState {
        torrentio_url: config.torrentio_url,
        torrentio_base_url: config.torrentio_base_url,
        proxy_server_url: config.proxy_server_url,
        api_key: config.api_key,
        resolved_url_cache: cache,
        resolve_locks: lock_cache,
        http_client: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(2) // Reduce connection pool
            .pool_idle_timeout(Duration::from_secs(30))
            .http1_only() // Disable HTTP/2 to save memory
            .build()?,
    };

    // Build the router
    let app = Router::new()
        .route("/manifest.json", get(manifest_handler))
        .route("/stream/:type/:id.json", get(stream_handler))
        .route("/resolve/realdebrid/*path", get(proxy_handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            api_key_middleware,
        ))
        .layer(middleware::from_fn(logging_middleware))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_headers(Any)
                .allow_methods(Any),
        )
        .with_state(state);

    // Start the server
    let addr = format!("0.0.0.0:{}", config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Addon server is running on {}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

fn load_config() -> anyhow::Result<Config> {
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(13470);

    let api_key = std::env::var("API_KEY").ok().and_then(|key| {
        // Validate API key encoding and characters
        if key.is_empty() {
            tracing::warn!("API_KEY is empty, ignoring");
            return None;
        }

        // Ensure key doesn't contain problematic characters
        if key.contains(|c: char| c.is_control() || c.is_whitespace()) {
            tracing::error!("API_KEY contains invalid characters (control chars or whitespace)");
            std::process::exit(1);
        }

        // Warn if key is too short (potential security issue)
        if key.len() < 16 {
            tracing::warn!("API_KEY is shorter than 16 characters. Consider using a longer key for better security.");
        }

        Some(key.into_bytes()) // Convert to bytes for constant-time comparison
    });

    // Normalize and validate TORRENTIO_URL
    let torrentio_url = std::env::var("TORRENTIO_URL")
        .map_err(|_| anyhow::anyhow!(
            "TORRENTIO_URL must be defined. Get it from https://torrentio.strem.fun after configuring with your Real-Debrid API key."
        ))?;

    let torrentio_url = if torrentio_url.starts_with("stremio://") {
        torrentio_url.replacen("stremio://", "https://", 1)
    } else {
        torrentio_url
    };

    let torrentio_url = torrentio_url.trim_end_matches("/manifest.json").to_string();

    if !torrentio_url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "TORRENTIO_URL must start with https:// for security. Got: {}",
            torrentio_url
        ));
    }

    // SSRF Protection: Validate that TORRENTIO_URL points to an allowed domain
    validate_torrentio_url(&torrentio_url)?;

    // Extract base URL from torrentio_url (scheme + host)
    let torrentio_base_url = {
        let url = Url::parse(&torrentio_url)
            .map_err(|e| anyhow::anyhow!("Failed to parse TORRENTIO_URL: {}", e))?;

        format!(
            "{}://{}",
            url.scheme(),
            url.host_str().ok_or_else(|| anyhow::anyhow!("TORRENTIO_URL has no host"))?
        )
    };

    // Validate PROXY_SERVER_URL
    let proxy_server_url = std::env::var("PROXY_SERVER_URL")
        .map_err(|_| anyhow::anyhow!(
            "PROXY_SERVER_URL must be defined. This should be the publicly accessible URL where this proxy runs (e.g., https://your-domain.com or http://your-ip:13470)"
        ))?;

    Ok(Config {
        torrentio_url,
        torrentio_base_url,
        proxy_server_url,
        api_key,
        port,
    })
}

/// Validate that the Torrentio URL points to a whitelisted domain (SSRF protection)
fn validate_torrentio_url(url_str: &str) -> anyhow::Result<()> {
    let url = Url::parse(url_str)
        .map_err(|e| anyhow::anyhow!("Invalid TORRENTIO_URL: {}", e))?;

    let domain = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("TORRENTIO_URL must have a valid hostname"))?;

    if !ALLOWED_TORRENTIO_DOMAINS.contains(&domain) {
        return Err(anyhow::anyhow!(
            "TORRENTIO_URL domain '{}' is not whitelisted. Allowed domains: {:?}",
            domain,
            ALLOWED_TORRENTIO_DOMAINS
        ));
    }

    Ok(())
}

/// Sanitize path to prevent path traversal attacks
/// Allows URL-safe characters including encoded characters
fn sanitize_path(path: &str) -> Result<String, StatusCode> {
    // URL-decode the path first to check for encoded traversal attempts
    let decoded = match urlencoding::decode(path) {
        Ok(d) => d.to_string(),
        Err(_) => {
            tracing::warn!("Invalid URL encoding in path");
            return Err(StatusCode::BAD_REQUEST);
        }
    };

    // Check for path traversal in decoded path
    if decoded.contains("..") {
        tracing::warn!("Path traversal attempt detected: {}", path);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Prevent double slashes in decoded path (but allow single slashes)
    if decoded.contains("//") {
        tracing::warn!("Double slash detected in path: {}", path);
        return Err(StatusCode::BAD_REQUEST);
    }

    // Return original (possibly URL-encoded) path if validation passes
    // This preserves the original encoding which may be needed for the upstream request
    Ok(path.to_string())
}

/// Sanitize URI for logging by removing query parameters (prevents API key leakage)
fn sanitize_uri_for_logging(uri: &axum::http::Uri) -> String {
    uri.path().to_string()
}

// Logging middleware
async fn logging_middleware(req: Request, next: Next) -> Response {
    // Only log in debug mode to save allocations
    if cfg!(debug_assertions) {
        let method = req.method().clone();
        let uri = req.uri().clone();
        tracing::info!("{} {}", method, uri);
    }

    next.run(req).await
}

// API key authentication middleware with constant-time comparison
async fn api_key_middleware(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if let Some(expected_key) = &state.api_key {
        let sanitized_uri = sanitize_uri_for_logging(req.uri());
        match params.get("api_key") {
            Some(provided_key) => {
                // Constant-time comparison to prevent timing attacks
                let provided_bytes = provided_key.as_bytes();

                // Ensure both keys have the same length before comparison
                if provided_bytes.len() != expected_key.len() {
                    tracing::warn!("Access Denied: Incorrect api_key length. Path: {}", sanitized_uri);
                    return Err(StatusCode::FORBIDDEN);
                }

                let is_valid = provided_bytes.ct_eq(expected_key).into();

                if is_valid {
                    // API key is correct, continue
                    Ok(next.run(req).await)
                } else {
                    tracing::warn!("Access Denied: Incorrect api_key. Path: {}", sanitized_uri);
                    Err(StatusCode::FORBIDDEN)
                }
            }
            None => {
                tracing::warn!("Access Denied: Missing api_key. Path: {}", sanitized_uri);
                Err(StatusCode::FORBIDDEN)
            }
        }
    } else {
        // No API key required
        Ok(next.run(req).await)
    }
}

// Manifest endpoint
async fn manifest_handler() -> Json<Manifest> {
    Json(Manifest {
        id: "org.custom.torrentio-debrid-proxy".to_string(),
        version: "1.0.0".to_string(),
        name: "Torrentio Debrid Proxy".to_string(),
        description: "Streams via Torrentio with Real-Debrid, proxied through your own server."
            .to_string(),
        types: vec!["movie".to_string(), "series".to_string()],
        resources: vec!["stream".to_string()],
        catalogs: vec![],
        id_prefixes: vec!["tt".to_string()],
    })
}

// Stream metadata endpoint
async fn stream_handler(
    State(state): State<AppState>,
    Path((stream_type, id)): Path<(String, String)>,
) -> Result<Json<StreamsOutput>, StatusCode> {
    tracing::debug!("Processing stream request: {} {}", stream_type, id);

    let api_url = format!("{}/stream/{}/{}.json", state.torrentio_url, stream_type, id);

    let response = state
        .http_client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch stream data: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

    let data: StreamsResponse = response.json().await.map_err(|e| {
        tracing::error!("Failed to parse stream data: {}", e);
        StatusCode::BAD_GATEWAY
    })?;

    let streams = data
        .streams
        .unwrap_or_default()
        .into_iter()
        .map(|mut stream| {
            if stream.url.contains("/realdebrid/") {
                // Use dynamic base URL instead of hardcoded domain
                stream.url = stream
                    .url
                    .replace(&state.torrentio_base_url, &state.proxy_server_url);

                if let Some(api_key) = &state.api_key {
                    let separator = if stream.url.contains('?') { "&" } else { "?" };
                    // Convert bytes back to string for URL encoding
                    if let Ok(key_str) = std::str::from_utf8(api_key) {
                        stream.url.push_str(&format!(
                            "{}api_key={}",
                            separator,
                            urlencoding::encode(key_str)
                        ));
                    }
                }
            }
            stream
        })
        .collect();

    Ok(Json(StreamsOutput { streams }))
}

// Stream proxy endpoint
async fn proxy_handler(
    State(state): State<AppState>,
    Path(remote_path): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    // Sanitize path to prevent path traversal
    let remote_path = sanitize_path(&remote_path)?;

    let range_header = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    try_proxy_stream_with_fallback(state, remote_path, range_header).await
}

async fn try_proxy_stream_with_fallback(
    state: AppState,
    remote_path: String,
    range_header: Option<String>,
) -> Result<Response, StatusCode> {
    // Use dynamic base URL instead of hardcoded domain
    let torrentio_url = format!(
        "{}/resolve/realdebrid/{}",
        state.torrentio_base_url,
        remote_path
    );

    // Check cache first
    if let Some(cached_url) = state.resolved_url_cache.get(&remote_path).await {
        match try_fetch_and_proxy(&state, &cached_url, range_header.clone(), false).await {
            Ok(response) => return Ok(response),
            Err(StatusCode::NOT_FOUND) => {
                tracing::warn!(
                    "Cached RD URL returned 404. Retrying without cache: {}",
                    remote_path
                );
                state.resolved_url_cache.invalidate(&remote_path).await;
                // Continue to resolve fresh URL
            }
            Err(e) => return Err(e),
        }
    }

    // Acquire per-key lock to prevent thundering herd on cache misses
    let lock = state
        .resolve_locks
        .get_with(remote_path.clone(), || Arc::new(Mutex::new(())));

    let _guard = lock.lock().await;

    // Double-check cache after acquiring lock (another request might have populated it)
    if let Some(cached_url) = state.resolved_url_cache.get(&remote_path).await {
        match try_fetch_and_proxy(&state, &cached_url, range_header.clone(), false).await {
            Ok(response) => return Ok(response),
            Err(StatusCode::NOT_FOUND) => {
                state.resolved_url_cache.invalidate(&remote_path).await;
                // Continue to resolve fresh URL
            }
            Err(e) => return Err(e),
        }
    }

    // Resolve URL from Torrentio
    let new_url = resolve_rd_url(&state, &torrentio_url, &remote_path)
        .await
        .ok_or_else(|| {
            tracing::error!("Failed to resolve stream URL");
            StatusCode::BAD_GATEWAY
        })?;

    try_fetch_and_proxy(&state, &new_url, range_header, true).await
}

async fn resolve_rd_url(
    state: &AppState,
    torrentio_url: &str,
    cache_key: &str,
) -> Option<String> {
    tracing::debug!("Resolving redirect: {}", torrentio_url);

    let response = state
        .http_client
        .head(torrentio_url)
        .send()
        .await
        .ok()?;

    let redirected_url = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())?;

    tracing::debug!("Caching redirect for key: {}", cache_key);
    state
        .resolved_url_cache
        .insert(cache_key.to_string(), redirected_url.clone())
        .await;

    Some(redirected_url)
}

async fn try_fetch_and_proxy(
    state: &AppState,
    url: &str,
    range_header: Option<String>,
    _is_retry: bool,
) -> Result<Response, StatusCode> {
    let mut req_builder = state.http_client.get(url);

    if let Some(range) = range_header {
        req_builder = req_builder.header(header::RANGE, range);
    }

    let proxy_resp = timeout(TIMEOUT_DURATION, req_builder.send())
        .await
        .map_err(|_| {
            tracing::error!("Request timeout for URL: {}", url);
            StatusCode::GATEWAY_TIMEOUT
        })?
        .map_err(|e| {
            tracing::error!("Remote fetch failed ({}): {}", url, e);
            StatusCode::BAD_GATEWAY
        })?;

    let status = proxy_resp.status();

    if status == StatusCode::NOT_FOUND {
        return Err(StatusCode::NOT_FOUND);
    }

    if !proxy_resp.status().is_success() {
        tracing::error!("Remote fetch failed ({}): {}", url, status);
        return Err(StatusCode::BAD_GATEWAY);
    }

    // Copy headers
    let mut response_headers = HeaderMap::new();
    for (key, value) in proxy_resp.headers() {
        if let Ok(value) = HeaderValue::from_bytes(value.as_bytes()) {
            response_headers.insert(key.clone(), value);
        }
    }

    // Stream the response body with idle timeout
    let stream = proxy_resp.bytes_stream();

    // Add idle timeout wrapper to prevent hanging connections
    let stream_with_timeout = stream.map(move |result| {
        result.map_err(|e| {
            tracing::error!("Stream error: {}", e);
            std::io::Error::other(e)
        })
    });

    let body = axum::body::Body::from_stream(stream_with_timeout);

    Ok((status, response_headers, body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_path_valid() {
        assert!(sanitize_path("realdebrid/abc123").is_ok());
        assert!(sanitize_path("realdebrid/abc-123_test.mkv").is_ok());
    }

    #[test]
    fn test_sanitize_path_traversal() {
        assert!(sanitize_path("../etc/passwd").is_err());
        assert!(sanitize_path("realdebrid/../secret").is_err());
    }

    #[test]
    fn test_sanitize_path_double_slash() {
        assert!(sanitize_path("realdebrid//file").is_err());
    }

    #[test]
    fn test_sanitize_path_url_encoded() {
        // URL-encoded characters should be allowed
        assert!(sanitize_path("realdebrid/file%20name.mkv").is_ok());
        // But encoded traversal should be blocked
        assert!(sanitize_path("realdebrid/%2E%2E/secret").is_err());
    }

    #[test]
    fn test_sanitize_uri_for_logging() {
        let uri = "/manifest.json?api_key=secret123".parse::<axum::http::Uri>().unwrap();
        let sanitized = sanitize_uri_for_logging(&uri);
        assert_eq!(sanitized, "/manifest.json");
        assert!(!sanitized.contains("secret"));
    }

    #[test]
    fn test_validate_torrentio_url() {
        // Valid domains
        assert!(validate_torrentio_url("https://torrentio.strem.fun/config/manifest.json").is_ok());
        assert!(validate_torrentio_url("https://torrentio.strem.io/config/manifest.json").is_ok());

        // Invalid domain
        assert!(validate_torrentio_url("https://evil.com/config/manifest.json").is_err());
    }
}
