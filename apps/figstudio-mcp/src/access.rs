//! Who may reach `/mcp`: loopback by default, or a non-loopback bind guarded by a bearer
//! token and a `Host` allowlist (docs/WEB-MCP-PLAN.md L5).
use axum::{
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::{net::SocketAddr, sync::Arc};
use subtle::ConstantTimeEq;

/// Host names every server accepts.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// A refused listen address or option, never containing the token.
#[derive(Debug)]
pub struct AccessError(&'static str);
impl std::fmt::Display for AccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for AccessError {}

/// `Host` allowlist and optional bearer token for `/mcp`.
#[derive(Clone)]
pub struct HttpAccess {
    hosts: Vec<String>,
    token: Option<Arc<str>>,
}
impl std::fmt::Debug for HttpAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAccess")
            .field("hosts", &self.hosts)
            .field("token", &self.token.is_some())
            .finish()
    }
}

impl HttpAccess {
    /// Loopback hosts only, no token: what `--http` and the app do by default.
    pub fn loopback() -> Self {
        Self {
            hosts: LOOPBACK_HOSTS.map(String::from).to_vec(),
            token: None,
        }
    }

    /// Access rules for a listener bound to `addr`. A non-loopback bind gets a fresh random
    /// token and also accepts its own IP and `extra_hosts`.
    ///
    /// # Errors
    /// A wildcard address (`0.0.0.0`, `::`) without `allow_any_interface`, extra hosts on a
    /// loopback bind (no token would guard them), or no operating-system randomness.
    pub fn for_bind(
        addr: SocketAddr,
        extra_hosts: &[String],
        allow_any_interface: bool,
    ) -> Result<Self, AccessError> {
        let ip = addr.ip();
        if ip.is_loopback() {
            return if extra_hosts.is_empty() {
                Ok(Self::loopback())
            } else {
                Err(AccessError(
                    "--allow-host needs a non-loopback --http address",
                ))
            };
        }
        if ip.is_unspecified() && !allow_any_interface {
            return Err(AccessError(
                "refusing to listen on every interface; pass --allow-any-interface or bind one address",
            ));
        }
        let mut hosts = Self::loopback().hosts;
        if !ip.is_unspecified() {
            hosts.push(ip.to_string());
        }
        hosts.extend(extra_hosts.iter().filter(|h| !h.is_empty()).cloned());
        Ok(Self {
            hosts,
            token: Some(random_token()?.into()),
        })
    }

    /// The bearer token, present exactly when the bind is not loopback.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub(crate) fn hosts(&self) -> &[String] {
        &self.hosts
    }

    pub(crate) fn token_arc(&self) -> Option<Arc<str>> {
        self.token.clone()
    }
}

fn random_token() -> Result<String, AccessError> {
    crate::bridge::pairing_token().map_err(|_| AccessError("no operating-system randomness"))
}

/// 401 unless `Authorization: Bearer <token>` matches, compared in constant time.
pub(crate) async fn require_bearer(
    State(token): State<Arc<str>>,
    request: Request,
    next: Next,
) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if bool::from(given.as_bytes().ct_eq(token.as_bytes())) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    #[test]
    fn loopback_has_no_token_and_non_loopback_always_has_one() {
        assert!(
            HttpAccess::for_bind(addr("127.0.0.1:3845"), &[], false)
                .unwrap()
                .token()
                .is_none()
        );
        let remote =
            HttpAccess::for_bind(addr("100.64.0.1:3845"), &["box.ts.net".into()], false).unwrap();
        assert_eq!(remote.token().map(str::len), Some(64));
        assert!(remote.hosts().iter().any(|h| h == "100.64.0.1"));
        assert!(remote.hosts().iter().any(|h| h == "box.ts.net"));
        assert!(remote.hosts().iter().any(|h| h == "localhost"));
        let again = HttpAccess::for_bind(addr("100.64.0.1:3845"), &[], false).unwrap();
        assert_ne!(remote.token(), again.token(), "a fresh token per start");
    }

    #[test]
    fn wildcards_need_the_flag_and_loopback_refuses_extra_hosts() {
        for wildcard in ["0.0.0.0:3845", "[::]:3845"] {
            assert!(
                HttpAccess::for_bind(addr(wildcard), &[], false).is_err(),
                "{wildcard}"
            );
            let open = HttpAccess::for_bind(addr(wildcard), &[], true).unwrap();
            assert!(open.token().is_some());
            assert!(!open.hosts().iter().any(|h| h == "0.0.0.0" || h == "::"));
        }
        assert!(HttpAccess::for_bind(addr("127.0.0.1:1"), &["x.example".into()], false).is_err());
    }

    #[test]
    fn debug_never_prints_the_token() {
        let access = HttpAccess::for_bind(addr("100.64.0.1:3845"), &[], false).unwrap();
        assert!(!format!("{access:?}").contains(access.token().unwrap()));
    }
}
