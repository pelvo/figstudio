//! Serves the `--origin` web app from the browser proxy's own loopback port.
//!
//! Chrome's Local Network Access check makes a public page (such as
//! `https://figstudio.pelvo.dev`) ask for permission before it opens
//! `ws://127.0.0.1:<port>/bridge`. A headless browser cannot show that prompt, so the
//! socket fails with `ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`. A page loaded from
//! `http://127.0.0.1:<port>` is already on loopback and needs no permission to reach its
//! own port, so the proxy fetches the app's static files from the origin and serves them
//! here, with the cross-origin isolation headers the Wasm threads need.

use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header, uri::Authority},
    response::{IntoResponse, Response},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, PoisonError},
    time::Duration,
};

/// Largest file fetched from the origin (the Wasm module is the biggest, well under this).
const MAX_ASSET_BYTES: u64 = 128 * 1024 * 1024;
/// Total bytes kept in memory; files past it are still served, just fetched again next time.
const MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// Host names a mirrored page may be requested under. Any other `Host` (a DNS-rebinding
/// page, for example) is refused.
const LOOPBACK_HOSTS: [&str; 3] = ["127.0.0.1", "localhost", "[::1]"];

/// Whether `origin` is already a loopback page, which reaches `ws://127.0.0.1` without a
/// Local Network Access prompt and so needs no mirror.
#[must_use]
pub fn is_loopback_origin(origin: &str) -> bool {
    url::Url::parse(origin).is_ok_and(|url| match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    })
}

/// The `Origin` headers a mirrored page on `port` sends.
#[must_use]
pub fn mirror_origins(port: u16) -> [String; 3] {
    LOOPBACK_HOSTS.map(|host| format!("http://{host}:{port}"))
}

struct Cached {
    content_type: Option<HeaderValue>,
    body: axum::body::Bytes,
}

/// An in-memory, read-through copy of one web app origin.
pub struct WebMirror {
    origin: url::Url,
    client: OnceLock<Option<reqwest::Client>>,
    cache: Mutex<(HashMap<String, Arc<Cached>>, usize)>,
}

impl std::fmt::Debug for WebMirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebMirror")
            .field("origin", &self.origin.as_str())
            .finish_non_exhaustive()
    }
}

impl WebMirror {
    /// A mirror of `origin` (`scheme://host[:port]`), or `None` when it is not one.
    #[must_use]
    pub fn new(origin: &str) -> Option<Self> {
        let origin = url::Url::parse(origin).ok()?;
        if !matches!(origin.scheme(), "http" | "https") || origin.host().is_none() {
            return None;
        }
        Some(Self {
            origin,
            client: OnceLock::new(),
            cache: Mutex::new((HashMap::new(), 0)),
        })
    }

    fn client(&self) -> Option<&reqwest::Client> {
        self.client
            .get_or_init(|| {
                let host = self.origin.host_str().map(str::to_owned);
                let port = self.origin.port_or_known_default();
                reqwest::Client::builder()
                    .timeout(FETCH_TIMEOUT)
                    // Follow redirects only within the origin, so a path can never make the
                    // proxy fetch from another host.
                    .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                        let url = attempt.url();
                        if attempt.previous().len() < 5
                            && url.host_str() == host.as_deref()
                            && url.port_or_known_default() == port
                        {
                            attempt.follow()
                        } else {
                            attempt.stop()
                        }
                    }))
                    .build()
                    .inspect_err(|e| tracing::warn!("web mirror client: {e}"))
                    .ok()
            })
            .as_ref()
    }

    /// The origin URL for a request path, or `None` when it would leave the origin.
    fn upstream_url(&self, uri: &Uri) -> Option<url::Url> {
        let path = uri.path_and_query().map_or("/", |pq| pq.as_str());
        if !path.starts_with('/') {
            return None;
        }
        let base = self.origin.as_str().trim_end_matches('/');
        let url = url::Url::parse(&format!("{base}{path}")).ok()?;
        (url.scheme() == self.origin.scheme()
            && url.host() == self.origin.host()
            && url.port_or_known_default() == self.origin.port_or_known_default())
        .then_some(url)
    }

    /// Answer one request to the proxy's port that no other route took.
    pub async fn serve(&self, method: &Method, uri: &Uri, headers: &HeaderMap) -> Response {
        if !host_is_loopback(headers) {
            return (StatusCode::FORBIDDEN, "Not a loopback host").into_response();
        }
        if method != Method::GET && method != Method::HEAD {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        // The app asks `GET /mcp/` to find a cloud relay. There is none behind this port,
        // so answer like a static-only deploy instead of asking the origin.
        let path = uri.path();
        if path == "/mcp" || path.starts_with("/mcp/") || path.starts_with("/bridge") {
            return StatusCode::NOT_FOUND.into_response();
        }
        let Some(url) = self.upstream_url(uri) else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let key = url.as_str().to_owned();
        let hit = self
            .cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .0
            .get(&key)
            .cloned();
        let cached = match hit {
            Some(hit) => hit,
            None => match self.fetch(url).await {
                Ok(fetched) => fetched,
                Err(response) => return *response,
            },
        };
        let mut response = if method == Method::HEAD {
            Response::new(Body::empty())
        } else {
            Response::new(Body::from(cached.body.clone()))
        };
        let out = response.headers_mut();
        if let Some(content_type) = &cached.content_type {
            out.insert(header::CONTENT_TYPE, content_type.clone());
        }
        isolation_headers(out);
        response
    }

    async fn fetch(&self, url: url::Url) -> Result<Arc<Cached>, Box<Response>> {
        let Some(client) = self.client() else {
            return Err(Box::new(
                (StatusCode::BAD_GATEWAY, "Web mirror unavailable").into_response(),
            ));
        };
        let key = url.as_str().to_owned();
        let upstream = client.get(url).send().await.map_err(|e| {
            tracing::warn!("web mirror fetch failed: {e}");
            Box::new((StatusCode::BAD_GATEWAY, "Could not reach the web app").into_response())
        })?;
        let status = upstream.status();
        if upstream
            .content_length()
            .is_some_and(|len| len > MAX_ASSET_BYTES)
        {
            return Err(Box::new(
                (StatusCode::BAD_GATEWAY, "Web app file too large").into_response(),
            ));
        }
        let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
        let body = read_capped(upstream).await?;
        if !status.is_success() {
            let mut response = (status, body).into_response();
            isolation_headers(response.headers_mut());
            return Err(Box::new(response));
        }
        let cached = Arc::new(Cached { content_type, body });
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let (entries, bytes) = &mut *cache;
        if *bytes + cached.body.len() <= MAX_CACHE_BYTES {
            *bytes += cached.body.len();
            entries.insert(key, cached.clone());
        }
        Ok(cached)
    }
}

async fn read_capped(mut upstream: reqwest::Response) -> Result<axum::body::Bytes, Box<Response>> {
    let mut body = Vec::new();
    loop {
        match upstream.chunk().await {
            Ok(Some(chunk)) => {
                if (body.len() + chunk.len()) as u64 > MAX_ASSET_BYTES {
                    return Err(Box::new(
                        (StatusCode::BAD_GATEWAY, "Web app file too large").into_response(),
                    ));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(body.into()),
            Err(e) => {
                tracing::warn!("web mirror read failed: {e}");
                return Err(Box::new(
                    (StatusCode::BAD_GATEWAY, "Could not read the web app").into_response(),
                ));
            }
        }
    }
}

/// The headers `web/_headers` sets on the deployed app: Wasm threads need a cross-origin
/// isolated page (`SharedArrayBuffer`).
fn isolation_headers(out: &mut HeaderMap) {
    out.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    out.insert(
        "cross-origin-embedder-policy",
        HeaderValue::from_static("require-corp"),
    );
    out.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    out.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    out.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
}

fn host_is_loopback(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<Authority>().ok())
        .is_some_and(|authority| {
            LOOPBACK_HOSTS
                .iter()
                .any(|host| authority.host().eq_ignore_ascii_case(host))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_origins_skip_the_mirror() {
        for origin in [
            "http://127.0.0.1:8765",
            "http://localhost:8765",
            "http://[::1]:8765",
        ] {
            assert!(is_loopback_origin(origin), "{origin}");
        }
        for origin in [
            "https://figstudio.pelvo.dev",
            "http://192.168.1.2:8765",
            "not a url",
        ] {
            assert!(!is_loopback_origin(origin), "{origin}");
        }
    }

    #[test]
    fn a_request_path_never_leaves_the_origin() {
        let mirror = WebMirror::new("https://figstudio.example.com").unwrap();
        let url = |path: &str| mirror.upstream_url(&path.parse::<Uri>().unwrap());
        assert_eq!(
            url("/figstudio_bg.wasm?v=1").unwrap().as_str(),
            "https://figstudio.example.com/figstudio_bg.wasm?v=1"
        );
        for path in ["//evil.example/x", "/@evil.example/x", "/..//evil.example"] {
            if let Some(found) = url(path) {
                assert_eq!(found.host_str(), Some("figstudio.example.com"), "{path}");
            }
        }
    }

    #[test]
    fn only_loopback_host_headers_are_served() {
        let with_host = |host: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
            host_is_loopback(&headers)
        };
        assert!(with_host("127.0.0.1:3845"));
        assert!(with_host("localhost:3845"));
        assert!(with_host("[::1]:3845"));
        assert!(!with_host("evil.example:3845"));
        assert!(!with_host("127.0.0.1.evil.example:3845"));
        assert!(!host_is_loopback(&HeaderMap::new()));
    }
}
